//! Vendored qdl against a device that misbehaves on purpose.

use std::io::{self, BufRead, Read, Write};

use qdl::types::{FirehoseConfiguration, QdlBackend, QdlChan};

/// Replays `reply` forever and records every byte the host sends.
struct LoopingChannel {
    reply: Vec<u8>,
    cursor: usize,
    writes: Vec<u8>,
    config: FirehoseConfiguration,
}

impl LoopingChannel {
    fn new(reply: &[u8]) -> Self {
        Self {
            reply: reply.to_vec(),
            cursor: 0,
            writes: Vec::new(),
            config: FirehoseConfiguration {
                backend: QdlBackend::Serial,
                ..FirehoseConfiguration::default()
            },
        }
    }
}

impl Read for LoopingChannel {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(buf.len());
        buf[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for LoopingChannel {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.cursor >= self.reply.len() {
            self.cursor = 0;
        }
        Ok(&self.reply[self.cursor..])
    }

    fn consume(&mut self, amount: usize) {
        self.cursor = (self.cursor + amount).min(self.reply.len());
    }
}

impl Write for LoopingChannel {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl QdlChan for LoopingChannel {
    fn fh_config(&self) -> &FirehoseConfiguration {
        &self.config
    }

    fn mut_fh_config(&mut self) -> &mut FirehoseConfiguration {
        &mut self.config
    }
}

fn power_requests(writes: &[u8]) -> usize {
    String::from_utf8_lossy(writes).matches("<power").count()
}

/// A NAKed reset must not retry recursively: unbounded retries overflow the
/// stack and abort the process.
#[test]
fn reset_that_is_always_nakked_fails_after_one_nested_attempt() {
    let mut channel = LoopingChannel::new(b"<data><response value=\"NAK\"/></data>");
    let result = qdl::firehose_reset(&mut channel, &qdl::types::FirehoseResetMode::Reset, 0);
    assert!(result.is_err());
    assert_eq!(power_requests(&channel.writes), 1);
}

#[test]
fn reset_answered_with_attempt_restart_does_not_recurse() {
    let mut channel =
        LoopingChannel::new(b"<data><response value=\"NAK\" AttemptRestart=\"1\"/></data>");
    let result = qdl::firehose_reset(&mut channel, &qdl::types::FirehoseResetMode::Reset, 0);
    assert!(result.is_err());
    assert_eq!(power_requests(&channel.writes), 1);
}

/// A memory-debug request while LTBox is uploading a programmer must not
/// start a ramdump: the dump table names host files.
#[test]
fn unsolicited_memory_debug_request_is_refused_before_any_reply() {
    let mut packet = Vec::new();
    packet.extend_from_slice(&0x10u32.to_le_bytes()); // SaharaMemDebug64
    packet.extend_from_slice(&0x18u32.to_le_bytes());
    packet.extend_from_slice(&0x1000u64.to_le_bytes());
    packet.extend_from_slice(&0x18u64.to_le_bytes());
    let mut channel = LoopingChannel::new(&packet);
    let error = qdl::sahara::sahara_run(
        &mut channel,
        qdl::sahara::SaharaMode::WaitingForImage,
        None,
        &mut [],
        Vec::new(),
        false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("unsolicited"), "{error:#}");
    assert!(channel.writes.is_empty());
}
