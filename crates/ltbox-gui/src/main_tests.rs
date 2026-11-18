use super::*;

fn device_poll(model: &str) -> DevicePollResult {
    DevicePollResult {
        status: ConnectionStatus::Adb,
        model: model.to_string(),
        market_name: "Legion Y700".to_string(),
        ..DevicePollResult::default()
    }
}

#[test]
fn package_upgrade_commands_cover_every_install_source() {
    use ltbox_core::install_source::InstallSource;

    for (source, expected) in [
        (InstallSource::Scoop, Some("scoop update ltbox")),
        (
            InstallSource::WinGet,
            Some("winget upgrade miner7222.LTBox"),
        ),
        (InstallSource::Homebrew, Some("brew upgrade --cask ltbox")),
        (
            InstallSource::Deb,
            Some("sudo apt update && sudo apt upgrade ltbox"),
        ),
        (InstallSource::Rpm, Some("sudo dnf upgrade ltbox")),
        (InstallSource::OtherPackageManager, None),
        (InstallSource::Direct, None),
    ] {
        let actual = package_upgrade_command(source);
        assert_eq!(actual.command, expected.unwrap_or_default());
        assert_eq!(actual.available, expected.is_some());
    }
}

#[test]
fn primary_phase_plans_use_refined_counts() {
    assert_eq!(OperationPhaseKind::Flash.keys().len(), 9);
    assert_eq!(OperationPhaseKind::Root.keys().len(), 8);
    assert_eq!(OperationPhaseKind::Unroot.keys().len(), 6);
}

#[test]
fn system_update_phase_plans_match_each_action() {
    assert_eq!(OperationPhaseKind::SysUpdateDisable.keys().len(), 3);
    assert_eq!(OperationPhaseKind::SysUpdateEnable.keys().len(), 3);
    assert_eq!(OperationPhaseKind::BootRecovery.keys().len(), 7);
}

#[test]
fn advanced_edl_phase_plans_match_each_action() {
    assert_eq!(OperationPhaseKind::ChangeCountry.keys().len(), 5);
    assert_eq!(OperationPhaseKind::DetectArb.keys().len(), 5);
    assert_eq!(OperationPhaseKind::SimpleFlash.keys().len(), 5);
    assert_eq!(OperationPhaseKind::FlashPartitions.keys().len(), 3);
    assert_eq!(OperationPhaseKind::DumpPartitions.keys().len(), 4);
    assert_eq!(OperationPhaseKind::FlashPhysical.keys().len(), 4);
    assert_eq!(OperationPhaseKind::DumpPhysical.keys().len(), 5);
    assert_eq!(OperationPhaseKind::KonaBess.keys().len(), 7);
}

#[test]
fn offline_advanced_phase_plans_match_each_action() {
    assert_eq!(OperationPhaseKind::OfflineConvertXml.keys().len(), 3);
    assert_eq!(OperationPhaseKind::RegionConversion.keys().len(), 4);
    assert_eq!(OperationPhaseKind::PatchArb.keys().len(), 4);
    assert_eq!(OperationPhaseKind::RebuildVbmeta.keys().len(), 3);
    assert_eq!(
        OperationPhaseKind::for_advanced_file(AdvAction::ConvertXml),
        Some(OperationPhaseKind::OfflineConvertXml)
    );
    assert_eq!(
        OperationPhaseKind::for_advanced_file(AdvAction::RegionConvert),
        Some(OperationPhaseKind::RegionConversion)
    );
    assert_eq!(
        OperationPhaseKind::for_advanced_file(AdvAction::PatchArb),
        Some(OperationPhaseKind::PatchArb)
    );
    assert_eq!(
        OperationPhaseKind::for_advanced_file(AdvAction::RebuildVbmeta),
        Some(OperationPhaseKind::RebuildVbmeta)
    );
    assert_eq!(
        OperationPhaseKind::for_advanced_file(AdvAction::ImageInfo),
        None
    );
}

#[test]
fn operation_phase_reporter_marker_uses_the_same_snapshot_total_and_label() {
    install_core_translator(Language::En);
    let reporter =
        PhaseReporter::from_labels(vec!["Prepare".into(), "Write".into(), "Reboot".into()]);
    let worker = reporter.clone();
    let marker = worker.marker(2);
    assert_eq!(reporter.current_step(), 1);
    worker.marker(3);
    assert_eq!(reporter.current_step(), 2);
    assert!(marker.contains("2/3"));
    assert!(marker.contains("Write"));
    assert_eq!(reporter.steps()[1].label, "Write");
}

#[test]
fn operation_phase_every_plan_has_unique_nonempty_keys() {
    for kind in OperationPhaseKind::all() {
        let keys = kind.keys();
        assert!(!keys.is_empty());
        let unique = keys
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), keys.len(), "duplicate key in {kind:?}");
    }
}

#[test]
fn every_lang_file_is_a_registered_language() {
    let lang_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lang");
    let mut on_disk = std::fs::read_dir(&lang_dir)
        .expect("read lang dir")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "json").then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect::<Vec<_>>();
    on_disk.sort();
    let mut registered = LANGUAGES
        .iter()
        .map(|language| language.code().to_owned())
        .collect::<Vec<_>>();
    registered.sort();
    assert_eq!(
        on_disk, registered,
        "every lang/*.json needs a `Language` variant listed in `LANGUAGES`"
    );
    for &language in LANGUAGES {
        assert_eq!(Language::from_code(language.code()), Some(language));
    }
}

#[test]
fn all_locale_tables_load_through_translations() {
    for &language in LANGUAGES {
        assert!(!Translations::load(language).primary.is_empty());
    }
}

#[test]
fn about_license_messages_open_and_close_the_dialog() {
    let mut app = App::default();
    assert!(!app.about_licenses_open);

    let _ = app.update(Message::AboutLicensesOpen);
    assert!(app.about_licenses_open);

    let _ = app.update(Message::AboutLicensesClose);
    assert!(!app.about_licenses_open);
}

#[test]
fn an_empty_partition_scan_is_an_error_in_both_wizards() {
    // Both wizards must report it rather than sit on the loader step with an
    // empty table and no explanation.
    let app = App::default();
    assert!(app.parts_scan_outcome(None, true).is_some());
    assert!(app.parts_scan_outcome(None, false).is_none());
    // A worker failure outranks the empty check.
    assert_eq!(
        app.parts_scan_outcome(Some("boom".to_string()), true)
            .as_deref(),
        Some("boom")
    );
}

#[test]
fn a_refused_loader_pick_clears_the_path_and_blocks_next() {
    // Guards against a stale loader_path surviving a bad pick, or Next not
    // being gated on the error, letting a wizard advance on a refused loader.
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("xbl_s_devprg_ns.melf");
    std::fs::write(&good, b"loader").unwrap();
    let bad = dir.path().join("not-a-loader.txt");
    std::fs::write(&bad, b"nope").unwrap();

    let mut app = App::default();
    let _ = app.update_dump_phys(DumpPhysMsg::DumpPhysLoaderChosen(Some(
        good.to_string_lossy().to_string(),
    )));
    assert!(app.dump_phys.loader_path.is_some());
    assert!(app.dump_phys.loader_error.is_none());
    assert!(app.dump_phys.can_next());

    let _ = app.update_dump_phys(DumpPhysMsg::DumpPhysLoaderChosen(Some(
        bad.to_string_lossy().to_string(),
    )));
    assert!(app.dump_phys.loader_path.is_none());
    assert!(app.dump_phys.loader_error.is_some());
    assert!(!app.dump_phys.can_next());

    // Cancelling the picker leaves the step exactly as it was.
    let before = app.dump_phys.loader_error.clone();
    let _ = app.update_dump_phys(DumpPhysMsg::DumpPhysLoaderChosen(None));
    assert_eq!(app.dump_phys.loader_error, before);
}

#[test]
fn a_loader_pick_no_longer_overwrites_why_the_scan_failed() {
    // Guards against loader errors funneling into `scan_error`, which
    // would let re-picking a loader erase the scan failure.
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("not-a-loader.txt");
    std::fs::write(&bad, b"nope").unwrap();

    let mut app = App::default();
    app.dump_parts.scan_error = Some("scan blew up".to_string());
    let _ = app.update_dump_parts(DumpPartsMsg::DumpPartsLoaderChosen(Some(
        bad.to_string_lossy().to_string(),
    )));
    assert_eq!(app.dump_parts.scan_error.as_deref(), Some("scan blew up"));
    assert!(app.dump_parts.loader_error.is_some());
}

/// App with a model connected and that model's loader already remembered.
fn app_remembering(model: &str, loader: &std::path::Path) -> App {
    let mut app = App {
        device: DeviceSnapshot {
            model: model.to_string(),
            ..Default::default()
        },
        ..App::default()
    };
    app.remembered_edl_loaders.insert(
        model.to_ascii_uppercase(),
        loader.to_string_lossy().into_owned(),
    );
    app
}

fn write_melf(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("xbl_s_devprg_ns.melf");
    std::fs::write(&path, b"loader").unwrap();
    path
}

#[test]
fn a_loader_is_remembered_against_the_model_the_operation_started_on() {
    let dir = tempfile::tempdir().unwrap();
    let melf = write_melf(dir.path());

    let mut app = App {
        device: DeviceSnapshot {
            model: "TB320FC".to_string(),
            ..Default::default()
        },
        ..App::default()
    };
    // Arming captures the model now, because the device is about to
    // re-enumerate into EDL and may stop reporting one.
    app.arm_loader_memory();
    app.device.model.clear();
    app.remember_uploaded_loader(&melf);

    assert_eq!(
        app.remembered_edl_loaders
            .get("TB320FC")
            .map(String::as_str),
        Some(melf.to_string_lossy().as_ref())
    );
    assert!(!app.remembered_edl_loaders.contains_key("TB323FU"));
}

#[test]
fn nothing_is_remembered_without_a_model_or_with_the_switch_off() {
    let dir = tempfile::tempdir().unwrap();
    let melf = write_melf(dir.path());

    // No model identified → nothing to remember it against.
    let mut anonymous = App::default();
    anonymous.arm_loader_memory();
    anonymous.remember_uploaded_loader(&melf);
    assert!(anonymous.remembered_edl_loaders.is_empty());

    let mut off = App {
        device: DeviceSnapshot {
            model: "TB320FC".to_string(),
            ..Default::default()
        },
        remember_edl_loader: false,
        ..App::default()
    };
    off.arm_loader_memory();
    off.remember_uploaded_loader(&melf);
    assert!(off.remembered_edl_loaders.is_empty());
}

#[test]
fn a_remembered_loader_is_only_offered_to_its_own_model() {
    let dir = tempfile::tempdir().unwrap();
    let melf = write_melf(dir.path());
    let want = melf.to_string_lossy().into_owned();

    // Case differences in the polled model must not lose the entry.
    let mut app = app_remembering("TB320FC", &melf);
    app.device.model = "tb320fc".to_string();
    assert_eq!(
        app.remembered_loader_for_model().as_deref(),
        Some(want.as_str())
    );

    // A different model on the same App gets the picker, not this loader.
    app.device.model = "TB328FU".to_string();
    assert!(app.remembered_loader_for_model().is_none());

    // No model identified at all → nothing to look up.
    app.device.model.clear();
    assert!(app.remembered_loader_for_model().is_none());

    // Switched off → still remembered, but not used.
    app.device.model = "TB320FC".to_string();
    app.remember_edl_loader = false;
    assert!(app.remembered_loader_for_model().is_none());
}

#[test]
fn a_remembered_loader_that_went_stale_falls_back_to_the_picker() {
    let dir = tempfile::tempdir().unwrap();
    let melf = write_melf(dir.path());
    let mut app = app_remembering("TB320FC", &melf);
    assert!(app.remembered_loader_for_model().is_some());

    // Moved or deleted since it was learned.
    std::fs::remove_file(&melf).unwrap();
    assert!(app.remembered_loader_for_model().is_none());

    // Present again, but a manifest short a payload: the same pick-time
    // gate applies, so the step is shown rather than skipped onto a loader
    // that cannot load.
    let manifest_dir = tempfile::tempdir().unwrap();
    let manifest = manifest_pack(manifest_dir.path(), &["prog_firehose_ddr.elf"]);
    app.device.model = "TB323FU".to_string();
    app.remembered_edl_loaders
        .insert("TB323FU".to_string(), manifest);
    assert!(app.remembered_loader_for_model().is_none());
}

#[test]
fn a_remembered_loader_skips_the_step_and_back_reopens_it() {
    let dir = tempfile::tempdir().unwrap();
    let melf = write_melf(dir.path());
    let remembered = melf.to_string_lossy().into_owned();

    let mut app = app_remembering("TB320FC", &melf);
    app.unroot.unroot_type = Some(UnrootType::MagiskLkm);
    let _ = app.update(Message::Unroot(UnrootMsg::UnrootNext));

    // Method (0) → loader filled in → folder (2), skipping the loader step.
    assert_eq!(app.unroot.step, 2);
    assert_eq!(app.unroot.loader_path.as_deref(), Some(remembered.as_str()));

    // Back lands on the skipped step so the user can still change it.
    let _ = app.update(Message::Unroot(UnrootMsg::UnrootBack));
    assert_eq!(app.unroot.step, 1);
    assert_eq!(UNROOT_STEPS[1], "edl_loader_label");

    // A different loader chosen there sticks: Next must not re-skip and
    // re-apply the remembered one on the way forward.
    let other = dir.path().join("other.melf");
    std::fs::write(&other, b"loader").unwrap();
    let _ = app.update(Message::Unroot(UnrootMsg::UnrootLoaderChosen(Some(
        other.to_string_lossy().into_owned(),
    ))));
    let _ = app.update(Message::Unroot(UnrootMsg::UnrootNext));
    assert_eq!(
        app.unroot.loader_path.as_deref(),
        Some(other.to_string_lossy().as_ref())
    );
}

#[test]
fn without_a_remembered_loader_the_step_is_shown() {
    let mut app = App {
        device: DeviceSnapshot {
            model: "TB320FC".to_string(),
            ..Default::default()
        },
        ..App::default()
    };
    app.unroot.unroot_type = Some(UnrootType::MagiskLkm);
    let _ = app.update(Message::Unroot(UnrootMsg::UnrootNext));
    assert_eq!(app.unroot.step, 1);
    assert!(app.unroot.loader_path.is_none());
    assert!(!app.unroot.can_next());
}

#[test]
fn the_remember_switch_keeps_what_was_already_learned() {
    let dir = tempfile::tempdir().unwrap();
    let melf = write_melf(dir.path());
    let mut app = app_remembering("TB320FC", &melf);

    let _ = app.update(Message::Settings(SettingsMsg::SetRememberEdlLoader(false)));
    assert!(!app.remember_edl_loader);
    assert!(app.remembered_loader_for_model().is_none());
    // Turning it off must not throw the memory away — switching back on
    // restores it instead of making the user relearn it per device.
    assert!(!app.remembered_edl_loaders.is_empty());

    let _ = app.update(Message::Settings(SettingsMsg::SetRememberEdlLoader(true)));
    assert!(app.remembered_loader_for_model().is_some());
}

/// Manifest + the subset of its payloads that should exist on disk.
fn manifest_pack(dir: &std::path::Path, present: &[&str]) -> String {
    std::fs::write(
        dir.join(ltbox_core::sahara_xml::MANIFEST_FILENAME),
        br#"<sahara_config><images>
               <image image_id="13" image_path="prog_firehose_ddr.elf"/>
               <image image_id="21" image_path="xbl_sc.elf"/>
            </images></sahara_config>"#,
    )
    .unwrap();
    for name in present {
        std::fs::write(dir.join(name), b"img").unwrap();
    }
    dir.join(ltbox_core::sahara_xml::MANIFEST_FILENAME)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn a_manifest_missing_its_images_cannot_leave_the_loader_step() {
    // Checked at pick time so an incomplete extracted pack can't sail
    // through the wizard and fail only after the device is in EDL.
    let dir = tempfile::tempdir().unwrap();
    let manifest = manifest_pack(dir.path(), &["prog_firehose_ddr.elf"]);

    let mut app = App::default();
    let _ = app.update_dump_phys(DumpPhysMsg::DumpPhysLoaderChosen(Some(manifest.clone())));
    assert!(app.dump_phys.loader_path.is_none());
    let rejected = app.dump_phys.loader_error.clone().expect("rejected");
    assert!(rejected.contains("xbl_sc.elf"), "{rejected}");
    assert!(!app.dump_phys.can_next());

    // Drop the missing payload in and the same pick is accepted.
    std::fs::write(dir.path().join("xbl_sc.elf"), b"img").unwrap();
    let _ = app.update_dump_phys(DumpPhysMsg::DumpPhysLoaderChosen(Some(manifest.clone())));
    assert_eq!(
        app.dump_phys.loader_path.as_deref(),
        Some(manifest.as_str())
    );
    assert!(app.dump_phys.loader_error.is_none());
    assert!(app.dump_phys.can_next());
}

#[test]
fn a_firmware_folder_with_an_incomplete_manifest_demands_its_own_loader() {
    // The flash wizard takes the loader from the firmware folder; when
    // that loader is an incomplete manifest the folder step has to ask
    // for one rather than treat the folder as self-sufficient.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("rawprogram0.xml"), b"<data/>").unwrap();
    manifest_pack(dir.path(), &["prog_firehose_ddr.elf"]);

    let mut app = App::default();
    app.set_flash_firmware_folder(dir.path().to_string_lossy().into_owned());
    assert!(app.flash.loader_required);
    assert!(app.flash.loader_error.is_some());
    assert!(!app.flash.can_next());

    std::fs::write(dir.path().join("xbl_sc.elf"), b"img").unwrap();
    app.set_flash_firmware_folder(dir.path().to_string_lossy().into_owned());
    assert!(!app.flash.loader_required);
    assert!(app.flash.loader_error.is_none());
}

#[test]
fn root_and_unroot_loader_steps_upgrade_a_tb323fu_melf_to_the_manifest() {
    // Guards against Root/Unroot assigning the raw picked path instead of
    // going through `resolve_loader_input`, which would let a manifest-route
    // model's `.melf` reach Sahara without its manifest.
    let dir = tempfile::tempdir().unwrap();
    let melf = dir.path().join("xbl_s_devprg_ns.melf");
    std::fs::write(&melf, b"loader").unwrap();
    // A real manifest with its one payload beside it: the upgrade is only
    // allowed to land on a loader that would actually load.
    let manifest = dir.path().join(ltbox_core::sahara_xml::MANIFEST_FILENAME);
    std::fs::write(
        &manifest,
        br#"<sahara_config><images>
               <image image_id="13" image_path="prog_firehose_ddr.elf"/>
            </images></sahara_config>"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("prog_firehose_ddr.elf"), b"elf").unwrap();
    let picked = melf.to_string_lossy().to_string();
    let want = manifest.to_string_lossy().to_string();

    let mut root_app = App {
        device: DeviceSnapshot {
            model: "TB323FU".to_string(),
            ..Default::default()
        },
        ..App::default()
    };
    let _ = root_app.update(Message::Root(RootMsg::RootLoaderChosen(Some(
        picked.clone(),
    ))));
    assert_eq!(root_app.root.folder_path.as_deref(), Some(want.as_str()));
    assert!(root_app.error_msg.is_none());

    let mut unroot_app = App {
        device: DeviceSnapshot {
            model: "TB323FU".to_string(),
            ..Default::default()
        },
        ..App::default()
    };
    let _ = unroot_app.update(Message::Unroot(UnrootMsg::UnrootLoaderChosen(Some(
        picked.clone(),
    ))));
    assert_eq!(
        unroot_app.unroot.loader_path.as_deref(),
        Some(want.as_str())
    );

    // A model without `requires_sahara_manifest` keeps the .melf it was given.
    let mut other = App {
        device: DeviceSnapshot {
            model: "TB320FC".to_string(),
            ..Default::default()
        },
        ..App::default()
    };
    let _ = other.update(Message::Root(RootMsg::RootLoaderChosen(Some(
        picked.clone(),
    ))));
    assert_eq!(other.root.folder_path.as_deref(), Some(picked.as_str()));

    // Both steps still record the pick in the shared file recents.
    assert!(
        root_app
            .recent_paths
            .recent(pickers::PickerKind::File.storage_key())
            .contains(&picked)
    );
}

#[test]
fn fastbootd_is_labelled_apart_from_the_bootloader() {
    let mut app = App {
        device: DeviceSnapshot {
            connection: ConnectionStatus::Fastboot,
            ..Default::default()
        },
        ..App::default()
    };
    assert_eq!(app.connection_label_key(), "conn_fastboot");

    app.device.fastboot_userspace = true;
    assert_eq!(app.connection_label_key(), "conn_fastbootd");

    // The flag only ever qualifies a Fastboot connection.
    app.device.connection = ConnectionStatus::Adb;
    assert_eq!(app.connection_label_key(), "conn_adb");
}

#[test]
fn fastbootd_is_reachable_from_every_transport_that_can_ask_for_it() {
    // ADB routes it through the `reboot:` service, so it works even in
    // sideload, where there is no shell. The bootloader sends
    // `reboot-fastboot`. EDL resets only to system or back to EDL.
    assert!(RebootTarget::Fastbootd.available_from(ConnectionStatus::Adb));
    assert!(RebootTarget::Fastbootd.available_from(ConnectionStatus::Fastboot));
    assert!(RebootTarget::Fastbootd.available_from(ConnectionStatus::AdbSideload));
    assert!(!RebootTarget::Fastbootd.available_from(ConnectionStatus::Edl));
    assert!(!RebootTarget::Fastbootd.available_from(ConnectionStatus::AdbUnauthorized));
}

#[test]
fn reboot_targets_recognize_the_current_transport_state() {
    assert!(RebootTarget::System.is_current_from(ConnectionStatus::Adb, false));
    assert!(RebootTarget::Recovery.is_current_from(ConnectionStatus::AdbRecovery, false));
    assert!(RebootTarget::Edl.is_current_from(ConnectionStatus::Edl, false));
    assert!(RebootTarget::Bootloader.is_current_from(ConnectionStatus::Fastboot, false));
    assert!(RebootTarget::Fastbootd.is_current_from(ConnectionStatus::Fastboot, true));
    assert!(!RebootTarget::Fastbootd.is_current_from(ConnectionStatus::Fastboot, false));
}

#[test]
fn closing_the_edl_wait_dialog_keeps_the_reboot_operation_running() {
    let mut app = App::default();
    app.device.connection = ConnectionStatus::Adb;

    drop(app.update(Message::Reboot(RebootMsg::RebootTo(RebootTarget::Edl))));
    let operation_id = app.operation.id();
    assert!(app.operation.is_running());
    assert_eq!(app.operation.view(), Some(View::Reboot));
    assert!(app.reboot_wait_transition);
    assert!(app.reboot_wait_dialog_open);
    assert!(!app.should_show_busy_progress_dialog());

    drop(app.update(Message::RebootWaitDismiss));

    assert!(app.operation.is_running());
    assert_eq!(app.operation.id(), operation_id);
    assert!(app.reboot_wait_transition);
    assert!(!app.reboot_wait_dialog_open);
    assert!(!app.should_show_busy_progress_dialog());
}

#[test]
fn edl_wait_state_clears_when_the_reboot_operation_completes() {
    let mut app = App::default();
    app.device.connection = ConnectionStatus::Adb;
    drop(app.update(Message::Reboot(RebootMsg::RebootTo(RebootTarget::Edl))));
    let operation_id = app.operation.id().expect("reboot operation id");

    drop(app.update(Message::OperationEvent(
        operation_id,
        Box::new(Message::Reboot(RebootMsg::RebootDone(Vec::new()))),
    )));

    assert!(!app.operation.is_running());
    assert!(!app.reboot_wait_transition);
    assert!(!app.reboot_wait_dialog_open);
}

#[test]
fn dual_usb_guide_auto_opens_on_first_eligible_poll() {
    let mut app = App {
        startup_disclaimer_open: false,
        dual_usb_advisory_dismissed: Vec::new(),
        dual_usb_advisory_closed: Vec::new(),
        ..App::default()
    };
    assert!(!app.dual_usb_help_open);
    assert!(app.dual_usb_help_model.is_empty());

    let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
    assert!(app.dual_usb_help_open);
    assert_eq!(app.dual_usb_help_model, "TB323FU");
    assert_eq!(app.dual_usb_help_name, "Legion Y700");
}

#[test]
fn dual_usb_guide_waits_for_startup_disclaimer_to_close() {
    let mut app = App {
        startup_disclaimer_open: true,
        startup_disclaimer_checked: false,
        dual_usb_advisory_dismissed: Vec::new(),
        dual_usb_advisory_closed: Vec::new(),
        ..App::default()
    };

    let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
    assert!(!app.dual_usb_help_open);
    assert!(app.dual_usb_help_model.is_empty());

    let _ = app.update(Message::StartupDisclaimerToggled(true));
    let _ = app.update(Message::StartupDisclaimerConfirm);
    assert!(!app.startup_disclaimer_open);
    assert!(app.dual_usb_help_open);
    assert_eq!(app.dual_usb_help_model, "TB323FU");
}

#[test]
fn dual_usb_guide_stays_open_across_unplug_and_same_model_replug() {
    let mut app = App {
        startup_disclaimer_open: false,
        dual_usb_advisory_dismissed: Vec::new(),
        dual_usb_advisory_closed: Vec::new(),
        ..App::default()
    };

    let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
    assert!(app.dual_usb_help_open);
    assert_eq!(app.dual_usb_help_model, "TB323FU");

    let _ = app.update(Message::DevicePolled(DevicePollResult::default()));
    assert!(app.dual_usb_help_open);
    assert_eq!(app.dual_usb_help_model, "TB323FU");
    assert_eq!(app.dual_usb_help_name, "Legion Y700");

    let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
    assert!(app.dual_usb_help_open);
    assert_eq!(app.dual_usb_help_model, "TB323FU");
}

#[test]
fn dual_usb_guide_does_not_reopen_after_session_close_and_replug() {
    let mut app = App {
        startup_disclaimer_open: false,
        dual_usb_advisory_dismissed: Vec::new(),
        dual_usb_advisory_closed: Vec::new(),
        ..App::default()
    };

    let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
    let _ = app.update(Message::CloseDualUsbAdvisory("TB323FU".to_string()));

    assert!(!app.dual_usb_help_open);
    assert_eq!(app.dual_usb_help_model, "TB323FU");
    assert_eq!(app.dual_usb_advisory_closed, ["TB323FU"]);

    let _ = app.update(Message::DevicePolled(DevicePollResult::default()));
    let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
    assert!(!app.dual_usb_help_open);
    assert_eq!(app.dual_usb_help_model, "TB323FU");
}

#[test]
fn dual_usb_dont_show_again_roundtrips_and_suppresses_a_fresh_app() {
    let mut app = App {
        startup_disclaimer_open: false,
        dual_usb_advisory_dismissed: Vec::new(),
        dual_usb_advisory_closed: Vec::new(),
        ..App::default()
    };
    let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
    let _ = app.update(Message::DismissDualUsbAdvisory("TB323FU".to_string()));
    assert!(!app.dual_usb_help_open);
    assert_eq!(app.dual_usb_help_model, "TB323FU");

    let saved = settings_store::PersistedSettings {
        dual_usb_advisory_dismissed_models: app.dual_usb_advisory_dismissed.clone(),
        ..settings_store::PersistedSettings::default()
    };
    let json = serde_json::to_string(&saved).unwrap();
    let restored: settings_store::PersistedSettings = serde_json::from_str(&json).unwrap();
    let mut fresh_app = App {
        startup_disclaimer_open: false,
        dual_usb_advisory_dismissed: restored.dual_usb_advisory_dismissed_models,
        dual_usb_advisory_closed: Vec::new(),
        ..App::default()
    };

    let _ = fresh_app.update(Message::DevicePolled(device_poll("TB323FU")));
    assert!(!fresh_app.dual_usb_help_open);
    assert!(fresh_app.dual_usb_help_model.is_empty());
    assert_eq!(fresh_app.dual_usb_advisory_model(), None);
}

#[test]
fn second_dual_usb_model_still_auto_opens_after_first_model_is_closed() {
    let mut app = App {
        startup_disclaimer_open: false,
        dual_usb_advisory_dismissed: Vec::new(),
        dual_usb_advisory_closed: Vec::new(),
        ..App::default()
    };

    let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
    let _ = app.update(Message::CloseDualUsbAdvisory("TB323FU".to_string()));
    let _ = app.update(Message::DevicePolled(device_poll("TB322FC")));

    assert!(app.dual_usb_help_open);
    assert_eq!(app.dual_usb_help_model, "TB322FC");
    assert_eq!(app.dual_usb_advisory_model(), Some("TB322FC"));
}

#[test]
fn driver_restart_recommendation_tracks_success_and_close() {
    let mut app = App::default();
    assert!(!app.driver_restart_recommended);

    let _ = app.update(Message::InstallDriversDone(Ok(Vec::new())));
    assert!(app.driver_restart_recommended);

    let _ = app.update(Message::CloseDriverRestartRecommended);
    assert!(!app.driver_restart_recommended);
}

#[test]
fn sidebar_specific_label_keys_use_trimmed_variants() {
    assert_eq!(View::Flash.sidebar_label_key(), "nav_flash_sidebar");
    assert_eq!(View::Flash.label_key(), "nav_flash");
    assert_eq!(View::KonaBess.sidebar_label_key(), "nav_konabess_sidebar");
    assert_eq!(View::KonaBess.label_key(), "nav_konabess");
    assert_eq!(
        View::Dashboard.sidebar_label_key(),
        View::Dashboard.label_key()
    );
}

#[test]
fn sidebar_animation_is_hover_driven_only_in_compact_layout() {
    let mut app = App::default();
    app.window_size.0 = MIN_WINDOW_WIDTH;
    assert_eq!(app.window_size_class(), WindowSizeClass::Compact);
    assert_eq!(app.sidebar_anim_target(), 0.0);

    let _ = app.update(Message::SidebarHoverEnter);
    assert!(app.sidebar_expanded);
    assert_eq!(app.sidebar_anim_target(), 1.0);

    app.window_size.0 = 1320.0;
    app.sidebar_expanded = false;
    let _ = app.update(Message::SidebarHoverEnter);
    assert_eq!(app.window_size_class(), WindowSizeClass::Expanded);
    assert!(!app.sidebar_expanded);
    assert_eq!(app.sidebar_anim_target(), 0.0);
    assert_eq!(app.sidebar_visual_progress(), 1.0);
}

#[test]
fn konabess_is_in_main_navigation_directly_after_unroot() {
    let unroot = NAV_MAIN
        .iter()
        .position(|view| *view == View::Unroot)
        .expect("Unroot is in main navigation");
    assert_eq!(NAV_MAIN.get(unroot + 1), Some(&View::KonaBess));
    assert_eq!(NAV_MAIN.get(unroot + 2), Some(&View::Reboot));
    assert!(!NAV_TOOLS.contains(&View::KonaBess));
}

#[test]
fn sidebar_motion_settles_after_reversing_direction() {
    let mut app = App::default();
    app.window_size.0 = 900.0;
    let _ = app.update(Message::SidebarHoverEnter);
    for _ in 0..4 {
        let _ = app.update(Message::SidebarAnimTick);
    }
    let _ = app.update(Message::SidebarHoverExit);
    for _ in 0..300 {
        let _ = app.update(Message::SidebarAnimTick);
        assert!((0.0..=1.0).contains(&app.sidebar_label_alpha));
    }
    assert_eq!(app.sidebar_anim, 0.0);
    assert_eq!(app.sidebar_velocity, 0.0);
    assert_eq!(app.sidebar_label_alpha, 0.0);
    assert_eq!(app.sidebar_label_velocity, 0.0);
}

#[test]
fn unknown_key_falls_back_to_itself() {
    let t = Translations::load(Language::En);
    assert_eq!(t.t("__no_such_key__"), "__no_such_key__");
}

#[test]
fn non_empty_prop_treats_blank_as_absent() {
    assert_eq!(non_empty_prop(""), None);
    assert_eq!(non_empty_prop("   \n\t"), None);
    assert_eq!(
        non_empty_prop("  Tab Plus 14  \n"),
        Some("Tab Plus 14".to_string())
    );
}

#[test]
fn select_device_name_falls_back_through_lgsi_props() {
    use std::collections::HashMap;
    let pick = |map: HashMap<&'static str, &'static str>| {
        select_device_name(|p| map.get(p).copied().unwrap_or("").to_string())
    };

    // Primary populated wins.
    assert_eq!(
        pick(HashMap::from([(
            "ro.vendor.config.lgsi.en.market_name",
            "Tab Plus"
        )])),
        "Tab Plus"
    );
    // Primary whitespace-only -> vendor LGSI market name.
    assert_eq!(
        pick(HashMap::from([
            ("ro.vendor.config.lgsi.en.market_name", "   "),
            ("ro.vendor.config.lgsi.market_name", "Tab Vendor"),
        ])),
        "Tab Vendor"
    );
    // -> system LGSI market name.
    assert_eq!(
        pick(HashMap::from([(
            "ro.config.lgsi.market_name",
            "Tab System"
        )])),
        "Tab System"
    );
    // -> legacy kirby_en final fallback (preserved).
    assert_eq!(
        pick(HashMap::from([("ro.vendor.config.lgsi.kirby_en", "Kirby")])),
        "Kirby"
    );
    // Nothing populated -> empty string.
    assert_eq!(pick(HashMap::new()), "");
}

#[test]
fn efisp_asset_suffix_picks_prc_or_row() {
    assert_eq!(efisp_asset_suffix(true, false), "_prc.efi");
    assert_eq!(efisp_asset_suffix(false, false), "_row.efi");
    // Anti-rollback downgrade requests the `_arb` GBL (testkey root).
    assert_eq!(efisp_asset_suffix(true, true), "_prc_arb.efi");
    assert_eq!(efisp_asset_suffix(false, true), "_row_arb.efi");
}

#[test]
fn efisp_is_empty_only_for_all_zero() {
    assert!(!efisp_is_empty(&[]));
    assert!(efisp_is_empty(&[0u8; 4096]));
    assert!(!efisp_is_empty(&[0, 0, 1, 0]));
    let mut buf = vec![0u8; 1024];
    buf[1000] = 0xEF;
    assert!(!efisp_is_empty(&buf));
}

#[test]
fn advanced_in_progress_gates_partition_table_on_edl() {
    let row = || FlashPartRow {
        lun: 4,
        label: "boot_a".into(),
        start_sector: 0,
        num_sectors: 0,
        size_bytes: 0,
        file_path: None,
        state: FlashRowState::Skip,
    };
    let mut app = App {
        device: DeviceSnapshot {
            connection: ConnectionStatus::Edl,
            ..Default::default()
        },
        advanced_wizard_open: AdvancedWizardOpen::FlashParts,
        ..App::default()
    };
    // No scanned rows yet → not preserve-worthy.
    assert!(!app.advanced_in_progress());
    // GPT table loaded + still in EDL → preserve.
    app.flash_parts.rows = vec![row()];
    assert!(app.advanced_in_progress());
    // Device left EDL → table is stale → reset.
    app.device.connection = ConnectionStatus::None;
    assert!(!app.advanced_in_progress());

    // Physical confirm screen preserves; DumpPhys (no confirm) + the grid
    // do not.
    let mut app = App {
        advanced_wizard_open: AdvancedWizardOpen::FlashPhys,
        ..App::default()
    };
    app.flash_phys.step = FLASH_PHYS_STEPS.len() - 2; // Confirm
    assert!(app.advanced_in_progress());
    app.flash_phys.step = 1; // Select
    assert!(!app.advanced_in_progress());
    app.advanced_wizard_open = AdvancedWizardOpen::DumpPhys;
    assert!(!app.advanced_in_progress());
    app.advanced_wizard_open = AdvancedWizardOpen::None;
    assert!(!app.advanced_in_progress());

    // Simple Flash survives a sidebar bounce on its confirm step (folder
    // picked) and on its exec/result step (>=2); the intro step (0) resets.
    let mut app = App {
        advanced_wizard_open: AdvancedWizardOpen::SimpleFlash,
        ..App::default()
    };
    app.simple_flash.step = 1; // Confirm
    assert!(app.advanced_in_progress());
    app.simple_flash.step = 2; // Exec / result
    assert!(app.advanced_in_progress());
    app.simple_flash.step = 0; // Intro
    assert!(!app.advanced_in_progress());
}

#[test]
fn logs_cannot_change_operation_progress() {
    let mut app = App::default();
    let reporter = app.begin_phased_op(View::Root, OperationPhaseKind::Root);
    let _ = reporter.marker(3);
    app.log_push("[Download] file: 45% (12.3 MB / 45.6 MB)");
    app.log_push("[old worker] Phase 7/8");
    assert_eq!(app.operation.current_step(), 2);
    app.fail_op();
    let next = app.begin_phased_op(View::Root, OperationPhaseKind::Root);
    let _ = reporter.marker(8);
    assert_eq!(app.operation.current_step(), 0);
    let _ = next.marker(2);
    assert_eq!(app.operation.current_step(), 1);
}

#[test]
fn clear_log_empties_history_and_rebuilds_the_editor() {
    let mut app = App {
        log_lines: vec!["first".into(), "second".into()],
        ..Default::default()
    };
    app.rebuild_log_editor();

    drop(app.update(Message::ClearLog));

    assert!(app.log_lines.is_empty());
    assert_eq!(app.log_editor.text(), "");
    assert!(!app.log_dirty);
}

#[test]
fn visible_log_tail_and_export_have_independent_retention() {
    use ltbox_core::live_sink::{Entry, Kind};
    let mut app = App::default();
    drop(app.update(Message::ClearLog));
    for i in 0..600 {
        app.log_push(format!("operation line {i}"));
    }
    app.record_log_entry(Entry::debug("GPT diagnostic".into()));
    for pct in [10, 20, 30] {
        app.record_log_entry(Entry {
            line: format!("transfer {pct}%"),
            kind: Kind::Progress {
                key: "test-transfer".into(),
            },
        });
    }
    app.rebuild_log_editor();
    let visible = app.log_editor.text();
    assert!(!visible.contains("GPT diagnostic"));
    assert!(!visible.contains("transfer 10%"));
    assert!(visible.contains("transfer 30%"));
    assert_eq!(app.log_lines.len(), LOG_MAX_LINES);
    let saved = app.log_text_for_save(LogSaveSource::Main);
    assert!(saved.starts_with("operation line 0\n"));
    assert!(saved.contains("[Debug] GPT diagnostic"));
    assert!(saved.contains("transfer 30%"));

    app.log_push("write failed: disconnected");
    app.fail_op();
    app.rebuild_log_editor();
    assert!(app.log_editor.text().contains("write failed: disconnected"));
    assert!(
        app.log_text_for_save(LogSaveSource::Main)
            .contains("transfer 30%")
    );

    drop(app.update(Message::ClearLog));
    assert!(app.log_text_for_save(LogSaveSource::Main).is_empty());
}

#[test]
fn every_operation_phase_label_exists_in_every_locale() {
    for &lang in LANGUAGES {
        let translations = Translations::load(lang);
        for kind in OperationPhaseKind::all() {
            for key in kind.keys() {
                assert_ne!(translations.t(key), *key, "{lang:?} missing {key}");
            }
        }
    }
}

#[test]
fn flash_wizard_next_back_round_trip() {
    let mut w = FlashWizard::default();
    assert_eq!(w.step, 0);
    // Can't advance without a region selected.
    assert!(!w.can_next());
    w.region_selection = Some(FlashRegionSelection::Manual(DeviceRegion::Prc));
    w.device_region = Some(DeviceRegion::Prc);
    assert!(w.can_next());
    w.next();
    assert_eq!(w.step, 1);
    w.back();
    assert_eq!(w.step, 0);
    // Reset wipes every field.
    w.next();
    w.reset();
    assert_eq!(w.step, 0);
    assert!(w.region_selection.is_none());
    assert!(w.device_region.is_none());
}

#[test]
fn flash_region_auto_lookup_is_user_initiated() {
    let mut app = App::default();

    drop(app.prepare_flash_region_on_entry());
    assert!(app.flash.region_selection.is_none());
    assert!(app.flash_serial_prompt.is_none());
    assert!(app.queries.region_pending.is_none());

    drop(app.update_flash(FlashMsg::FlashRegionAuto));
    assert_eq!(app.flash.region_selection, Some(FlashRegionSelection::Auto));
    assert!(app.flash_serial_prompt.is_none());

    drop(app.update_flash(FlashMsg::FlashNext));
    assert_eq!(app.flash.step, 0);
    assert!(app.flash_serial_prompt.is_some());

    drop(app.update_flash(FlashMsg::FlashSerialPromptSkip));
    assert!(app.flash_serial_prompt.is_none());
    assert!(app.flash.region_auto_unknown);
}

#[test]
fn flash_confirm_requires_loader_when_folder_has_none() {
    let mut w = FlashWizard {
        step: 4,
        firmware_folder: Some("firmware".to_string()),
        firmware_identity: Some(FirmwareIdentity {
            efisp_load: ltbox_patch::efisp_load::EfispLoad::Undetermined,
            key_class: ltbox_patch::key_map::KeyClass::Testkey,
            fingerprint: None,
            model_token: None,
        }),
        loader_required: true,
        ..Default::default()
    };
    assert!(!w.can_next());

    w.loader_override = Some("prog_firehose.elf".to_string());
    assert!(w.can_next());
}

#[test]
fn confirm_step_is_the_step_before_exec() {
    // Linear (trait default): confirm = step_count - 2, exec = -1.
    let mut f = FlashWizard::default();
    let confirm = f.step_count() - 2;
    f.step = 0;
    assert!(!f.is_on_confirm_step());
    f.step = confirm;
    assert!(f.is_on_confirm_step());
    assert!(!f.is_in_exec());
    f.step = f.step_count() - 1;
    assert!(!f.is_on_confirm_step());
    assert!(f.is_in_exec());

    // SysUpdate step count flexes with rescue mode; confirm still tracks
    // step_count - 2 on both the compact and the longer rescue flow.
    let mut s = SysUpdateWizard::default();
    s.step = s.step_count() - 2; // compact: confirm = step 1
    assert!(s.is_on_confirm_step());
    s.action = Some(SysUpdateAction::Rescue);
    s.step = s.step_count() - 2; // rescue: confirm = step 2
    assert!(s.is_on_confirm_step());
    s.step = 1; // rescue folder step — not confirm
    assert!(!s.is_on_confirm_step());

    // Root is non-linear: confirm = step 6, exec = step 7.
    let mut r = RootWizard {
        step: 6,
        ..Default::default()
    };
    assert!(r.is_on_confirm_step());
    r.step = 7;
    assert!(!r.is_on_confirm_step());
    assert!(r.is_in_exec());
    r.step = 0;
    assert!(!r.is_on_confirm_step());
}

#[test]
fn root_wizard_kernelsu_lkm_path() {
    let mut w = RootWizard {
        family: Some(Family::KernelSU),
        ..RootWizard::default()
    };
    w.next(); // 0 → 1 (Mode)
    assert_eq!(w.step, 1);
    w.mode = Some(RootMode::Lkm);
    w.next(); // 1 → 2 (Provider)
    assert_eq!(w.step, 2);
    w.provider = Some(Provider::KernelSU);
    w.next(); // 2 → 3 (Version)
    assert_eq!(w.step, 3);
    w.version = Some(VerChoice::Stable);
    w.next(); // Stable skips NightlySource, jumps to Confirm (5)
    assert_eq!(w.step, 5);
}

#[test]
fn root_wizard_kernelsu_lkm_requires_kernel_version_before_exec() {
    let mut w = RootWizard {
        family: Some(Family::KernelSU),
        mode: Some(RootMode::Lkm),
        provider: Some(Provider::KernelSU),
        version: Some(VerChoice::Stable),
        folder_path: Some("firmware".to_string()),
        step: 6,
        ..RootWizard::default()
    };

    assert!(w.needs_ksu_lkm_kernel_version());
    w.kernel_version = Some("6.1".to_string());
    assert!(!w.needs_ksu_lkm_kernel_version());
}

#[test]
fn root_wizard_magisk_skips_mode() {
    let mut w = RootWizard {
        family: Some(Family::Magisk),
        ..RootWizard::default()
    };
    w.next(); // 0 → 2 directly (Magisk has no modes)
    assert_eq!(w.step, 2);
}

#[test]
fn root_wizard_skroot_lite_skips_provider_version() {
    let mut w = RootWizard {
        family: Some(Family::Skroot),
        ..RootWizard::default()
    };
    w.next(); // 0 → 1 (Lite / Pro)
    assert_eq!(w.step, 1);
    assert!(!w.can_next());
    w.skroot_flavor = Some(SkrootFlavor::Pro);
    assert!(!w.can_next());
    w.skroot_flavor = Some(SkrootFlavor::Lite);
    assert!(w.can_next());
    w.next(); // 1 → 5 (Loader)
    assert_eq!(w.step, 5);
    assert_eq!(w.display_step(), 2);
    w.back(); // 5 → 1
    assert_eq!(w.step, 1);
}

#[test]
fn image_info_wizard_runs_after_multi_image_selection() {
    let mut w = AdvWizard::default();
    w.open(AdvAction::ImageInfo);

    assert_eq!(w.steps(), &["adv_step_source", "adv_step_info"]);
    assert!(!w.is_confirm_step());
    assert!(!w.can_next());

    w.file_paths = vec!["boot.img".into(), "vbmeta.img".into()];
    assert!(w.can_next());
    w.next();
    assert_eq!(w.step, w.exec_step());
}

#[test]
fn advanced_menu_taxonomy_matches_avb_image_reclass() {
    let section = |key: &str| {
        ADV_SECTIONS
            .iter()
            .find(|section| section.title_key == key)
            .expect("section exists")
            .items
    };

    assert_eq!(
        section("adv_section_region_patch"),
        &[AdvAction::RegionConvert, AdvAction::PatchDevinfo]
    );
    assert!(
        ADV_SECTIONS
            .iter()
            .all(|section| section.title_key != "adv_section_country_code")
    );
    assert_eq!(
        section("adv_section_rollback"),
        &[
            AdvAction::ImageInfo,
            AdvAction::DetectArb,
            AdvAction::PatchArb,
            AdvAction::RebuildVbmeta,
        ]
    );
    assert_eq!(
        section("adv_section_edl_ops"),
        &[
            AdvAction::ConvertXml,
            AdvAction::DumpPartitions,
            AdvAction::FlashPartitions,
            AdvAction::DumpPhysical,
            AdvAction::FlashPhysical,
            AdvAction::SimpleFlash,
        ]
    );
}

#[test]
fn country_popup_selection_uses_opening_flow_context() {
    let app = App {
        adv_needs_country: true,
        adv_wizard: AdvWizard {
            country: Some("KR".to_string()),
            ..AdvWizard::default()
        },
        wf_config: WorkflowConfig {
            country_action: CountryAction::Set("CN".to_string()),
            ..WorkflowConfig::default()
        },
        ..App::default()
    };
    assert_eq!(app.country_popup_selected_code(), Some("KR"));

    let app = App {
        adv_needs_country: false,
        adv_wizard: AdvWizard {
            country: Some("KR".to_string()),
            ..AdvWizard::default()
        },
        wf_config: WorkflowConfig {
            country_action: CountryAction::Set("CN".to_string()),
            ..WorkflowConfig::default()
        },
        ..App::default()
    };
    assert_eq!(app.country_popup_selected_code(), Some("CN"));
}

#[test]
fn country_popup_stages_a_row_until_footer_confirmation() {
    let mut app = App {
        country_popup_open: true,
        country_popup_draft: CountryAction::Set("CN".to_string()),
        wf_config: WorkflowConfig {
            country_action: CountryAction::Set("US".to_string()),
            ..WorkflowConfig::default()
        },
        ..App::default()
    };

    let _ = app.update(Message::SelectCountry("KR".to_string()));
    assert!(app.country_popup_open);
    assert_eq!(app.country_popup_draft.target(), Some("KR"));
    assert_eq!(app.wf_config.country_action.target(), Some("US"));

    let _ = app.update(Message::CountryPopupConfirm);
    assert!(!app.country_popup_open);
    assert_eq!(app.wf_config.country_action.target(), Some("KR"));
}

#[test]
fn flash_keep_data_preserves_confirm_country_override() {
    let mut app = App {
        wf_config: WorkflowConfig {
            wipe: true,
            country_action: CountryAction::Set("KR".to_string()),
            ..WorkflowConfig::default()
        },
        ..App::default()
    };

    let _ = app.update_flash(FlashMsg::FlashConfirmSetData(DataMode::Keep));

    assert!(!app.wf_config.wipe);
    assert_eq!(app.wf_config.country_action.target(), Some("KR"));
}

#[test]
fn flash_confirm_country_popup_dismiss_stays_on_confirm() {
    let mut app = App {
        flash: FlashWizard {
            step: 4,
            ..FlashWizard::default()
        },
        country_popup_open: true,
        wf_config: WorkflowConfig {
            country_action: CountryAction::Unset,
            ..WorkflowConfig::default()
        },
        ..App::default()
    };

    let _ = app.update(Message::DismissCountryPopup);

    assert!(!app.country_popup_open);
    assert_eq!(app.flash.step, 4);
}

#[test]
fn sysupdate_wizard_gate_requires_action() {
    let mut w = SysUpdateWizard::default();
    assert!(!w.can_next());
    w.action = Some(SysUpdateAction::Disable);
    assert!(w.can_next());
    w.next();
    assert_eq!(w.step, 1);
    w.next();
    w.next();
    // Caps at len - 1.
    assert_eq!(w.step, SYSUPDATE_STEPS_COMPACT.len() - 1);
}

#[test]
fn flash_parts_wizard_requires_selection() {
    let mut w = FlashPartsWizard::default();
    assert!(!w.can_next());
    w.loader_path = Some("/tmp/xbl.melf".to_string());
    // Step 0 only needs a loader picked.
    assert!(w.can_next());
    w.next();
    assert_eq!(w.step, 1);
    // Step 1: need at least one row with a resolvable action.
    w.rows.push(FlashPartRow {
        lun: 0,
        label: "boot_a".into(),
        start_sector: 0,
        num_sectors: 8192,
        size_bytes: 4 * 1024 * 1024,
        file_path: None,
        state: FlashRowState::Skip,
    });
    assert!(!w.can_next()); // Unchecked doesn't count
    w.rows[0].state = FlashRowState::Write;
    assert!(!w.can_next()); // Flash w/o file still invalid
    w.rows[0].file_path = Some("/tmp/boot.img".into());
    assert!(w.can_next());
    // Erase alone is enough — no file required.
    w.rows[0].state = FlashRowState::Erase;
    w.rows[0].file_path = None;
    assert!(w.can_next());
}

#[test]
fn partition_checkbox_cycles_without_picker_and_blocks_incomplete_writes() {
    let mut app = App::default();
    app.flash_parts.step = 1;
    app.flash_parts.rows.push(FlashPartRow {
        lun: 0,
        label: "userdata".into(),
        start_sector: 0,
        num_sectors: 1,
        size_bytes: 512,
        file_path: None,
        state: FlashRowState::Skip,
    });
    let task = app.update(Message::FlashParts(FlashPartsMsg::FlashPartsToggleRow(0)));
    assert_eq!(task.units(), 0);
    assert_eq!(app.flash_parts.rows[0].state, FlashRowState::Write);
    assert!(!app.flash_parts.can_next());
    let task = app.update(Message::FlashParts(FlashPartsMsg::FlashPartsToggleRow(0)));
    assert_eq!(task.units(), 0);
    assert_eq!(app.flash_parts.rows[0].state, FlashRowState::Erase);
    assert!(app.flash_parts.can_next());
    let mut incomplete = app.flash_parts.rows[0].clone();
    incomplete.state = FlashRowState::Write;
    app.flash_parts.rows.push(incomplete);
    for step in [1, 2] {
        app.flash_parts.step = step;
        assert!(!app.flash_parts.can_next());
    }
    assert_eq!(
        app.update(Message::FlashParts(FlashPartsMsg::FlashPartsExecStart))
            .units(),
        0
    );
    assert!(!app.operation.is_running());
}

#[test]
fn partition_confirmation_cancel_restores_only_transitional_entry_modes() {
    let loader = tempfile::Builder::new().suffix(".melf").tempfile().unwrap();
    for entry in [
        ConnectionStatus::Adb,
        ConnectionStatus::Fastboot,
        ConnectionStatus::Edl,
    ] {
        let mut app = App {
            current_view: View::Advanced,
            advanced_wizard_open: AdvancedWizardOpen::FlashParts,
            flash_parts: FlashPartsWizard {
                step: 2,
                loader_path: Some(loader.path().to_string_lossy().into_owned()),
                entry_connection: Some(entry),
                ..Default::default()
            },
            ..App::default()
        };
        // Dropping the task proves scheduling without touching any device.
        let _task = app.update(Message::StartOver);
        assert_eq!(app.advanced_wizard_open, AdvancedWizardOpen::None);
        assert_eq!(app.operation.is_running(), entry != ConnectionStatus::Edl);
        assert_eq!(app.flash_parts.entry_connection, None);
    }
}

#[test]
fn advanced_partition_tables_started_in_edl_keep_back_without_rebooting() {
    assert_eq!(
        partition_table_leading_action(Some(ConnectionStatus::Edl)),
        WizardLeadingAction::Back
    );

    let mut flash_app = App {
        device: DeviceSnapshot {
            connection: ConnectionStatus::Edl,
            ..Default::default()
        },
        advanced_wizard_open: AdvancedWizardOpen::FlashParts,
        flash_parts: FlashPartsWizard {
            step: 1,
            entry_connection: Some(ConnectionStatus::Edl),
            ..FlashPartsWizard::default()
        },
        ..App::default()
    };
    let _task = flash_app.update_flash_parts(FlashPartsMsg::FlashPartsBack);
    assert_eq!(flash_app.flash_parts.step, 0);
    assert!(!flash_app.operation.is_running());
    assert_eq!(
        flash_app.advanced_wizard_open,
        AdvancedWizardOpen::FlashParts
    );

    let mut dump_app = App {
        device: DeviceSnapshot {
            connection: ConnectionStatus::Edl,
            ..Default::default()
        },
        advanced_wizard_open: AdvancedWizardOpen::DumpParts,
        dump_parts: DumpPartsWizard {
            step: 1,
            entry_connection: Some(ConnectionStatus::Edl),
            ..DumpPartsWizard::default()
        },
        ..App::default()
    };
    let _task = dump_app.update_dump_parts(DumpPartsMsg::DumpPartsBack);
    assert_eq!(dump_app.dump_parts.step, 0);
    assert!(!dump_app.operation.is_running());
    assert_eq!(dump_app.advanced_wizard_open, AdvancedWizardOpen::DumpParts);
}

#[test]
fn advanced_partition_tables_started_elsewhere_cancel_and_schedule_system_reboot() {
    assert_eq!(
        partition_table_leading_action(Some(ConnectionStatus::Adb)),
        WizardLeadingAction::Cancel
    );
    assert_eq!(
        partition_table_leading_action(Some(ConnectionStatus::Fastboot)),
        WizardLeadingAction::Cancel
    );
    assert_eq!(
        partition_table_leading_action(Some(ConnectionStatus::None)),
        WizardLeadingAction::Cancel
    );
    assert_eq!(
        partition_table_leading_action(None),
        WizardLeadingAction::Back
    );

    let loader = tempfile::Builder::new()
        .suffix(".melf")
        .tempfile()
        .expect("temporary loader");
    let loader_path = loader.path().to_string_lossy().to_string();

    let mut flash_app = App {
        advanced_wizard_open: AdvancedWizardOpen::FlashParts,
        // The live state is EDL after the scan; only the captured entry
        // state can prove that LTBox changed it.
        device: DeviceSnapshot {
            connection: ConnectionStatus::Edl,
            ..Default::default()
        },
        flash_parts: FlashPartsWizard {
            step: 1,
            loader_path: Some(loader_path.clone()),
            entry_connection: Some(ConnectionStatus::Adb),
            ..FlashPartsWizard::default()
        },
        ..App::default()
    };
    let _task = flash_app.update_flash_parts(FlashPartsMsg::FlashPartsBack);
    assert!(flash_app.operation.is_running());
    assert_eq!(flash_app.operation.view(), Some(View::Reboot));
    assert_eq!(flash_app.advanced_wizard_open, AdvancedWizardOpen::None);
    assert_eq!(flash_app.flash_parts.entry_connection, None);

    let mut dump_app = App {
        device: DeviceSnapshot {
            connection: ConnectionStatus::Edl,
            ..Default::default()
        },
        advanced_wizard_open: AdvancedWizardOpen::DumpParts,
        dump_parts: DumpPartsWizard {
            step: 1,
            loader_path: Some(loader_path),
            entry_connection: Some(ConnectionStatus::Fastboot),
            ..DumpPartsWizard::default()
        },
        ..App::default()
    };
    let _task = dump_app.update_dump_parts(DumpPartsMsg::DumpPartsBack);
    assert!(dump_app.operation.is_running());
    assert_eq!(dump_app.operation.view(), Some(View::Reboot));
    assert_eq!(dump_app.advanced_wizard_open, AdvancedWizardOpen::None);
    assert_eq!(dump_app.dump_parts.entry_connection, None);
}

#[test]
fn flash_parts_erase_marker_keeps_checkbox_square_footprint() {
    assert_eq!(FLASH_PARTS_MARKER_CELL_WIDTH, 32.0);
    assert_eq!(FLASH_PARTS_MARKER_SIZE, 16.0);
    let dash_width = std::hint::black_box(FLASH_PARTS_ERASE_DASH_WIDTH);
    let marker_size = std::hint::black_box(FLASH_PARTS_MARKER_SIZE);
    assert!(dash_width < marker_size);
    assert!(marker_size < FLASH_PARTS_MARKER_CELL_WIDTH);
}

#[test]
fn busy_progress_dialog_shows_only_without_inline_log_surface() {
    let mut app = App {
        operation: OperationExecution::fixture(true, Some(View::Reboot), Vec::new(), 0, None),
        current_view: View::Reboot,
        ..App::default()
    };

    assert!(app.should_show_busy_progress_dialog());

    app.current_view = View::Dashboard;
    assert!(!app.should_show_busy_progress_dialog());

    app.current_view = View::Advanced;
    app.advanced_wizard_open = AdvancedWizardOpen::FlashParts;
    app.flash_parts.step = 0;
    assert!(app.should_show_busy_progress_dialog());

    app.flash_parts.step = 3;
    assert!(!app.should_show_busy_progress_dialog());

    app.advanced_wizard_open = AdvancedWizardOpen::DumpParts;
    app.dump_parts.step = 0;
    assert!(app.should_show_busy_progress_dialog());

    app.dump_parts.step = 2;
    assert!(!app.should_show_busy_progress_dialog());

    app.advanced_wizard_open = AdvancedWizardOpen::None;
    app.current_view = View::Flash;
    app.flash.step = FLASH_STEPS.len() - 1;
    assert!(!app.should_show_busy_progress_dialog());
}

#[test]
fn konabess_inspection_uses_busy_dialog_and_flash_uses_inline_exec_surface() {
    let mut app = App {
        operation: OperationExecution::fixture(true, Some(View::KonaBess), Vec::new(), 0, None),
        current_view: View::KonaBess,
        ..App::default()
    };

    app.konabess.step = 0;
    assert!(!app.current_view_has_inline_exec_surface());
    assert!(app.should_show_busy_progress_dialog());
    assert_eq!(
        app.busy_body_override().as_deref(),
        Some(app.t("busy_konabess_inspection"))
    );

    app.konabess.step = 3;
    assert!(app.current_view_has_inline_exec_surface());
    assert!(!app.should_show_busy_progress_dialog());
}

#[test]
fn wizard_step_state_tracks_completed_active_and_upcoming() {
    assert_eq!(wizard_step_state(0, 2), WizardStepState::Completed);
    assert_eq!(wizard_step_state(2, 2), WizardStepState::Active);
    assert_eq!(wizard_step_state(3, 2), WizardStepState::Upcoming);
}

#[test]
fn arb_answer_is_blank_until_the_model_is_known() {
    assert_eq!(arb_from_model(""), "");
    assert_eq!(arb_from_model("   "), "");
    assert_eq!(arb_from_model("TB322FC"), "arb_no");
    assert_eq!(arb_from_model("TB520FU"), "arb_yes");
}

/// Floors come only from a bootloader poll, so their presence is the
/// transport test; the model check keeps an exempt SKU from offering
/// a breakdown behind a cell that reads "No".
#[test]
fn rollback_detail_needs_both_floors_and_a_protected_model() {
    let floors = ltbox_patch::rollback::FastbootRollbackFloors {
        vbmeta_system_location: 2,
        vbmeta_system_index: 0x69D1_A600,
        boot_location: 3,
        boot_index: 0x69D1_A600,
    };

    let mut app = App {
        device: DeviceSnapshot {
            model: "TB520FU".into(),
            rollback_floors: Some(floors),
            ..Default::default()
        },
        ..App::default()
    };
    assert!(app.rollback_detail_available());

    // Exempt SKU — the cell reads "No", so it must not be clickable.
    app.device.model = "TB322FC".into();
    assert!(!app.rollback_detail_available());

    // Any non-bootloader transport leaves the floors unset.
    app.device.model = "TB520FU".into();
    app.device.rollback_floors = None;
    assert!(!app.rollback_detail_available());
}

/// The popup's cycle must return to where it started, and each form
/// must render the value the copy button will put on the clipboard.
#[test]
fn rollback_value_format_cycles_and_renders() {
    // Real TB520FU floor: `stored_rollback_index:3 = 69D1A600`.
    const IDX: u64 = 0x69D1_A600;

    let raw = RollbackValueFormat::Raw;
    assert_eq!(raw.render(IDX), "0x69D1A600");

    let unix = raw.next();
    assert_eq!(unix, RollbackValueFormat::Unix);
    assert_eq!(unix.render(IDX), "1775347200");

    let date = unix.next();
    assert_eq!(date, RollbackValueFormat::Date);
    assert_eq!(date.render(IDX), "2026-04-05");

    assert_eq!(date.next(), RollbackValueFormat::Raw);
}

#[test]
fn manual_rollback_format_round_trips_in_every_mode() {
    const IDX: u64 = 0x69D1_A600;
    let cases = [
        (RollbackValueFormat::Raw, format!("0x{IDX:X}")),
        (RollbackValueFormat::Unix, IDX.to_string()),
        (RollbackValueFormat::Date, "2026-04-05".to_string()),
    ];

    for (format, rendered) in cases {
        assert_eq!(format.render(IDX), rendered);
        assert_eq!(format.parse(&rendered), Ok(IDX));
    }
}

#[test]
fn manual_rollback_rejects_future_timestamps() {
    let app = App {
        rollback_value_format: RollbackValueFormat::Unix,
        ..App::default()
    };
    let now = current_unix_timestamp().expect("clock is after the epoch");
    let rejects_same_timestamp = (0..10).any(|_| {
        app.parse_manual_rollback(&now.to_string())
            == Err("rollback_manual_error_future".to_string())
    });
    assert!(
        rejects_same_timestamp,
        "same-second target must be rejected"
    );
    assert!(app.parse_manual_rollback("1775347199").is_ok());
}

#[test]
fn reopening_the_manual_editor_keeps_what_the_user_confirmed() {
    let mut app = App {
        manual_rollback_format: RollbackValueFormat::Unix,
        ..Default::default()
    };
    // Image defaults differ from what the user settled on.
    app.flash.firmware_rollback_indices = Some((Ok(1_500_000_000), Ok(1_500_000_000)));
    app.wf_config.manual_rollback_indices = Some(ManualRollbackIndices {
        boot: 1_700_000_000,
        vbmeta_system: 1_600_000_000,
    });

    let _ = app.open_manual_rollback_editor();
    let (boot, vbmeta) = app.manual_rollback_buffers.clone().expect("buffers");
    assert_eq!(
        boot, "1700000000",
        "reopening must not revert to the image value"
    );
    assert_eq!(vbmeta, "1600000000");
    // The hint under each field reads these, so reopening must leave them
    // as the image reported them rather than adopting what the user typed.
    assert_eq!(
        app.flash.firmware_rollback_indices,
        Some((Ok(1_500_000_000), Ok(1_500_000_000))),
        "image indices are the hint's source and are not the user's values"
    );
}

#[test]
fn manual_rollback_editor_defaults_to_unix_and_dashboard_to_raw() {
    let app = App::default();
    assert_eq!(app.manual_rollback_format, RollbackValueFormat::Unix);
    assert_eq!(app.rollback_value_format, RollbackValueFormat::Raw);
}

#[test]
fn manual_rollback_cycle_reexpresses_the_typed_values() {
    let mut app = App {
        manual_rollback_format: RollbackValueFormat::Unix,
        manual_rollback_buffers: Some(("1700000000".into(), "1600000000".into())),
        manual_rollback_values: (Some(1_700_000_000), Some(1_600_000_000)),
        ..Default::default()
    };

    let _ = app.update(Message::Flash(FlashMsg::FlashManualRollbackCycleFormat));
    let (boot, vbmeta) = app.manual_rollback_buffers.clone().expect("buffers");
    assert_eq!(app.manual_rollback_format, RollbackValueFormat::Date);
    assert_eq!(boot, RollbackValueFormat::Date.render(1_700_000_000));
    assert_eq!(vbmeta, RollbackValueFormat::Date.render(1_600_000_000));

    let _ = app.update(Message::Flash(FlashMsg::FlashManualRollbackCycleFormat));
    let (boot, _) = app.manual_rollback_buffers.clone().expect("buffers");
    assert_eq!(app.manual_rollback_format, RollbackValueFormat::Raw);
    assert_eq!(boot, RollbackValueFormat::Raw.render(1_700_000_000));
}

#[test]
fn manual_rollback_cycle_leaves_unparsable_text_alone() {
    let mut app = App {
        manual_rollback_format: RollbackValueFormat::Unix,
        manual_rollback_buffers: Some(("not-a-number".into(), "1600000000".into())),
        manual_rollback_values: (None, Some(1_600_000_000)),
        ..Default::default()
    };

    let _ = app.update(Message::Flash(FlashMsg::FlashManualRollbackCycleFormat));
    let (boot, vbmeta) = app.manual_rollback_buffers.clone().expect("buffers");
    assert_eq!(boot, "not-a-number");
    assert_eq!(vbmeta, RollbackValueFormat::Date.render(1_600_000_000));
}

#[test]
fn manual_rollback_requires_valid_confirm() {
    install_core_translator(Language::En);
    let mut app = App {
        flash: FlashWizard {
            step: 4,
            firmware_folder: Some("firmware".into()),
            firmware_rollback_indices: Some((Ok(100), Ok(200))),
            ..FlashWizard::default()
        },
        wf_config: WorkflowConfig {
            modify_rollback: RollbackSetting::Off,
            ..WorkflowConfig::default()
        },
        confirm_edit_field: Some(ConfirmField::Rollback),
        ..App::default()
    };
    app.manual_rollback_editor = Some(ManualRollbackEditor::Boot);
    app.manual_rollback_buffers = Some(("99".to_string(), "199".to_string()));

    // A future target is valid decimal but fails the time gate.
    let future = current_unix_timestamp().map(|now| now + 1).unwrap_or(0);
    app.manual_rollback_buffers = Some((future.to_string(), "199".to_string()));
    let _ = app.update_flash(FlashMsg::FlashManualRollbackConfirm);
    assert_eq!(app.wf_config.modify_rollback, RollbackSetting::Off);
    assert_eq!(app.wf_config.manual_rollback_indices, None);

    let _ = app.update(Message::Flash(FlashMsg::FlashConfirmSetRollback(
        RollbackSetting::Manual,
    )));
    assert_eq!(app.wf_config.modify_rollback, RollbackSetting::Off);
    assert!(app.manual_rollback_editor.is_some());
    app.manual_rollback_buffers = Some(("0x63".to_string(), "0x199".to_string()));
    app.rollback_value_format = RollbackValueFormat::Unix;
    let _ = app.update_flash(FlashMsg::FlashManualRollbackConfirm);
    assert_ne!(app.wf_config.modify_rollback, RollbackSetting::Manual);
    assert_eq!(app.wf_config.manual_rollback_indices, None);

    app.rollback_value_format = RollbackValueFormat::Unix;
    app.manual_rollback_buffers = Some(("99".to_string(), "199".to_string()));
    let _ = app.update_flash(FlashMsg::FlashManualRollbackConfirm);
    assert_eq!(app.wf_config.modify_rollback, RollbackSetting::Manual);
    assert_eq!(
        app.wf_config.manual_rollback_indices,
        Some(ManualRollbackIndices {
            boot: 99,
            vbmeta_system: 199
        })
    );
    assert_eq!(app.confirm_edit_field, None);
    assert_eq!(app.manual_rollback_editor, None);
}

#[test]
fn concise_error_summary_collapses_lines_and_truncates_unicode() {
    assert_eq!(
        concise_error_summary("\n  loader   handshake failed  \nfull detail", 80),
        "loader handshake failed"
    );
    assert_eq!(concise_error_summary("가나다라마바사", 5), "가나다라…");
}

#[test]
fn error_summary_keeps_diagnosis_without_followup_or_nested_details() {
    for (full, expected) in [
        (
            "30초 이내에 활성 슬롯을 감지하지 못했습니다. Android나 복구 모드에서 다시 시도하세요.",
            "30초 이내에 활성 슬롯을 감지하지 못했습니다.",
        ),
        (
            "Unable to detect the active slot. Connect using ADB and retry.",
            "Unable to detect the active slot.",
        ),
        (
            "スロットを検出できませんでした。再試行してください。",
            "スロットを検出できませんでした。",
        ),
        ("无法检测当前槽位。请重试。", "无法检测当前槽位。"),
        (
            "Не удалось определить слот. Повторите попытку.",
            "Не удалось определить слот.",
        ),
        (
            "EDL 세션 열기 실패: USB error\nstack detail",
            "EDL 세션 열기 실패",
        ),
        (
            "boot.img version 1.2.3 failed. Retry.",
            "boot.img version 1.2.3 failed.",
        ),
        (
            "C:\\firmware\\boot.img not found",
            "C:\\firmware\\boot.img not found",
        ),
    ] {
        assert_eq!(concise_error_summary(full, 120), expected);
    }
    assert_eq!(concise_error_summary("failure", 0), "");
}

#[test]
fn banner_only_validation_errors_preserve_full_details_in_log() {
    let mut app = App::default();
    let _ = app.update(Message::OperationError(
        "First sentence. Full recovery instructions.".into(),
    ));
    assert_eq!(
        app.error_msg.as_deref(),
        Some("First sentence. Full recovery instructions.")
    );
    assert!(
        app.log_lines
            .iter()
            .any(|line| line.contains("Full recovery instructions."))
    );
    assert_eq!(
        app.log_lines
            .iter()
            .filter(|line| line.contains("Full recovery instructions."))
            .count(),
        1
    );
}

#[test]
fn shared_execution_failure_owns_error_presentation() {
    let mut app = App {
        error_msg: Some("failed".into()),
        operation_error: Some("failed".into()),
        ..App::default()
    };
    app.current_view = View::Flash;
    app.flash.step = FLASH_STEPS.len() - 1;
    assert!(!app.should_show_error_banner());

    app.current_view = View::Dashboard;
    assert!(app.should_show_error_banner());

    app.current_view = View::Advanced;
    app.adv_wizard.open(AdvAction::ImageInfo);
    app.adv_wizard.step = app.adv_wizard.exec_step();
    assert!(app.should_show_error_banner());
}

#[test]
fn non_operation_error_on_execution_surface_stays_global() {
    let mut app = App {
        current_view: View::Flash,
        error_msg: Some("log save failed".into()),
        operation_error: None,
        ..App::default()
    };
    app.flash.step = FLASH_STEPS.len() - 1;

    assert!(app.should_show_error_banner());
}

#[test]
fn operation_error_drives_shared_failure_status() {
    let app = App {
        operation_error: Some("firehose failed".into()),
        ..App::default()
    };

    assert_eq!(app.exec_status_copy().0, app.t("exec_failed_title"));
}

#[test]
fn failed_operation_preserves_the_phase_that_failed() {
    let mut app = App {
        operation: OperationExecution::fixture(
            true,
            Some(View::Flash),
            vec![
                OpStep {
                    label: "one".into(),
                },
                OpStep {
                    label: "two".into(),
                },
            ],
            0,
            None,
        ),
        ..App::default()
    };

    app.fail_op();

    assert_eq!(app.operation.current_step(), 0);
    assert!(!app.operation.is_running());
    assert_eq!(app.operation.view(), None);
}

#[test]
fn firmware_progress_steps_map_only_full_and_simple_flash() {
    assert!(OperationPhaseKind::Flash.is_firmware_progress_step(7));
    assert!(OperationPhaseKind::Flash.is_firmware_progress_step(8));
    assert!(OperationPhaseKind::SimpleFlash.is_firmware_progress_step(3));
    for kind in OperationPhaseKind::all() {
        if !matches!(
            kind,
            OperationPhaseKind::Flash | OperationPhaseKind::SimpleFlash
        ) {
            for step in 1..=kind.keys().len() {
                assert!(!kind.is_firmware_progress_step(step), "{kind:?}, {step}");
            }
        }
    }
}

#[test]
fn firmware_flash_progress_label_visibility_and_format() {
    let app = |kind: OperationPhaseKind, step: usize, busy: bool, err: Option<&str>| App {
        operation: OperationExecution::fixture(busy, None, Vec::new(), step, Some(kind)),
        flash_progress: Some(ltbox_device::edl::FlashProgress {
            partition: "super".into(),
            percent: 42,
            completed_bytes: 42,
            total_bytes: 100,
            operation_completed_bytes: 42,
            operation_total_bytes: 100,
        }),
        operation_error: err.map(str::to_string),
        ..App::default()
    };
    assert_eq!(
        app(OperationPhaseKind::Flash, 6, true, None)
            .firmware_flash_progress_label()
            .as_deref(),
        Some("super (42%)")
    );
    let mut simple = app(OperationPhaseKind::SimpleFlash, 2, true, None);
    simple.flash_progress = Some(ltbox_device::edl::FlashProgress {
        partition: "boot_a".into(),
        percent: 7,
        completed_bytes: 7,
        total_bytes: 100,
        operation_completed_bytes: 7,
        operation_total_bytes: 100,
    });
    assert_eq!(
        simple.firmware_flash_progress_label().as_deref(),
        Some("boot_a (7%)")
    );
    assert!(
        app(OperationPhaseKind::Flash, 5, true, None)
            .firmware_flash_progress_label()
            .is_none()
    );
    assert!(
        app(OperationPhaseKind::FlashPartitions, 1, true, None)
            .firmware_flash_progress_label()
            .is_none()
    );
    assert!(
        app(OperationPhaseKind::FlashPhysical, 2, true, None)
            .firmware_flash_progress_label()
            .is_none()
    );
    assert!(
        app(OperationPhaseKind::Root, 5, true, None)
            .firmware_flash_progress_label()
            .is_none()
    );
    assert!(
        app(OperationPhaseKind::Flash, 6, false, None)
            .firmware_flash_progress_label()
            .is_none()
    );
    assert!(
        app(OperationPhaseKind::Flash, 6, true, Some("boom"))
            .firmware_flash_progress_label()
            .is_none()
    );
}

#[test]
fn flash_progress_clears_across_op_lifecycle() {
    type Transition = (bool, fn(&mut App));
    let transitions: [Transition; 5] = [
        (false, |a| a.begin_op(View::Flash)),
        (true, |a| a.end_op()),
        (true, |a| a.fail_op()),
        (false, |a| a.begin_silent_op(View::Root)),
        (true, |a| a.end_silent_op()),
    ];
    for (running, clear) in transitions {
        let mut app = App::default();
        if running {
            let _ = app.begin_phased_op(View::Flash, OperationPhaseKind::Flash);
        }
        app.flash_progress = Some(ltbox_device::edl::FlashProgress {
            partition: "super".into(),
            percent: 10,
            completed_bytes: 10,
            total_bytes: 100,
            operation_completed_bytes: 10,
            operation_total_bytes: 100,
        });
        clear(&mut app);
        assert!(app.flash_progress.is_none());
        assert_eq!(app.operation.phase_kind(), None);
    }
}

#[test]
fn firmware_write_phases_share_the_same_localized_label() {
    for &lang in LANGUAGES {
        let translations = Translations::load(lang);
        assert_eq!(
            translations.t("op_flash_phase_7"),
            translations.t("op_simple_phase_write")
        );
    }
}

#[test]
fn exec_action_layout_keeps_one_primary_action() {
    assert_eq!(
        exec_action_layout(true, false, false),
        ExecActionLayout {
            primary: None,
            start_over_utility: false,
        }
    );
    assert_eq!(
        exec_action_layout(false, false, false),
        ExecActionLayout {
            primary: Some(ExecPrimaryAction::StartOver),
            start_over_utility: false,
        }
    );
    assert_eq!(
        exec_action_layout(false, false, true),
        ExecActionLayout {
            primary: Some(ExecPrimaryAction::OpenFolder),
            start_over_utility: true,
        }
    );
    assert_eq!(
        exec_action_layout(false, true, true),
        ExecActionLayout {
            primary: Some(ExecPrimaryAction::StartOver),
            start_over_utility: false,
        }
    );
}

#[test]
fn busy_operation_label_names_advanced_subtask() {
    let mut app = App {
        operation: OperationExecution::fixture(true, Some(View::Advanced), Vec::new(), 0, None),
        current_view: View::Advanced,
        ..App::default()
    };

    app.adv_wizard.action = Some(AdvAction::PatchDevinfo);
    assert_eq!(
        app.busy_operation_label(),
        app.t(AdvAction::PatchDevinfo.label_key()).to_string()
    );

    app.advanced_wizard_open = AdvancedWizardOpen::FlashParts;
    assert_eq!(
        app.busy_operation_label(),
        app.t(AdvAction::FlashPartitions.label_key()).to_string()
    );

    app.end_silent_op();
    app.begin_silent_op(View::Reboot);
    assert_eq!(app.busy_operation_label(), app.t("nav_reboot").to_string());
}

#[test]
fn busy_navigation_target_requires_a_live_operation() {
    assert_eq!(
        busy_navigation_target(true, Some(View::Flash)),
        Some(View::Flash)
    );
    assert_eq!(busy_navigation_target(false, Some(View::Flash)), None);
    assert_eq!(busy_navigation_target(true, None), None);
}

#[test]
fn loader_file_check_is_extension_based() {
    assert!(is_loader_file(std::path::Path::new("xbl_anything.melf")));
    assert!(is_loader_file(std::path::Path::new("firehose_loader.MBN")));
    assert!(is_loader_file(std::path::Path::new("prog.elf")));
    assert!(!is_loader_file(std::path::Path::new("xbl_s_devprg_ns.bin")));
}

#[test]
fn loader_picker_accepts_encrypted_manifests_for_supported_models() {
    let mut app = App::default();
    let melf = std::path::Path::new("xbl_s_devprg_ns.melf");
    let xml = std::path::Path::new("qsahara_device_programmer.xml");

    assert_eq!(app.loader_picker_exts(), &["melf", "xml", "x"]);
    let encrypted = std::path::Path::new("qsahara_device_programmer.X");
    assert!(app.loader_fits_model(encrypted));
    assert!(app.loader_fits_model(melf));
    assert!(app.loader_fits_model(xml));
    assert_eq!(
        app.loader_picker_subtitle(),
        app.t("loader_picker_subtitle_unknown")
    );

    app.device.connection = ConnectionStatus::Adb;
    app.device.model = "TB520FU".into();
    assert_eq!(app.loader_picker_exts(), &["melf"]);
    assert!(app.loader_fits_model(melf));
    assert!(!app.loader_fits_model(xml));
    assert!(!app.loader_fits_model(encrypted));
    assert_eq!(
        app.loader_picker_subtitle(),
        app.t("loader_picker_subtitle_standard")
    );

    app.device.model = "TB323FU".into();
    assert_eq!(app.loader_picker_exts(), &["xml", "x"]);
    assert!(app.loader_fits_model(encrypted));
    assert!(!app.loader_fits_model(melf));
    assert!(app.loader_fits_model(xml));
    assert_eq!(
        app.loader_picker_subtitle(),
        app.t("loader_picker_subtitle_manifest")
    );
    app.device.model = "TB324ZC".into();
    assert_eq!(app.loader_picker_exts(), &["xml", "x"]);
    assert!(app.loader_fits_model(encrypted));
    assert!(app.loader_fits_model(xml));
    assert!(!app.loader_fits_model(melf));
}

#[test]
fn edl_entry_action_uses_adb_from_fastboot() {
    assert_eq!(
        edl_entry_action(ConnectionStatus::Fastboot),
        EdlEntryAction::FastbootRebootThenAdb
    );
}

#[test]
fn edl_entry_action_waits_manual_without_usable_adb() {
    assert_eq!(
        edl_entry_action(ConnectionStatus::AdbUnauthorized),
        EdlEntryAction::ManualWait
    );
}

#[test]
fn country_patch_progress_requires_all_expected() {
    install_core_translator(Language::En);
    let mut progress = CountryPatchProgress::new(&["devinfo", "persist"]);
    progress.mark_flashed("devinfo");

    let err = progress.finish().expect_err("persist must be required");
    assert!(err.contains("persist"));
}

#[test]
fn country_patch_progress_oemowninfo_expected() {
    // Some models (e.g. TB320FC, TB323FU) patch oemowninfo + persist, not devinfo.
    let mut progress = CountryPatchProgress::new(&["oemowninfo", "persist"]);
    progress.mark_flashed("oemowninfo");
    progress.mark_flashed("persist");
    assert!(progress.finish().is_ok());
}

#[test]
fn country_patch_progress_surfaces_partition_failures() {
    install_core_translator(Language::En);
    let mut progress = CountryPatchProgress::new(&["devinfo", "persist"]);
    progress.mark_flashed("devinfo");
    progress.mark_failed("persist", "no known country code");

    let err = progress
        .finish()
        .expect_err("recorded persist failure must fail workflow");
    assert!(err.contains("persist: no known country code"));
}

#[test]
fn default_log_filter_hides_iced_noise_but_keeps_actionable_warnings() {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let capture = Capture(Arc::default());
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_new(DEFAULT_LOG_FILTER).unwrap())
        .with_ansi(false)
        .without_time()
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "iced_winit", "hidden-window-dump");
        tracing::info!(target: "iced_wgpu::window::compositor", "hidden-compositor-dump");
        tracing::warn!(target: "iced_futures::subscription::tracker", "hidden-full-channel");
        tracing::warn!(target: "iced_futures::runtime", "retained-stream-failure");
        tracing::warn!(target: "iced_winit", "retained-window-warning");
        tracing::info!(target: "ltbox_patch", "retained-operation-info");
        tracing::error!(target: "iced_futures::subscription::tracker", "retained-tracker-error");
    });
    let bytes = capture.0.lock().unwrap();
    let log = std::str::from_utf8(&bytes).unwrap();
    assert!(!log.contains("hidden-"), "{log}");
    for message in [
        "retained-stream-failure",
        "retained-window-warning",
        "retained-operation-info",
        "retained-tracker-error",
    ] {
        assert!(log.contains(message), "missing {message}: {log}");
    }
}
