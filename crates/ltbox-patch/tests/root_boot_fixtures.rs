//! Real-image TB320FC boot root-patch coverage.
//!
//! The repository does not carry firmware or patched-image fixtures. Opt in:
//!
//! ```powershell
//! $env:LTBOX_TB320FC_BOOT = 'D:\fixtures\boot.img'
//! $env:LTBOX_TB320FC_VBMETA = 'D:\fixtures\vbmeta.img'
//! $env:LTBOX_TB320FC_KSU_BOOT_PATCHED = 'D:\fixtures\kernelsu_patched.img'
//! $env:LTBOX_TB320FC_MAGISK_BOOT_PATCHED = 'D:\fixtures\magisk_patched.img'
//! cargo test -p ltbox-patch --test root_boot_fixtures -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use ltbox_patch::root_pipeline::{
    RootFamily, RootImageTarget, RootPipelineConfig, RootProvider, RootVersion,
    build_patched_artifacts,
};
use ltbox_patch::{avb, boot};

const BOOT_HEADER_V4_SIZE: usize = 1_584;
const BOOT_CMDLINE_OFFSET: usize = 44;
const BOOT_CMDLINE_SIZE: usize = 1_536;

struct FixturePaths {
    stock: PathBuf,
    vbmeta: PathBuf,
    ksu_reference: PathBuf,
    magisk_reference: PathBuf,
}

impl FixturePaths {
    fn from_env() -> Option<Self> {
        let names = [
            "LTBOX_TB320FC_BOOT",
            "LTBOX_TB320FC_VBMETA",
            "LTBOX_TB320FC_KSU_BOOT_PATCHED",
            "LTBOX_TB320FC_MAGISK_BOOT_PATCHED",
        ];
        let missing = names
            .iter()
            .filter(|name| env::var_os(name).is_none())
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            println!("skipping TB320FC boot fixtures; unset: {missing:?}");
            return None;
        }
        Some(Self {
            stock: env_path(names[0]),
            vbmeta: env_path(names[1]),
            ksu_reference: env_path(names[2]),
            magisk_reference: env_path(names[3]),
        })
    }
}

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(env::var_os(name).expect("environment variable was checked"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BootHeader {
    kernel_size: u32,
    ramdisk_size: u32,
    os_version: u32,
    header_size: u32,
    reserved: [u32; 4],
    version: u32,
    cmdline: Vec<u8>,
    signature_size: u32,
}

struct UnpackedImage {
    _temp: tempfile::TempDir,
    dir: PathBuf,
    header: BootHeader,
    ramdisk: PathBuf,
}

fn unpack_image(image: &Path) -> UnpackedImage {
    let bytes = std::fs::read(image).unwrap();
    assert!(bytes.starts_with(b"ANDROID!"), "{}", image.display());
    assert!(bytes.len() >= BOOT_HEADER_V4_SIZE);
    let header = BootHeader {
        kernel_size: read_u32(&bytes, 8),
        ramdisk_size: read_u32(&bytes, 12),
        os_version: read_u32(&bytes, 16),
        header_size: read_u32(&bytes, 20),
        reserved: [
            read_u32(&bytes, 24),
            read_u32(&bytes, 28),
            read_u32(&bytes, 32),
            read_u32(&bytes, 36),
        ],
        version: read_u32(&bytes, 40),
        cmdline: bytes[BOOT_CMDLINE_OFFSET..BOOT_CMDLINE_OFFSET + BOOT_CMDLINE_SIZE].to_vec(),
        signature_size: read_u32(&bytes, 1_580),
    };

    let temp = tempfile::tempdir().unwrap();
    let report = magiskboot::bootimg::unpack(image, temp.path(), false, false).unwrap();
    assert!(!report.is_vendor);
    assert_eq!(report.header_version, header.version);
    let ramdisk = temp.path().join("ramdisk.cpio");
    assert!(ramdisk.is_file());
    let dir = temp.path().to_path_buf();
    UnpackedImage {
        _temp: temp,
        dir,
        header,
        ramdisk,
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn build_fixture_output(
    paths: &FixturePaths,
    family: RootFamily,
    reference: &UnpackedImage,
) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().join("work");
    let output = temp.path().join("out");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::copy(&paths.stock, work.join("boot.img")).unwrap();
    std::fs::copy(&paths.vbmeta, work.join("vbmeta.img")).unwrap();
    std::fs::write(work.join("manager.apk"), []).unwrap();

    let reference_cpio = parse_newc(&std::fs::read(&reference.ramdisk).unwrap());
    let preinit_device = match family {
        RootFamily::Magisk => {
            stage_magisk_payload(&work, &reference_cpio);
            config_value(&reference_cpio, "PREINITDEVICE").unwrap_or_default()
        }
        RootFamily::KernelSU => {
            stage_ksu_payload(&work, &reference_cpio);
            String::new()
        }
        _ => unreachable!("fixture only covers ramdisk root families"),
    };

    let provider = match family {
        RootFamily::Magisk => RootProvider::Magisk,
        RootFamily::KernelSU => RootProvider::KernelSU,
        _ => unreachable!("fixture only covers ramdisk root families"),
    };
    let config = RootPipelineConfig {
        family,
        provider,
        version: RootVersion::Stable,
        root_image_target: RootImageTarget::Boot,
        // TB320FC hashes boot in vbmeta rather than chaining it.
        rebuild_vbmeta: true,
        work_dir: work,
        output_dir: output,
        loader: PathBuf::new(),
        slot_suffix: "_a".into(),
        preinit_device,
        gki_kernel_zip: None,
        kernel_version: Some("6.1.0".into()),
        kernel_gki_branch: None,
        gki_mode: false,
        kpm_paths: Vec::new(),
        superkey: String::new(),
        magisk_forks_apk: None,
        nightly_run_id: None,
        release_tag: None,
    };
    let mut log = Vec::new();
    let artifacts = build_patched_artifacts(&config, false, &mut log).unwrap();
    assert_eq!(artifacts.root_partition, "boot_a");
    let log_text = log.join("\n");
    assert!(log_text.contains("boot.img"), "{log_text}");
    assert!(!log_text.contains("init_boot.img"), "{log_text}");
    (
        temp,
        artifacts.patched_root_image,
        artifacts.patched_vbmeta.unwrap(),
    )
}

fn stage_magisk_payload(work: &Path, cpio: &BTreeMap<String, CpioEntry>) {
    std::fs::write(
        work.join("magiskinit"),
        entry(cpio, "init").contents.as_slice(),
    )
    .unwrap();
    for (archive_path, compressed_name, output_name) in [
        ("overlay.d/sbin/magisk.xz", "magisk.xz", "magisk"),
        ("overlay.d/sbin/stub.xz", "stub.xz", "stub.apk"),
        ("overlay.d/sbin/init-ld.xz", "init-ld.xz", "init-ld"),
    ] {
        std::fs::write(
            work.join(compressed_name),
            entry(cpio, archive_path).contents.as_slice(),
        )
        .unwrap();
        boot::decompress(work, compressed_name, output_name).unwrap();
    }
}

fn stage_ksu_payload(work: &Path, cpio: &BTreeMap<String, CpioEntry>) {
    std::fs::write(work.join("init"), entry(cpio, "init").contents.as_slice()).unwrap();
    std::fs::write(
        work.join("kernelsu.ko"),
        entry(cpio, "kernelsu.ko").contents.as_slice(),
    )
    .unwrap();
}

fn config_value(cpio: &BTreeMap<String, CpioEntry>, key: &str) -> Option<String> {
    let config = std::str::from_utf8(&entry(cpio, ".backup/.magisk").contents).unwrap();
    config.lines().find_map(|line| {
        line.strip_prefix(key)
            .and_then(|value| value.strip_prefix('='))
            .map(str::to_owned)
    })
}

fn entry<'a>(cpio: &'a BTreeMap<String, CpioEntry>, path: &str) -> &'a CpioEntry {
    cpio.get(path).unwrap_or_else(|| {
        let related = cpio
            .keys()
            .filter(|candidate| candidate.contains("init") || candidate.contains("kernel"))
            .collect::<Vec<_>>();
        panic!("missing CPIO entry {path}; related entries: {related:?}")
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CpioEntry {
    file_type: u32,
    mode: u32,
    uid: u32,
    gid: u32,
    contents: Vec<u8>,
}

fn parse_newc(bytes: &[u8]) -> BTreeMap<String, CpioEntry> {
    const HEADER_SIZE: usize = 110;
    const FILE_TYPE_MASK: u32 = 0o170_000;
    const MODE_MASK: u32 = 0o7_777;
    let mut entries = BTreeMap::new();
    let mut offset = 0usize;
    while offset + HEADER_SIZE <= bytes.len() {
        let header = &bytes[offset..offset + HEADER_SIZE];
        assert!(matches!(&header[..6], b"070701" | b"070702"));
        let mode = parse_hex(&header[14..22]);
        let uid = parse_hex(&header[22..30]);
        let gid = parse_hex(&header[30..38]);
        let size = parse_hex(&header[54..62]) as usize;
        let name_size = parse_hex(&header[94..102]) as usize;
        let name_start = offset + HEADER_SIZE;
        let name_end = name_start + name_size;
        assert!(name_size > 0 && name_end <= bytes.len());
        let name = std::str::from_utf8(&bytes[name_start..name_end - 1])
            .unwrap()
            .to_string();
        let data_start = align4(name_end);
        let data_end = data_start + size;
        assert!(data_end <= bytes.len());
        offset = align4(data_end);
        if name == "TRAILER!!!" {
            let Some(next) = bytes[offset..]
                .windows(6)
                .position(|window| matches!(window, b"070701" | b"070702"))
            else {
                break;
            };
            offset += next;
            continue;
        }
        entries.insert(
            name,
            CpioEntry {
                file_type: mode & FILE_TYPE_MASK,
                mode: mode & MODE_MASK,
                uid,
                gid,
                contents: bytes[data_start..data_end].to_vec(),
            },
        );
    }
    assert!(!entries.is_empty());
    entries
}

fn parse_hex(bytes: &[u8]) -> u32 {
    u32::from_str_radix(std::str::from_utf8(bytes).unwrap(), 16).unwrap()
}

const fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn assert_boot_shape(stock: &UnpackedImage, other: &UnpackedImage) {
    assert_eq!(other.header.version, stock.header.version);
    assert_eq!(other.header.kernel_size, stock.header.kernel_size);
    assert_eq!(other.header.os_version, stock.header.os_version);
    assert_eq!(other.header.header_size, stock.header.header_size);
    assert_eq!(other.header.reserved, stock.header.reserved);
    assert_eq!(other.header.cmdline, stock.header.cmdline);
    assert_eq!(other.header.signature_size, stock.header.signature_size);

    assert_eq!(
        read_component(&other.dir, "kernel"),
        read_component(&stock.dir, "kernel")
    );
    for name in [
        "dtb",
        "second",
        "extra",
        "recovery_dtbo",
        "bootconfig",
        "signature",
    ] {
        assert_optional_component_eq(&stock.dir, &other.dir, name);
    }
}

fn read_component(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap()
}

fn assert_optional_component_eq(stock_dir: &Path, other_dir: &Path, name: &str) {
    let stock = std::fs::read(stock_dir.join(name));
    let other = std::fs::read(other_dir.join(name));
    match (stock, other) {
        (Ok(stock), Ok(other)) => assert_eq!(other, stock, "{name}"),
        (Err(stock), Err(other))
            if stock.kind() == std::io::ErrorKind::NotFound
                && other.kind() == std::io::ErrorKind::NotFound => {}
        (stock, other) => panic!("{name} presence differs: stock={stock:?}, other={other:?}"),
    }
}

fn normalized_cpio(image: &UnpackedImage) -> BTreeMap<String, CpioEntry> {
    parse_newc(&std::fs::read(&image.ramdisk).unwrap())
}

fn assert_reference_cpio(reference: &UnpackedImage, ours: &UnpackedImage) {
    let reference_entries = normalized_cpio(reference);
    let our_entries = normalized_cpio(ours);
    assert_eq!(our_entries, reference_entries);
}

fn hash_descriptor(path: &Path) -> avbtool_rs::info::DescriptorInfo {
    avbtool_rs::image::inspect_avb_image(path)
        .unwrap()
        .descriptors
        .into_iter()
        .find(|descriptor| {
            matches!(
                descriptor,
                avbtool_rs::info::DescriptorInfo::Hash { partition_name, .. }
                    if partition_name == "boot"
            )
        })
        .expect("boot Hash descriptor")
}

fn assert_avb_pair(boot: &Path, vbmeta: &Path) {
    let info = avb::extract_image_avb_info(boot).unwrap();
    assert_eq!(info.partition_name.as_deref(), Some("boot"));
    assert_eq!(hash_descriptor(boot), hash_descriptor(vbmeta));
}

#[test]
#[ignore = "requires locally supplied TB320FC stock and app-patched images"]
fn real_tb320fc_boot_matches_magisk_and_ksu_apps() {
    let Some(paths) = FixturePaths::from_env() else {
        return;
    };

    let stock_size = std::fs::metadata(&paths.stock).unwrap().len();
    assert_eq!(stock_size, 96 * 1024 * 1024);
    assert_eq!(
        std::fs::metadata(&paths.ksu_reference).unwrap().len(),
        stock_size
    );
    assert_eq!(
        std::fs::metadata(&paths.magisk_reference).unwrap().len(),
        stock_size
    );

    let stock = unpack_image(&paths.stock);
    let ksu_reference = unpack_image(&paths.ksu_reference);
    let magisk_reference = unpack_image(&paths.magisk_reference);
    for image in [&stock, &ksu_reference, &magisk_reference] {
        assert_eq!(image.header.version, 4);
        assert_eq!(image.header.header_size as usize, BOOT_HEADER_V4_SIZE);
    }
    assert_boot_shape(&stock, &ksu_reference);
    assert_boot_shape(&stock, &magisk_reference);

    let (_ksu_temp, ksu_output, ksu_vbmeta) =
        build_fixture_output(&paths, RootFamily::KernelSU, &ksu_reference);
    let ksu_ours = unpack_image(&ksu_output);
    assert_boot_shape(&stock, &ksu_ours);
    assert_reference_cpio(&ksu_reference, &ksu_ours);
    let ksu_entries = normalized_cpio(&ksu_ours);
    for path in ["init", "kernelsu.ko"] {
        entry(&ksu_entries, path);
    }
    let stock_entries = normalized_cpio(&stock);
    assert_eq!(
        ksu_entries.contains_key("init.real"),
        stock_entries.contains_key("init"),
        "init.real is created exactly when a stock init was renamed"
    );
    assert_avb_pair(&ksu_output, &ksu_vbmeta);

    let (_magisk_temp, magisk_output, magisk_vbmeta) =
        build_fixture_output(&paths, RootFamily::Magisk, &magisk_reference);
    let magisk_ours = unpack_image(&magisk_output);
    assert_boot_shape(&stock, &magisk_ours);
    assert_reference_cpio(&magisk_reference, &magisk_ours);
    let magisk_reference_entries = normalized_cpio(&magisk_reference);
    let magisk_entries = normalized_cpio(&magisk_ours);
    for path in [
        "init",
        "overlay.d/sbin/magisk.xz",
        "overlay.d/sbin/stub.xz",
        "overlay.d/sbin/init-ld.xz",
        ".backup",
        ".backup/.magisk",
    ] {
        entry(&magisk_entries, path);
    }
    let reference_config = &entry(&magisk_reference_entries, ".backup/.magisk").contents;
    let config = &entry(&magisk_entries, ".backup/.magisk").contents;
    assert_eq!(
        config, reference_config,
        "Magisk config must match byte-for-byte"
    );
    let config = std::str::from_utf8(config).unwrap();
    for setting in [
        "VENDORBOOT=false",
        "KEEPVERITY=true",
        "KEEPFORCEENCRYPT=true",
    ] {
        assert!(config.lines().any(|line| line == setting), "{setting}");
    }
    assert!(
        !config
            .lines()
            .any(|line| line.starts_with("PATCHVBMETAFLAG="))
    );
    assert_avb_pair(&magisk_output, &magisk_vbmeta);

    println!(
        "TB320FC boot: v4 header and non-ramdisk components preserved, Magisk/KSU CPIO parity matched, AVB descriptors matched"
    );
}
