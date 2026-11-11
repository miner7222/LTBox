use super::{EdlSession, PartitionFlash, PartitionFlashError};
use qdl::types::{FirehoseConfiguration, QdlBackend, QdlDevice, QdlReadWrite};
use std::io::{self, BufRead, Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
    mpsc,
};

const ACK: &[u8] = b"<data><response value=\"ACK\" /></data>";
const NAK: &[u8] = b"<data><response value=\"NAK\" /></data>";
const LABELS: [&str; 3] = ["vendor_boot_a", "vbmeta_a", "boot_a"];
const STARTS: [u64; 3] = [64, 80, 96];
const CAPACITIES: [u64; 3] = [2, 4, 2];

#[derive(Clone, Debug, PartialEq)]
enum Event {
    Partition(String),
    Start,
    Program(u64, usize),
    Erase(u64, usize),
    Payload(Vec<u8>),
    Reset(String),
    Patch(String, String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fault {
    CommandWrite(usize),
    PartialPayload(usize),
    DisconnectedPayload(usize),
    CommandNak(usize),
    CommandTimeout(usize),
    FinalNak(usize),
    FinalTimeout(usize),
    GptDisconnect,
    GptTimeout,
}

struct AckGate {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

type Events = Arc<Mutex<Vec<Event>>>;

/// Respond to actual Firehose read/program XML, including ACK/raw-data/ACK.
struct Transport {
    disk: Vec<u8>,
    sector_size: usize,
    responses: Cursor<Vec<u8>>,
    events: Events,
    program_count: usize,
    fault: Option<Fault>,
    payload_remaining: usize,
    final_ack_gate: Option<AckGate>,
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.responses.read(buf)
    }
}

impl BufRead for Transport {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.program_count == 1
            && self.payload_remaining == 0
            && let Some(gate) = self.final_ack_gate.take()
        {
            gate.entered
                .send(())
                .map_err(|_| io::ErrorKind::BrokenPipe)?;
            gate.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|_| io::ErrorKind::TimedOut)?;
        }
        let bytes = self.responses.fill_buf()?;
        if bytes.is_empty() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        Ok(bytes)
    }

    fn consume(&mut self, amount: usize) {
        self.responses.consume(amount);
    }
}

impl Write for Transport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // The drop-time `<power>` can follow a faulted transfer that left
        // payload owed or responses unread, so it is recognised first.
        if let Some(value) = power_command(bytes) {
            self.events.lock().unwrap().push(Event::Reset(value));
            self.payload_remaining = 0;
            self.responses = Cursor::new(ACK.to_vec());
            return Ok(bytes.len());
        }
        if self.payload_remaining > 0 {
            assert!(bytes.len() <= self.payload_remaining);
            if self.fault == Some(Fault::DisconnectedPayload(self.program_count)) {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            let accepted = if self.fault == Some(Fault::PartialPayload(self.program_count)) {
                137
            } else {
                bytes.len()
            };
            self.events
                .lock()
                .unwrap()
                .push(Event::Payload(bytes[..accepted].to_vec()));
            self.payload_remaining -= accepted;
            if self.payload_remaining == 0 {
                self.responses = Cursor::new(match self.fault {
                    Some(Fault::FinalNak(n)) if n == self.program_count => NAK.to_vec(),
                    Some(Fault::FinalTimeout(n)) if n == self.program_count => Vec::new(),
                    _ => ACK.to_vec(),
                });
            }
            return Ok(accepted);
        }

        let xml = std::str::from_utf8(bytes).unwrap();
        let document = roxmltree::Document::parse(xml).unwrap();
        let command = document.root_element().first_element_child().unwrap();
        assert_eq!(
            self.responses.position() as usize,
            self.responses.get_ref().len()
        );
        match command.tag_name().name() {
            "read" => {
                assert_eq!(
                    self.program_count, 0,
                    "Unexpected readback after programming"
                );
                match self.fault {
                    Some(Fault::GptDisconnect) => return Err(io::ErrorKind::BrokenPipe.into()),
                    Some(Fault::GptTimeout) => return Ok(bytes.len()),
                    _ => {}
                }
                let start: usize = command.attribute("start_sector").unwrap().parse().unwrap();
                let sectors: usize = command
                    .attribute("num_partition_sectors")
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut response = ACK.to_vec();
                response.extend_from_slice(
                    &self.disk[start * self.sector_size..(start + sectors) * self.sector_size],
                );
                response.extend_from_slice(ACK);
                self.responses = Cursor::new(response);
            }
            "patch" => {
                self.events.lock().unwrap().push(Event::Patch(
                    command.attribute("start_sector").unwrap().to_owned(),
                    command.attribute("value").unwrap().to_owned(),
                ));
                self.responses = Cursor::new(ACK.to_vec());
            }
            "erase" => {
                let start = command.attribute("start_sector").unwrap().parse().unwrap();
                let sectors = command
                    .attribute("num_partition_sectors")
                    .unwrap()
                    .parse()
                    .unwrap();
                self.events
                    .lock()
                    .unwrap()
                    .push(Event::Erase(start, sectors));
                self.responses = Cursor::new(ACK.to_vec());
            }
            "program" => {
                let start = command.attribute("start_sector").unwrap().parse().unwrap();
                let sectors = command
                    .attribute("num_partition_sectors")
                    .unwrap()
                    .parse()
                    .unwrap();
                self.events
                    .lock()
                    .unwrap()
                    .push(Event::Program(start, sectors));
                self.program_count += 1;
                if self.fault == Some(Fault::CommandWrite(self.program_count)) {
                    return Err(io::ErrorKind::BrokenPipe.into());
                }
                self.responses = Cursor::new(match self.fault {
                    Some(Fault::CommandNak(n)) if n == self.program_count => NAK.to_vec(),
                    Some(Fault::CommandTimeout(n)) if n == self.program_count => Vec::new(),
                    _ => ACK.to_vec(),
                });
                self.payload_remaining = sectors * self.sector_size;
            }
            other => panic!("unexpected Firehose command: {other}"),
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl QdlReadWrite for Transport {}

/// The reset mode of a Firehose `<power>` command, if `bytes` is one.
fn power_command(bytes: &[u8]) -> Option<String> {
    let xml = std::str::from_utf8(bytes).ok()?;
    let document = roxmltree::Document::parse(xml).ok()?;
    let command = document.root_element().first_element_child()?;
    command
        .has_tag_name("power")
        .then(|| command.attribute("value").unwrap_or_default().to_string())
}

struct Fixture {
    dir: PathBuf,
    images: [PathBuf; 3],
    session: EdlSession,
    events: Events,
}

impl Fixture {
    fn new(sector_size: usize, omit_second: bool, fault: Option<Fault>) -> Self {
        Self::with_gate(sector_size, omit_second, fault, None)
    }

    fn with_gate(
        sector_size: usize,
        omit_second: bool,
        fault: Option<Fault>,
        final_ack_gate: Option<AckGate>,
    ) -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ltbox-partition-flash-{}-{nanos}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let images = LABELS.map(|label| dir.join(format!("{label}.img")));
        for (index, image) in images.iter().enumerate() {
            std::fs::write(image, vec![0xa1 + index as u8; sector_size + 1]).unwrap();
        }

        let mut disk = Cursor::new(vec![0; sector_size * 256]);
        let mut gpt = gptman::GPT::new_from(&mut disk, sector_size as u64, [0x11; 16]).unwrap();
        for index in 0..3 {
            if omit_second && index == 1 {
                continue;
            }
            gpt[index as u32 + 1] = gptman::GPTPartitionEntry {
                partition_type_guid: [0x22; 16],
                unique_partition_guid: [index as u8 + 1; 16],
                starting_lba: STARTS[index],
                ending_lba: STARTS[index] + CAPACITIES[index] - 1,
                attribute_bits: 0,
                partition_name: LABELS[index].into(),
            };
        }
        gpt.write_into(&mut disk).unwrap();
        let events = Events::default();
        let transport = Transport {
            disk: disk.into_inner(),
            sector_size,
            responses: Cursor::new(Vec::new()),
            events: Arc::clone(&events),
            program_count: 0,
            fault,
            payload_remaining: 0,
            final_ack_gate,
        };
        let session = EdlSession {
            dev: QdlDevice {
                rw: Box::new(transport),
                fh_cfg: FirehoseConfiguration {
                    storage_sector_size: sector_size,
                    send_buffer_size: sector_size * 2,
                    backend: QdlBackend::Serial,
                    ..FirehoseConfiguration::default()
                },
                reset_on_drop: false,
            },
            lun_sectors: std::collections::BTreeMap::new(),
            exited: false,
        };
        Self {
            dir,
            images,
            session,
            events,
        }
    }

    fn flash(&mut self, count: usize) -> Result<(), PartitionFlashError> {
        let requests: Vec<_> = (0..count)
            .map(|index| PartitionFlash {
                label: LABELS[index],
                image: &self.images[index],
                slot: 0,
                lun: 4,
            })
            .collect();
        let events = &self.events;
        self.session.flash_partition_batch(
            &requests,
            &mut Vec::new(),
            |label, _, _| events.lock().unwrap().push(Event::Partition(label.into())),
            || events.lock().unwrap().push(Event::Start),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for image in &self.images {
            let _ = std::fs::remove_file(image);
        }
        let _ = std::fs::remove_dir(&self.dir);
    }
}

#[test]
fn second_image_preflight_failure_prevents_all_programs_and_notifications() {
    for invalid in ["missing", "empty", "oversized"] {
        let mut fixture = Fixture::new(512, false, None);
        match invalid {
            "missing" => std::fs::remove_file(&fixture.images[1]).unwrap(),
            "empty" => std::fs::write(&fixture.images[1], []).unwrap(),
            // One byte over capacity requires another full sector on the wire.
            "oversized" => std::fs::write(&fixture.images[1], vec![1; 4 * 512 + 1]).unwrap(),
            _ => unreachable!(),
        }
        let error = fixture.flash(2).unwrap_err();
        assert_eq!(error.partition, LABELS[1], "{invalid}");
        assert!(fixture.events.lock().unwrap().is_empty(), "{invalid}");
    }
}

#[test]
fn missing_second_target_prevents_all_programs_and_notifications() {
    let mut fixture = Fixture::new(512, true, None);
    let error = fixture.flash(2).unwrap_err();
    assert_eq!(error.partition, LABELS[1]);
    assert!(matches!(
        error.source,
        super::EdlError::PartitionNotFound(_)
    ));
    assert!(fixture.events.lock().unwrap().is_empty());
}

#[test]
fn valid_pair_uses_actual_capacity_and_rounded_transfer_lengths_in_order() {
    for sector_size in [512, 4096] {
        let mut fixture = Fixture::new(sector_size, false, None);
        fixture.flash(2).unwrap();
        let mut expected = Vec::new();
        for index in 0..2 {
            let mut padded = vec![0xa1 + index as u8; sector_size + 1];
            padded.resize(sector_size * 2, 0);
            expected.extend([
                Event::Partition(LABELS[index].into()),
                Event::Start,
                Event::Program(STARTS[index], 2),
                Event::Payload(padded),
            ]);
        }
        assert_eq!(*fixture.events.lock().unwrap(), expected);
    }
}

#[test]
fn failed_program_retains_start_notification_and_stops_remaining_writes() {
    for fail_program in [1, 2] {
        let mut fixture = Fixture::new(512, false, Some(Fault::CommandWrite(fail_program)));
        let error = fixture.flash(3).unwrap_err();
        assert_eq!(error.partition, LABELS[fail_program - 1]);
        let events = fixture.events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::Start))
                .count(),
            fail_program
        );
        let programs: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::Program(start, _) => Some(*start),
                _ => None,
            })
            .collect();
        assert_eq!(programs, STARTS[..fail_program]);
        assert_eq!(
            events.last(),
            Some(&Event::Program(STARTS[fail_program - 1], 2))
        );
    }
}

fn expected_program(index: usize, payload_bytes: Option<usize>) -> Vec<Event> {
    let mut events = vec![
        Event::Partition(LABELS[index].into()),
        Event::Start,
        Event::Program(STARTS[index], 2),
    ];
    if let Some(length) = payload_bytes {
        let mut payload = vec![0xa1 + index as u8; 513];
        payload.resize(1024, 0);
        payload.truncate(length);
        events.push(Event::Payload(payload));
    }
    events
}

#[test]
fn transport_faults_retain_attempted_partition_and_stop_batch() {
    for operation in [1, 2] {
        for (fault, payload_bytes) in [
            (Fault::PartialPayload(operation), Some(137)),
            (Fault::DisconnectedPayload(operation), None),
            (Fault::CommandNak(operation), None),
            (Fault::CommandTimeout(operation), None),
            (Fault::FinalNak(operation), Some(1024)),
            (Fault::FinalTimeout(operation), Some(1024)),
        ] {
            let mut fixture = Fixture::new(512, false, Some(fault));
            let error = fixture.flash(3).unwrap_err();
            assert_eq!(error.partition, LABELS[operation - 1], "{fault:?}");
            let mut expected = Vec::new();
            for index in 0..operation - 1 {
                expected.extend(expected_program(index, Some(1024)));
            }
            expected.extend(expected_program(operation - 1, payload_bytes));
            assert_eq!(*fixture.events.lock().unwrap(), expected, "{fault:?}");
        }
    }
}

#[test]
fn gpt_transport_faults_prevent_all_write_starts_and_programs() {
    for fault in [Fault::GptDisconnect, Fault::GptTimeout] {
        let mut fixture = Fixture::new(512, false, Some(fault));
        let error = fixture.flash(3).unwrap_err();
        assert_eq!(error.partition, LABELS[0], "{fault:?}");
        assert!(fixture.events.lock().unwrap().is_empty(), "{fault:?}");
    }
}

#[test]
fn delayed_final_ack_blocks_next_partition_until_released() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut fixture = Fixture::with_gate(
        512,
        false,
        None,
        Some(AckGate {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        }),
    );
    let events = Arc::clone(&fixture.events);
    let worker = std::thread::spawn(move || fixture.flash(2));
    let entered = entered_rx.recv_timeout(std::time::Duration::from_secs(5));
    // Capture before release, but always release and join before asserting so
    // a failed assertion cannot strand a worker waiting for the test thread.
    let while_waiting = events.lock().unwrap().clone();
    let release = release_tx.send(());
    let result = worker.join();
    entered.expect("worker must reach first final ACK");
    release.expect("worker must still be waiting for ACK release");
    result.unwrap().unwrap();
    assert_eq!(while_waiting, expected_program(0, Some(1024)));
    let mut expected = expected_program(0, Some(1024));
    expected.extend(expected_program(1, Some(1024)));
    // The worker drops the un-reset session when it finishes.
    expected.push(Event::Reset("reset_to_edl".into()));
    assert_eq!(*events.lock().unwrap(), expected);
}

#[test]
fn partition_snapshot_rejects_stale_coordinates_and_sector_size_before_programming() {
    for mismatch in [
        "none",
        "start",
        "length",
        "sector-size",
        "label",
        "duplicate",
    ] {
        let mut fixture = Fixture::new(512, false, None);
        let mut row = super::super::GptPartitionInfo {
            lun: 4,
            name: LABELS[0].into(),
            start_sector: STARTS[0],
            num_sectors: CAPACITIES[0],
            size_bytes: CAPACITIES[0] * 512,
        };
        match mismatch {
            "start" => row.start_sector += 1,
            "length" => row.num_sectors += 1,
            "sector-size" => row.size_bytes *= 8,
            "label" => row.name = "absent".into(),
            _ => {}
        }
        let rows = if mismatch == "duplicate" {
            vec![row.clone(), row]
        } else {
            vec![row]
        };
        assert_eq!(
            fixture.session.validate_partition_snapshot(&rows).is_ok(),
            mismatch == "none"
        );
        assert!(fixture.events.lock().unwrap().is_empty());
    }
}

#[test]
fn mixed_batch_validates_later_sources_before_erasing_and_preserves_order() {
    for valid in [false, true] {
        let mut fixture = Fixture::new(512, false, None);
        if !valid {
            std::fs::write(&fixture.images[1], []).unwrap();
        }
        let requests = [
            super::PartitionOperation {
                label: LABELS[0],
                image: None,
                slot: 0,
                lun: 4,
            },
            super::PartitionOperation {
                label: LABELS[1],
                image: Some(&fixture.images[1]),
                slot: 0,
                lun: 4,
            },
        ];
        let result =
            fixture
                .session
                .apply_partition_batch(&requests, &mut Vec::new(), |_, _| {}, || {});
        assert_eq!(result.is_ok(), valid);
        let events = fixture.events.lock().unwrap();
        if valid {
            assert_eq!(events[0], Event::Erase(STARTS[0], CAPACITIES[0] as usize));
            assert_eq!(events[1], Event::Program(STARTS[1], 2));
        } else {
            assert!(events.is_empty());
        }
    }
}

#[test]
fn partition_span_rejects_overflow() {
    assert!(EdlSession::partition_span_sectors("overflow", 0, u64::MAX).is_err());
}

#[test]
fn single_image_apis_reject_empty_sources_without_programming() {
    for api in ["partition", "coordinates", "physical"] {
        let mut fixture = Fixture::new(512, false, None);
        std::fs::write(&fixture.images[0], []).unwrap();
        let mut log = Vec::new();
        let result = match api {
            "partition" => {
                fixture
                    .session
                    .flash_partition(LABELS[0], &fixture.images[0], 0, 4, &mut log)
            }
            "coordinates" => fixture.session.flash_partition_at(
                LABELS[0],
                &fixture.images[0],
                4,
                "64",
                CAPACITIES[0],
                &mut log,
            ),
            _ => fixture
                .session
                .flash_physical_storage(4, &fixture.images[0], &mut log),
        };
        assert!(result.is_err(), "{api}");
        assert!(fixture.events.lock().unwrap().is_empty());
    }
}

#[test]
fn efisp_erase_programs_exact_zero_image_without_readback() {
    for sector_size in [512, 4096] {
        let mut fixture = Fixture::new(sector_size, false, None);
        fixture
            .session
            .erase_partition_at("efisp", 4, "64", 3, &mut Vec::new())
            .unwrap();
        let events = fixture.events.lock().unwrap();
        assert_eq!(events[0], Event::Program(64, 3));
        let mut total = 0;
        for event in &events[1..] {
            let Event::Payload(bytes) = event else {
                panic!("Unexpected event: {event:?}")
            };
            assert!(bytes.iter().all(|b| *b == 0));
            total += bytes.len();
        }
        assert_eq!(total, 3 * sector_size);
    }
}

#[test]
fn efisp_zero_write_propagates_transfer_failures_and_does_not_report_completion() {
    for fault in [
        Fault::CommandWrite(1),
        Fault::DisconnectedPayload(1),
        Fault::FinalNak(1),
        Fault::FinalTimeout(1),
    ] {
        let mut fixture = Fixture::new(4096, false, Some(fault));
        let mut log = Vec::new();
        assert!(
            fixture
                .session
                .erase_partition_at("efisp", 4, "64", 3, &mut log)
                .is_err()
        );
        assert!(!log.iter().any(|line| line
            == &format!(
                "[EDL] {}",
                ltbox_core::i18n::tr("log_edl_erased_part").replace("{part}", "efisp")
            )));
    }
}

#[test]
fn efisp_zero_write_rejects_empty_geometry_before_commands() {
    let mut fixture = Fixture::new(4096, false, None);
    assert!(
        fixture
            .session
            .erase_partition_at("efisp", 4, "64", 0, &mut Vec::new())
            .is_err()
    );
    assert!(fixture.events.lock().unwrap().is_empty());
}

#[test]
fn rawprogram_efisp_erase_uses_zero_program_but_other_labels_keep_erase() {
    for label in ["efisp", "userdata", ""] {
        let mut fixture = Fixture::new(4096, false, None);
        let xml = format!(
            r#"<data><erase label="{label}" physical_partition_number="4" start_sector="64" num_partition_sectors="3" /></data>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let node = doc.root_element().first_element_child().unwrap();
        fixture
            .session
            .erase_program_node(&node, &mut Vec::new())
            .unwrap();
        let events = fixture.events.lock().unwrap();
        if label == "efisp" {
            assert_eq!(events[0], Event::Program(64, 3));
            assert!(events[1..].iter().all(|event| matches!(event, Event::Payload(bytes) if bytes.iter().all(|byte| *byte == 0))));
        } else {
            assert_eq!(*events, vec![Event::Erase(64, 3)]);
        }
    }
}

#[test]
fn dropping_a_session_without_a_reset_returns_the_device_to_edl() {
    let fixture = Fixture::new(512, false, None);
    let events = Arc::clone(&fixture.events);
    drop(fixture);
    assert_eq!(
        *events.lock().unwrap(),
        vec![Event::Reset("reset_to_edl".into())]
    );
}

#[test]
fn an_explicit_reset_is_not_followed_by_a_drop_time_reset() {
    let mut fixture = Fixture::new(512, false, None);
    let events = Arc::clone(&fixture.events);
    fixture.session.reset_tolerant(&mut Vec::new());
    drop(fixture);
    assert_eq!(*events.lock().unwrap(), vec![Event::Reset("reset".into())]);
}

#[test]
fn a_failed_transfer_still_resets_to_edl_when_the_session_drops() {
    let mut fixture = Fixture::new(512, false, Some(Fault::CommandNak(1)));
    let events = Arc::clone(&fixture.events);
    assert!(fixture.flash(2).is_err());
    drop(fixture);
    assert_eq!(
        events.lock().unwrap().last(),
        Some(&Event::Reset("reset_to_edl".into()))
    );
}

// Exercise the public rawprogram entrypoints, including wipe pre-erase.
fn run_rawprogram(
    fixture: &mut Fixture,
    mode: &str,
    programs: &[PathBuf],
    patches: &[PathBuf],
) -> super::Result<()> {
    match mode {
        "verbatim" => fixture
            .session
            .flash_rawprogram_verbatim(programs, patches, &mut Vec::new()),
        _ => fixture.session.flash_rawprogram_with_wipe(
            programs,
            patches,
            mode == "wipe",
            &mut Vec::new(),
        ),
    }
}

fn rawprogram_fixture(dir: &std::path::Path) -> PathBuf {
    std::fs::write(dir.join("image.bin"), vec![0x5a; 512]).unwrap();
    let path = dir.join("rawprogram.xml");
    std::fs::write(&path, r#"<data>
        <program label="boot_a" filename="image.bin" physical_partition_number="4" start_sector="64" num_partition_sectors="1" />
        <program label="userdata" filename="image.bin" physical_partition_number="4" start_sector="160" num_partition_sectors="1" />
    </data>"#).unwrap();
    path
}

#[test]
fn rawprogram_rejects_later_invalid_inputs_before_any_destructive_command() {
    for mode in ["wipe", "keep", "verbatim"] {
        for (patch, invalid) in [
            (false, "<data>"),
            (
                false,
                r#"<data><program filename="image.bin" physical_partition_number="4" start_sector="255" num_partition_sectors="2" /></data>"#,
            ),
            (true, "<data>"),
            (
                true,
                r#"<data><patch filename="DISK" start_sector="0" /></data>"#,
            ),
            (
                true,
                r#"<data><patch filename="DISK" physical_partition_number="4" start_sector="0" size_in_bytes="bad" /></data>"#,
            ),
        ] {
            let mut fixture = Fixture::new(512, false, None);
            let dir = crate::edl::test_support::TempFirmwareDir::new();
            let first = rawprogram_fixture(dir.path());
            let second = dir.path().join("later.xml");
            std::fs::write(&second, invalid).unwrap();
            let (programs, patches) = if patch {
                let valid_patch = dir.path().join("first_patch.xml");
                std::fs::write(&valid_patch, r#"<data><patch filename="DISK" physical_partition_number="4" start_sector="0" size_in_bytes="4" value="1" /></data>"#).unwrap();
                (vec![first], vec![valid_patch, second])
            } else {
                (vec![first, second], vec![])
            };
            assert!(
                run_rawprogram(&mut fixture, mode, &programs, &patches).is_err(),
                "{mode}: {invalid}"
            );
            assert!(
                fixture.events.lock().unwrap().is_empty(),
                "{mode}: destructive I/O before validation completed"
            );
        }
    }
}

#[test]
fn rawprogram_applies_disk_patches_after_programs_and_preserves_expressions() {
    for mode in ["wipe", "keep", "verbatim"] {
        let mut fixture = Fixture::new(512, false, None);
        let dir = crate::edl::test_support::TempFirmwareDir::new();
        let program = rawprogram_fixture(dir.path());
        let patch = dir.path().join("patch.xml");
        std::fs::write(&patch, r#"<data>
            <patch filename="image.bin" />
            <patch filename="DISK" physical_partition_number="4" start_sector="NUM_DISK_SECTORS-1." byte_offset="16" size_in_bytes="4" value="CRC32(0,512)" />
        </data>"#).unwrap();
        run_rawprogram(&mut fixture, mode, &[program], &[patch]).unwrap();
        let events = fixture.events.lock().unwrap();
        let mut expected = Vec::new();
        if mode == "wipe" {
            expected.push(Event::Erase(160, 1));
        }
        expected.extend([Event::Program(64, 1), Event::Payload(vec![0x5a; 512])]);
        if mode != "keep" {
            expected.extend([Event::Program(160, 1), Event::Payload(vec![0x5a; 512])]);
        }
        expected.push(Event::Patch(
            "NUM_DISK_SECTORS-1.".into(),
            "CRC32(0,512)".into(),
        ));
        assert_eq!(*events, expected, "{mode}");
    }
}

#[test]
fn keep_data_skips_userdata_and_metadata_erases_but_wipe_sends_them() {
    for mode in ["keep", "wipe"] {
        let mut fixture = Fixture::new(512, false, None);
        let dir = crate::edl::test_support::TempFirmwareDir::new();
        let xml = dir.path().join("rawprogram.xml");
        std::fs::write(&xml, r#"<data>
            <erase label="userdata" physical_partition_number="4" start_sector="160" num_partition_sectors="2" />
            <erase label="metadata_a" physical_partition_number="4" start_sector="170" num_partition_sectors="2" />
            <erase label="cache" physical_partition_number="4" start_sector="180" num_partition_sectors="2" />
        </data>"#).unwrap();
        run_rawprogram(&mut fixture, mode, &[xml], &[]).unwrap();
        let erases: Vec<_> = fixture
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::Erase(start, _) => Some(*start),
                _ => None,
            })
            .collect();
        if mode == "keep" {
            assert_eq!(erases, vec![180], "{mode}");
        } else {
            assert!(
                erases.contains(&160) && erases.contains(&170) && erases.contains(&180),
                "{mode}: {erases:?}"
            );
        }
    }
}

#[test]
fn rawprogram_rejects_empty_or_non_file_images_before_any_destructive_command() {
    for mode in ["wipe", "keep", "verbatim"] {
        for broken in ["empty", "directory"] {
            let mut fixture = Fixture::new(512, false, None);
            let dir = crate::edl::test_support::TempFirmwareDir::new();
            let first = rawprogram_fixture(dir.path());
            match broken {
                "empty" => std::fs::write(dir.path().join("dtbo.img"), []).unwrap(),
                _ => std::fs::create_dir(dir.path().join("dtbo.img")).unwrap(),
            }
            let second = dir.path().join("later.xml");
            std::fs::write(&second, r#"<data><program label="dtbo_a" filename="dtbo.img" physical_partition_number="4" start_sector="200" num_partition_sectors="2" /></data>"#).unwrap();
            assert!(
                run_rawprogram(&mut fixture, mode, &[first, second], &[]).is_err(),
                "{mode}: {broken}"
            );
            assert!(
                fixture.events.lock().unwrap().is_empty(),
                "{mode}: {broken}: destructive I/O before validation completed"
            );
        }
    }
}
