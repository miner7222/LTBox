use std::process::Command;

#[test]
fn helper_isolates_working_directories_and_environment() {
    let parent_dir = std::env::current_dir().unwrap();
    let parent_env = std::env::var_os("KEEPVERITY");
    let directories = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    std::thread::scope(|scope| {
        for (index, directory) in directories.iter().enumerate() {
            scope.spawn(move || {
                std::fs::write(directory.path().join("input"), vec![index as u8; 1024]).unwrap();
                let status = Command::new(env!("CARGO_BIN_EXE_ltbox-magiskboot"))
                    .args([
                        "--ltbox-internal-magiskboot",
                        "compress=gzip",
                        "input",
                        "output.gz",
                    ])
                    .env("KEEPVERITY", if index == 0 { "true" } else { "false" })
                    .current_dir(directory.path())
                    .status()
                    .unwrap();
                assert!(status.success());
                assert!(directory.path().join("output.gz").is_file());
            });
        }
    });
    assert_eq!(std::env::current_dir().unwrap(), parent_dir);
    assert_eq!(std::env::var_os("KEEPVERITY"), parent_env);
}

/// Set in the re-executed copy of this test binary; see below.
const PIN_CHILD_ENV: &str = "LTBOX_TEST_PINNED_ENV_CHILD";

/// A user's `XZBCJ=true` must not reach the helper through `boot::compress`,
/// while `compress_xz_bcj` must produce exactly what the helper writes with
/// the BCJ switch on. The test re-runs itself with the hostile variables set
/// so the parent process environment is never modified.
#[test]
fn helper_env_is_pinned_and_bcj_is_opt_in() {
    if std::env::var_os(PIN_CHILD_ENV).is_none() {
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "helper_env_is_pinned_and_bcj_is_opt_in",
                "--nocapture",
            ])
            .env(PIN_CHILD_ENV, "1")
            .env("XZBCJ", "true")
            .env("PATCHVBMETAFLAG", "true")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    assert_eq!(std::env::var("XZBCJ").as_deref(), Ok("true"));

    let helper = env!("CARGO_BIN_EXE_ltbox-magiskboot");
    ltbox_patch::boot::register_magiskboot_host(helper.into()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    // AArch64 ELF header followed by BL instructions, so the BCJ filter applies.
    let mut elf = vec![0u8; 64];
    elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    elf[18..20].copy_from_slice(&0xB7u16.to_le_bytes());
    for _ in 0..16_384 {
        elf.extend_from_slice(&0x9400_0001u32.to_le_bytes());
    }
    std::fs::write(dir.path().join("input"), &elf).unwrap();

    ltbox_patch::boot::compress(dir.path(), "xz", "input", "plain.xz").unwrap();
    ltbox_patch::boot::compress_xz_bcj(dir.path(), "input", "bcj.xz").unwrap();
    for (switch, output) in [("false", "ref_plain.xz"), ("true", "ref_bcj.xz")] {
        let status = Command::new(helper)
            .args([
                "--ltbox-internal-magiskboot",
                "compress=xz",
                "input",
                output,
            ])
            .env("XZBCJ", switch)
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
    }
    let read = |name: &str| std::fs::read(dir.path().join(name)).unwrap();
    assert_eq!(read("plain.xz"), read("ref_plain.xz"));
    assert_eq!(read("bcj.xz"), read("ref_bcj.xz"));
    assert_ne!(read("plain.xz"), read("bcj.xz"));
}
