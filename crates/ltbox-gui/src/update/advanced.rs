//! Advanced-menu handlers: physical/partition dump+flash, wizard nav. Extracted from `main.rs`.

use crate::*;
use iced::Task;
use ltbox_core::tr_args;

impl App {
    pub(crate) fn update_dump_phys(&mut self, msg: DumpPhysMsg) -> Task<Message> {
        match msg {
            DumpPhysMsg::DumpPhysSelectLoader => self.pick_loader_with_default(|__v| {
                Message::DumpPhys(DumpPhysMsg::DumpPhysLoaderChosen(__v))
            }),
            DumpPhysMsg::DumpPhysLoaderChosen(path) => {
                self.apply_loader_pick(path, |app, loader, err| {
                    app.dump_phys.loader_path = loader;
                    app.dump_phys.loader_error = err;
                });
                Task::none()
            }
            DumpPhysMsg::DumpPhysToggleRow(idx) => {
                if let Some(slot) = self.dump_phys.selected.get_mut(idx) {
                    *slot = !*slot;
                }
                Task::none()
            }
            DumpPhysMsg::DumpPhysNext => {
                match self.dump_phys.step {
                    0 => self.dump_phys.step = 1, // loader → select
                    1 => return self.update(Message::DumpPhys(DumpPhysMsg::DumpPhysSelectFolder)),
                    _ => {}
                };
                Task::none()
            }
            DumpPhysMsg::DumpPhysBack => {
                self.dump_phys.back();
                Task::none()
            }
            DumpPhysMsg::DumpPhysClose => {
                self.advanced_wizard_open = AdvancedWizardOpen::None;
                self.dump_phys.reset();
                Task::none()
            }
            DumpPhysMsg::DumpPhysSelectFolder => {
                // Dump destination — see DumpPartsSelectFolder.
                pickers::pick_folder_for(
                    pickers::PickerKind::OutputFolder,
                    &self.recent_paths,
                    |__v| Message::DumpPhys(DumpPhysMsg::DumpPhysFolderChosen(__v)),
                )
            }
            DumpPhysMsg::DumpPhysFolderChosen(path) => {
                if let Some(folder) = path {
                    let loader =
                        match self.validate_loader_path(&self.dump_phys.loader_path.clone()) {
                            Ok(p) => p,
                            Err(()) => return Task::none(),
                        };
                    self.remember_recent(pickers::PickerKind::OutputFolder, &folder);
                    self.dump_phys.output_dir = Some(folder.clone());
                    self.dump_phys.step = 2;
                    let phases =
                        self.begin_phased_op(View::Advanced, OperationPhaseKind::DumpPhysical);
                    self.error_msg = None;
                    let conn = self.device.connection;
                    let luns = self.dump_phys.selected_luns();
                    self.log_push(format!(
                        "[DumpPhys] {}",
                        tr_args!(
                            "live_dump_phys_batch_start",
                            count = luns.len().to_string(),
                            path = folder
                        )
                    ));
                    return task_heavy(
                        move || dump_physical_execute(conn, loader, folder, luns, phases),
                        |result| match result {
                            Ok(lines) => Message::DumpPhys(DumpPhysMsg::DumpPhysExecDone(lines)),
                            Err(e) => Message::OperationError(e),
                        },
                        |e| Err(format!("[DumpPhys] {e}")),
                    );
                }
                Task::none()
            }
            DumpPhysMsg::DumpPhysExecDone(lines) => {
                self.flush_exec_done_log(lines);
                self.end_op();
                Task::none()
            }
        }
    }

    pub(crate) fn update_flash_phys(&mut self, msg: FlashPhysMsg) -> Task<Message> {
        match msg {
            FlashPhysMsg::FlashPhysSelectLoader => self.pick_loader_with_default(|__v| {
                Message::FlashPhys(FlashPhysMsg::FlashPhysLoaderChosen(__v))
            }),
            FlashPhysMsg::FlashPhysLoaderChosen(path) => {
                self.apply_loader_pick(path, |app, loader, err| {
                    app.flash_phys.loader_path = loader;
                    app.flash_phys.loader_error = err;
                });
                Task::none()
            }
            FlashPhysMsg::FlashPhysToggleRow(idx) => {
                if let Some(slot) = self.flash_phys.selected.get_mut(idx) {
                    *slot = !*slot;
                }
                Task::none()
            }
            FlashPhysMsg::FlashPhysPickRowFile(idx) => {
                let spec = pickers::FilePickSpec::single()
                    .with_filter("Storage image", &["img", "bin", "mbn", "melf", "elf"]);
                pickers::pick_file_for(spec, &self.recent_paths, move |path| {
                    Message::FlashPhys(FlashPhysMsg::FlashPhysRowFileChosen(idx, path))
                })
            }
            FlashPhysMsg::FlashPhysRowFileChosen(idx, path) => {
                if idx < PHYS_LUN_COUNT
                    && let Some(p) = path
                {
                    self.remember_recent(pickers::PickerKind::File, &p);
                    self.flash_phys.file_paths[idx] = Some(p);
                    // Picking a file implicitly selects the row.
                    self.flash_phys.selected[idx] = true;
                }
                Task::none()
            }
            FlashPhysMsg::FlashPhysNext => {
                match self.flash_phys.step {
                    0 => self.flash_phys.step = 1,
                    1 => self.flash_phys.next(), // → Confirm
                    2 => return self.update(Message::FlashPhys(FlashPhysMsg::FlashPhysExecStart)),
                    _ => {}
                };
                Task::none()
            }
            FlashPhysMsg::FlashPhysBack => {
                self.flash_phys.back();
                Task::none()
            }
            FlashPhysMsg::FlashPhysClose => {
                self.advanced_wizard_open = AdvancedWizardOpen::None;
                self.flash_phys.reset();
                Task::none()
            }
            FlashPhysMsg::FlashPhysExecStart => {
                let loader = match self.validate_loader_path(&self.flash_phys.loader_path.clone()) {
                    Ok(p) => p,
                    Err(()) => return Task::none(),
                };
                self.flash_phys.next(); // advance to Exec screen
                let phases =
                    self.begin_phased_op(View::Advanced, OperationPhaseKind::FlashPhysical);
                self.error_msg = None;
                let conn = self.device.connection;
                let pairs = self.flash_phys.active_pairs();
                self.log_lines.push(format!(
                    "[FlashPhys] {}",
                    tr_args!("log_flashphys_starting", count = pairs.len())
                ));
                task_heavy(
                    move || flash_physical_execute(conn, loader, pairs, phases),
                    |result| match result {
                        Ok(lines) => Message::FlashPhys(FlashPhysMsg::FlashPhysExecDone(lines)),
                        Err(e) => Message::OperationError(e),
                    },
                    |e| Err(format!("[FlashPhys] {e}")),
                )
            }
            FlashPhysMsg::FlashPhysExecDone(lines) => {
                self.flush_exec_done_log(lines);
                self.end_op();
                Task::none()
            }
        }
    }

    pub(crate) fn update_dump_parts(&mut self, msg: DumpPartsMsg) -> Task<Message> {
        match msg {
            DumpPartsMsg::DumpPartsSelectLoader => self.pick_loader_with_default(|__v| {
                Message::DumpParts(DumpPartsMsg::DumpPartsLoaderChosen(__v))
            }),
            DumpPartsMsg::DumpPartsLoaderChosen(path) => {
                self.apply_loader_pick(path, |app, loader, err| {
                    app.dump_parts.loader_path = loader;
                    app.dump_parts.loader_error = err;
                });
                Task::none()
            }
            DumpPartsMsg::DumpPartsToggleRow(idx) => {
                if let Some(row) = self.dump_parts.rows.get_mut(idx) {
                    row.selected = !row.selected;
                }
                Task::none()
            }
            DumpPartsMsg::DumpPartsNext => {
                match self.dump_parts.step {
                    0 => return self.update(Message::DumpParts(DumpPartsMsg::DumpPartsScanStart)),
                    1 => {
                        return self
                            .update(Message::DumpParts(DumpPartsMsg::DumpPartsSelectFolder));
                    }
                    _ => {}
                };
                Task::none()
            }
            DumpPartsMsg::DumpPartsBack => {
                if self.dump_parts.step == 1
                    && partition_table_leading_action(self.dump_parts.entry_connection)
                        == WizardLeadingAction::Cancel
                {
                    let loader =
                        match self.validate_loader_path(&self.dump_parts.loader_path.clone()) {
                            Ok(loader) => loader,
                            Err(()) => return Task::none(),
                        };
                    self.advanced_wizard_open = AdvancedWizardOpen::None;
                    self.dump_parts.reset();
                    return self.start_edl_reboot_with_loader(
                        RebootTarget::System,
                        std::path::PathBuf::from(loader),
                    );
                }
                self.dump_parts.back();
                Task::none()
            }
            DumpPartsMsg::DumpPartsClose => {
                self.advanced_wizard_open = AdvancedWizardOpen::None;
                self.dump_parts.reset();
                Task::none()
            }
            DumpPartsMsg::DumpPartsScanStart => {
                let loader = match self.validate_loader_path(&self.dump_parts.loader_path.clone()) {
                    Ok(p) => p,
                    Err(()) => return Task::none(),
                };
                self.dump_parts.entry_connection = Some(self.device.connection);
                self.dump_parts.scanning = true;
                self.dump_parts.scan_error = None;
                self.dump_parts.rows.clear();
                self.begin_op(View::Advanced);
                self.error_msg = None;
                let conn = self.device.connection;
                self.log_push(format!(
                    "[DumpParts] {}",
                    ltbox_core::i18n::tr("live_dumpparts_scan_start")
                ));
                task_heavy(
                    move || dump_parts_scan(conn, loader),
                    |__v| Message::DumpParts(DumpPartsMsg::DumpPartsScanDone(__v)),
                    |e| DumpPartsScanResult {
                        logs: vec![format!("[DumpParts] {e}")],
                        rows: Vec::new(),
                        error: Some(e),
                    },
                )
            }
            DumpPartsMsg::DumpPartsScanDone(result) => {
                self.flush_exec_done_log(result.logs);
                self.end_op();
                self.dump_parts.scanning = false;
                self.dump_parts.rows = result.rows;
                self.dump_parts.apply_sort();
                self.dump_parts.scan_error =
                    self.parts_scan_outcome(result.error, self.dump_parts.rows.is_empty());
                if self.dump_parts.scan_error.is_none() {
                    self.dump_parts.step = 1;
                    // A successful Firehose GPT scan proves the device is in
                    // EDL; reflect it immediately (the 3s poll may still show a
                    // stale ADB/Fastboot state) so a sidebar bounce right after
                    // the scan keeps the loaded table via `advanced_in_progress`.
                    self.apply_device_snapshot(DevicePollResult {
                        status: ConnectionStatus::Edl,
                        ..DevicePollResult::default()
                    });
                }
                Task::none()
            }
            DumpPartsMsg::DumpPartsSortBy(col) => {
                self.dump_parts.toggle_sort(col);
                Task::none()
            }
            DumpPartsMsg::DumpPartsToggleAll => {
                let all_selected = !self.dump_parts.rows.is_empty()
                    && self.dump_parts.rows.iter().all(|r| r.selected);
                let target = !all_selected;
                for r in self.dump_parts.rows.iter_mut() {
                    r.selected = target;
                }
                Task::none()
            }
            DumpPartsMsg::DumpPartsSelectFolder => {
                // Dump destination, not a firmware source — goes to the
                // `OutputFolder` bucket so the MRU list doesn't mix input
                // firmware dirs with output dump dirs.
                pickers::pick_folder_for(
                    pickers::PickerKind::OutputFolder,
                    &self.recent_paths,
                    |__v| Message::DumpParts(DumpPartsMsg::DumpPartsFolderChosen(__v)),
                )
            }
            DumpPartsMsg::DumpPartsFolderChosen(path) => {
                if let Some(folder) = path {
                    self.remember_recent(pickers::PickerKind::OutputFolder, &folder);
                    self.dump_parts.output_dir = Some(folder.clone());
                    self.dump_parts.step = 2;
                    let phases =
                        self.begin_phased_op(View::Advanced, OperationPhaseKind::DumpPartitions);
                    self.error_msg = None;
                    let loader = self.dump_parts.loader_path.clone().unwrap_or_default();
                    let rows = self.dump_parts.selected_rows();
                    self.log_push(format!(
                        "[DumpParts] {}",
                        tr_args!(
                            "live_dumpparts_batch_start",
                            count = rows.len().to_string(),
                            path = folder
                        )
                    ));
                    return task_heavy(
                        move || dump_parts_execute(loader, folder, rows, phases),
                        |result| match result {
                            Ok(lines) => Message::DumpParts(DumpPartsMsg::DumpPartsExecDone(lines)),
                            Err(e) => Message::OperationError(e),
                        },
                        |e| Err(format!("[DumpParts] {e}")),
                    );
                }
                Task::none()
            }
            DumpPartsMsg::DumpPartsExecDone(lines) => {
                self.flush_exec_done_log(lines);
                self.end_op();
                Task::none()
            }
        }
    }

    pub(crate) fn update_flash_parts(&mut self, msg: FlashPartsMsg) -> Task<Message> {
        match msg {
            FlashPartsMsg::FlashPartsSelectLoader => self.pick_loader_with_default(|__v| {
                Message::FlashParts(FlashPartsMsg::FlashPartsLoaderChosen(__v))
            }),
            FlashPartsMsg::FlashPartsLoaderChosen(path) => {
                self.apply_loader_pick(path, |app, loader, err| {
                    app.flash_parts.loader_path = loader;
                    app.flash_parts.loader_error = err;
                });
                Task::none()
            }
            FlashPartsMsg::FlashPartsToggleRow(idx) => {
                if let Some(row) = self.flash_parts.rows.get_mut(idx) {
                    row.advance_action();
                }
                Task::none()
            }
            FlashPartsMsg::FlashPartsPickRowFile(idx) => {
                let spec = pickers::FilePickSpec::single().with_filter(
                    "Partition image",
                    &["img", "bin", "mbn", "melf", "elf", "efi"],
                );
                pickers::pick_file_for(spec, &self.recent_paths, move |path| {
                    Message::FlashParts(FlashPartsMsg::FlashPartsRowFileChosen(idx, path))
                })
            }
            FlashPartsMsg::FlashPartsRowFileChosen(idx, path) => {
                if let Some(p) = path {
                    self.remember_recent(pickers::PickerKind::File, &p);
                    if let Some(row) = self.flash_parts.rows.get_mut(idx) {
                        row.assign_file(p);
                    }
                }
                Task::none()
            }
            FlashPartsMsg::FlashPartsClearRowFile(idx) => {
                if let Some(row) = self.flash_parts.rows.get_mut(idx) {
                    row.clear_file();
                }
                Task::none()
            }
            FlashPartsMsg::FlashPartsNext => {
                match self.flash_parts.step {
                    0 => {
                        return self
                            .update(Message::FlashParts(FlashPartsMsg::FlashPartsScanStart));
                    }
                    1 => self.flash_parts.next(), // → Confirm
                    2 => {
                        return self
                            .update(Message::FlashParts(FlashPartsMsg::FlashPartsExecStart));
                    }
                    _ => {}
                };
                Task::none()
            }
            FlashPartsMsg::FlashPartsBack => {
                if self.flash_parts.step == 1
                    && partition_table_leading_action(self.flash_parts.entry_connection)
                        == WizardLeadingAction::Cancel
                {
                    return self.update(Message::FlashParts(FlashPartsMsg::FlashPartsClose));
                }
                self.flash_parts.back();
                Task::none()
            }
            FlashPartsMsg::FlashPartsClose => {
                if matches!(self.flash_parts.step, 1 | 2)
                    && partition_table_leading_action(self.flash_parts.entry_connection)
                        == WizardLeadingAction::Cancel
                {
                    let loader =
                        match self.validate_loader_path(&self.flash_parts.loader_path.clone()) {
                            Ok(loader) => loader,
                            Err(()) => return Task::none(),
                        };
                    self.advanced_wizard_open = AdvancedWizardOpen::None;
                    self.flash_parts.reset();
                    return self.start_edl_reboot_with_loader(RebootTarget::System, loader.into());
                }
                self.advanced_wizard_open = AdvancedWizardOpen::None;
                self.flash_parts.reset();
                Task::none()
            }
            FlashPartsMsg::FlashPartsScanStart => {
                let loader = match self.validate_loader_path(&self.flash_parts.loader_path.clone())
                {
                    Ok(p) => p,
                    Err(()) => return Task::none(),
                };
                self.flash_parts.entry_connection = Some(self.device.connection);
                // Loader-upload + GPT read to enumerate partitions — a
                // *read*, not a flash. Use the Advanced busy view so the
                // dialog shows `busy_partition_scan` ("Reading partition
                // info…") like Read Partitions, not "Flash Firmware".
                self.begin_op(View::Advanced);
                self.error_msg = None;
                self.flash_parts.scanning = true;
                self.flash_parts.scan_error = None;
                self.flash_parts.rows.clear();
                let conn = self.device.connection;
                self.log_push(format!(
                    "[FlashParts] {}",
                    ltbox_core::i18n::tr("live_flashparts_scan_start")
                ));
                task_heavy(
                    move || flash_parts_scan(conn, loader),
                    |__v| Message::FlashParts(FlashPartsMsg::FlashPartsScanDone(__v)),
                    |e| FlashPartsScanResult {
                        logs: vec![format!("[FlashParts] {e}")],
                        rows: Vec::new(),
                        error: Some(e),
                    },
                )
            }
            FlashPartsMsg::FlashPartsScanDone(result) => {
                self.flush_exec_done_log(result.logs);
                self.flash_parts.scanning = false;
                self.flash_parts.rows = result.rows;
                self.flash_parts.apply_sort();
                self.flash_parts.scan_error =
                    self.parts_scan_outcome(result.error, self.flash_parts.rows.is_empty());
                self.end_op();
                if self.flash_parts.scan_error.is_none() {
                    self.flash_parts.next(); // → Select
                    // A successful Firehose GPT scan proves the device is in
                    // EDL; reflect it immediately (the 3s poll may still show a
                    // stale ADB/Fastboot state) so a sidebar bounce right after
                    // the scan keeps the loaded table via `advanced_in_progress`.
                    self.apply_device_snapshot(DevicePollResult {
                        status: ConnectionStatus::Edl,
                        ..DevicePollResult::default()
                    });
                }
                Task::none()
            }
            FlashPartsMsg::FlashPartsSortBy(col) => {
                self.flash_parts.toggle_sort(col);
                Task::none()
            }
            FlashPartsMsg::FlashPartsExecStart => {
                if self.flash_parts.step != 2 || !self.flash_parts.can_next() {
                    return Task::none();
                }
                self.flash_parts.next(); // advance to Exec screen
                // Advanced busy view (not Flash) so the busy dialog shows the
                // partition-write message via `busy_body_override`, not
                // "Flash Firmware is in progress".
                let phases =
                    self.begin_phased_op(View::Advanced, OperationPhaseKind::FlashPartitions);
                self.error_msg = None;
                let loader = self.flash_parts.loader_path.clone().unwrap_or_default();
                let rows = self.flash_parts.active_rows();
                let flash_cnt = rows
                    .iter()
                    .filter(|r| r.state == FlashRowState::Write)
                    .count();
                let erase_cnt = rows
                    .iter()
                    .filter(|r| r.state == FlashRowState::Erase)
                    .count();
                self.log_push(format!(
                    "[FlashParts] {}",
                    tr_args!(
                        "live_flashparts_batch_start",
                        flash_count = flash_cnt.to_string(),
                        erase_count = erase_cnt.to_string()
                    )
                ));
                task_heavy(
                    move || flash_parts_execute(loader, rows, phases),
                    |result| match result {
                        Ok(lines) => Message::FlashParts(FlashPartsMsg::FlashPartsExecDone(lines)),
                        Err(e) => Message::OperationError(e),
                    },
                    |e| Err(format!("[FlashParts] {e}")),
                )
            }
            FlashPartsMsg::FlashPartsExecDone(lines) => {
                self.flush_exec_done_log(lines);
                self.end_op();
                Task::none()
            }
        }
    }

    pub(crate) fn update_simple_flash(&mut self, msg: SimpleFlashMsg) -> Task<Message> {
        match msg {
            SimpleFlashMsg::SimpleFlashNext => {
                match self.simple_flash.step {
                    // Source → Confirm once a firmware folder is selected.
                    0 if self.simple_flash.firmware_folder.is_some() => {
                        self.simple_flash.next();
                    }
                    // Confirm → start the flash.
                    1 => {
                        return self
                            .update(Message::SimpleFlash(SimpleFlashMsg::SimpleFlashExecStart));
                    }
                    _ => {}
                };
                Task::none()
            }
            SimpleFlashMsg::SimpleFlashBack => {
                self.simple_flash.back();
                Task::none()
            }
            SimpleFlashMsg::SimpleFlashClose => {
                self.advanced_wizard_open = AdvancedWizardOpen::None;
                self.simple_flash.reset();
                Task::none()
            }
            SimpleFlashMsg::SimpleFlashSelectFolder => pickers::pick_folder_for(
                pickers::PickerKind::QfilFirmwareFolder,
                &self.recent_paths,
                |__v| Message::SimpleFlash(SimpleFlashMsg::SimpleFlashFolderChosen(__v)),
            ),
            SimpleFlashMsg::SimpleFlashFolderChosen(path) => {
                if let Some(folder) = path {
                    self.remember_recent(pickers::PickerKind::QfilFirmwareFolder, &folder);
                    // Mirror the Flash wizard: retarget an extracted-root pick
                    // to its flashable `image/` child.
                    let folder = crate::loader::redirect_str(folder);
                    self.simple_flash.firmware_folder = Some(folder);
                }
                Task::none()
            }
            SimpleFlashMsg::SimpleFlashExecStart => {
                self.simple_flash.next(); // → Exec screen
                let phases = self.begin_phased_op(View::Advanced, OperationPhaseKind::SimpleFlash);
                self.error_msg = None;
                let conn = self.device.connection;
                let fw_folder = self
                    .simple_flash
                    .firmware_folder
                    .clone()
                    .unwrap_or_default();
                self.log_push(format!(
                    "[SimpleFlash] {}",
                    tr_args!("live_flash_firmware_folder", path = fw_folder.clone())
                ));
                task_heavy(
                    move || simple_flash_worker(conn, fw_folder, phases),
                    |result| match result {
                        Ok(lines) => {
                            Message::SimpleFlash(SimpleFlashMsg::SimpleFlashExecDone(lines))
                        }
                        Err(e) => Message::OperationError(e),
                    },
                    Err,
                )
            }
            SimpleFlashMsg::SimpleFlashExecDone(lines) => {
                self.flush_exec_done_log(lines);
                self.end_op();
                Task::none()
            }
        }
    }

    pub(crate) fn update_adv(&mut self, msg: AdvMsg) -> Task<Message> {
        match msg {
            AdvMsg::AdvConfirm(a) => {
                // Dedicated EDL wizards can skip their loader step via Settings.
                if matches!(a, AdvAction::FlashPartitions) {
                    self.flash_parts.reset();
                    self.advanced_wizard_open = AdvancedWizardOpen::FlashParts;
                    self.apply_default_loader_to_advanced_wizard()
                } else if matches!(a, AdvAction::DumpPartitions) {
                    self.dump_parts.reset();
                    self.advanced_wizard_open = AdvancedWizardOpen::DumpParts;
                    self.apply_default_loader_to_advanced_wizard()
                } else if matches!(a, AdvAction::DumpPhysical) {
                    self.dump_phys.reset();
                    self.advanced_wizard_open = AdvancedWizardOpen::DumpPhys;
                    self.apply_default_loader_to_advanced_wizard()
                } else if matches!(a, AdvAction::FlashPhysical) {
                    self.flash_phys.reset();
                    self.advanced_wizard_open = AdvancedWizardOpen::FlashPhys;
                    self.apply_default_loader_to_advanced_wizard()
                } else if matches!(a, AdvAction::SimpleFlash) {
                    // Dedicated wizard: intro (description) → folder picker →
                    // confirm → flash. No loader step (the loader comes from
                    // the firmware folder), so no default-loader fold-through.
                    self.simple_flash.reset();
                    self.advanced_wizard_open = AdvancedWizardOpen::SimpleFlash;
                    Task::none()
                } else {
                    self.update(Message::Adv(AdvMsg::AdvWizOpen(a)))
                }
            }
            AdvMsg::AdvWizOpen(a) => {
                self.adv_wizard.open(a);
                // Apply the saved loader only to EDL-based index queries.
                if matches!(a, AdvAction::DetectArb)
                    && self.rollback_query_needs_loader()
                    && let Some(path) = self.resolved_default_loader()
                    && let Ok(resolved) = self.resolve_loader_input(&path)
                {
                    self.adv_wizard.file_path = Some(resolved);
                }
                self.adv_confirm_path = None;
                Task::none()
            }
            AdvMsg::AdvWizBack => {
                if self.adv_wizard.step == 0 {
                    // Back on step 0 closes the wizard.
                    self.adv_wizard.reset();
                    self.adv_confirm_path = None;
                } else {
                    self.adv_wizard.back();
                }
                Task::none()
            }
            AdvMsg::AdvWizNext => {
                if self.adv_wizard.action != Some(AdvAction::DetectArb)
                    && !self.adv_wizard.can_next()
                {
                    return Task::none();
                }
                if self.adv_wizard.is_image_info() && self.adv_wizard.step == 0 {
                    self.adv_wizard.next();
                    return self.update(Message::Adv(AdvMsg::AdvImageInfoExecStart));
                }
                // DetectArb source step jumps straight to exec.
                if matches!(self.adv_wizard.action, Some(AdvAction::DetectArb))
                    && self.adv_wizard.step == 0
                {
                    if !self.can_query_rollback() {
                        return Task::none();
                    }
                    self.adv_wizard.step = 1;
                    return self.update(Message::Adv(AdvMsg::AdvDetectArbExecStart));
                }
                // PatchArb source step inspects rollback indices.
                if matches!(self.adv_wizard.action, Some(AdvAction::PatchArb)) {
                    if self.adv_wizard.step == 0 {
                        let Some(folder) = self.adv_wizard.file_path.clone() else {
                            return Task::none();
                        };
                        let dir = std::path::PathBuf::from(&folder);
                        let boot = dir.join("boot.img");
                        let vbmeta = dir.join("vbmeta_system.img");
                        if !boot.is_file() {
                            self.error_msg = Some(tr_args!(
                                "err_patch_arb_missing_image",
                                image = "boot.img",
                                path = dir.display().to_string()
                            ));
                            return Task::none();
                        }
                        if !vbmeta.is_file() {
                            self.error_msg = Some(tr_args!(
                                "err_patch_arb_missing_image",
                                image = "vbmeta_system.img",
                                path = dir.display().to_string()
                            ));
                            return Task::none();
                        }
                        let boot_info = match ltbox_patch::avb::extract_image_avb_info(&boot) {
                            Ok(i) => i,
                            Err(e) => {
                                self.error_msg = Some(tr_args!(
                                    "err_patch_arb_inspect_failed",
                                    image = "boot.img",
                                    error = e.to_string()
                                ));
                                return Task::none();
                            }
                        };
                        let vbmeta_info = match ltbox_patch::avb::extract_image_avb_info(&vbmeta) {
                            Ok(i) => i,
                            Err(e) => {
                                self.error_msg = Some(tr_args!(
                                    "err_patch_arb_inspect_failed",
                                    image = "vbmeta_system.img",
                                    error = e.to_string()
                                ));
                                return Task::none();
                            }
                        };
                        self.adv_wizard.arb_inspect =
                            Some((boot_info.rollback_index, vbmeta_info.rollback_index));
                        self.error_msg = None;
                        return self.open_manual_rollback_editor();
                    }
                }
                // Change Country: leaving the Country step → apply the Settings
                // default EDL loader the same way the dedicated EDL wizards do
                // (auto-fill + skip the Loader step when a fitting default is set),
                // so loader handling stays consistent across every picker flow.
                if matches!(self.adv_wizard.action, Some(AdvAction::PatchDevinfo))
                    && self.adv_wizard.step == 0
                {
                    self.adv_wizard.next(); // Country → Loader
                    if let Some(path) = self.resolved_default_loader() {
                        // resolved_default_loader already model-fit-checks the path.
                        match self.resolve_loader_input(&path) {
                            Ok(resolved) => {
                                self.adv_wizard.file_path = Some(resolved);
                                self.error_msg = None;
                                self.adv_wizard.next(); // Loader → Confirm
                            }
                            Err(msg) => self.error_msg = Some(msg),
                        }
                    }
                    return Task::none();
                }
                if self.adv_wizard.is_confirm_step() {
                    let Some(action) = self.adv_wizard.action else {
                        return Task::none();
                    };
                    // Change Country Code runs the EDL country-change worker
                    // (dump → patch → flash the model's country partitions →
                    // reset to system), not the local-file advanced_file_worker.
                    if matches!(action, AdvAction::PatchDevinfo) {
                        let Some(target_code) = self.adv_wizard.country.clone() else {
                            return Task::none();
                        };
                        // Re-resolve the loader against the NOW-connected model:
                        // selections may predate this device (e.g. a `.melf` needs
                        // its Sahara manifest only when the live device is TB323FU).
                        // Pick-time resolve (AdvWizBrowseDone) keeps Confirm
                        // accurate; this re-check keeps Start correct if the model
                        // changed in between.
                        let Some(picked) = self.adv_wizard.file_path.clone() else {
                            return Task::none();
                        };
                        let loader = match self.resolve_loader_input(&picked) {
                            Ok(l) => l,
                            Err(msg) => {
                                self.error_msg = Some(msg);
                                return Task::none();
                            }
                        };
                        self.adv_wizard.next(); // → exec screen
                        let phases =
                            self.begin_phased_op(View::Advanced, OperationPhaseKind::ChangeCountry);
                        self.error_msg = None;
                        let conn = self.device.connection;
                        let device_model = self.device.model.clone();
                        let ll = self.live_labels();
                        let label = self.t(action.label_key()).to_string();
                        self.log_push(format!("[Advanced] {label}"));
                        return task_heavy(
                            move || {
                                change_country_worker(
                                    conn,
                                    device_model,
                                    target_code,
                                    std::path::PathBuf::from(loader),
                                    ll,
                                    phases,
                                )
                            },
                            |result| match result {
                                Ok(lines) => Message::Adv(AdvMsg::AdvExecDone(lines)),
                                Err(e) => Message::OperationError(e),
                            },
                            Err,
                        );
                    }
                    self.adv_confirm_path = self.adv_wizard.file_path.clone();
                    if let Some(code) = self.adv_wizard.country.clone() {
                        self.wf_config.country_action = CountryAction::Set(code);
                    }
                    // Pre-create output folder so the Done card's
                    // "Open Folder" pill always points somewhere real.
                    if action == AdvAction::PatchArb {
                        self.adv_wizard.output_dir = self
                            .adv_wizard
                            .file_path
                            .as_ref()
                            .map(std::path::PathBuf::from);
                    } else if action.produces_output() {
                        let dir = adv_output_dir(action);
                        let _ = std::fs::create_dir_all(&dir);
                        self.adv_wizard.output_dir = Some(dir);
                    } else {
                        self.adv_wizard.output_dir = None;
                    }
                    self.adv_wizard.next();
                    return self.update(Message::Adv(AdvMsg::AdvExec(action)));
                }
                self.adv_wizard.next();
                Task::none()
            }
            AdvMsg::AdvWizBrowse => {
                if self.adv_wizard.is_image_info() {
                    let spec = pickers::FilePickSpec::multi()
                        .with_filter("Android image (*.img)", &["img"]);
                    return pickers::pick_files_for(spec, &self.recent_paths, |__v| {
                        Message::Adv(AdvMsg::AdvWizBrowseManyDone(__v))
                    });
                }
                let kind = self.adv_wizard.picker_kind();
                if kind.is_folder() {
                    return pickers::pick_folder_for(kind, &self.recent_paths, |__v| {
                        Message::Adv(AdvMsg::AdvWizBrowseDone(__v))
                    });
                }
                let (filter_label, filter_exts) = self.advanced_picker_exts();
                let mut spec = pickers::FilePickSpec::single();
                if !filter_exts.is_empty() {
                    spec = spec.with_filter(filter_label, filter_exts);
                }
                pickers::pick_file_for(spec, &self.recent_paths, |__v| {
                    Message::Adv(AdvMsg::AdvWizBrowseDone(__v))
                })
            }
            AdvMsg::AdvWizBrowseDone(path) => {
                if let Some(p) = path {
                    if std::path::Path::new(&p).exists() {
                        // Kind is derived from the action (folder ops →
                        // folder bucket, file ops → File) rather than the
                        // runtime is_dir() check — trusting the action
                        // keeps buckets consistent even if rfd returns an
                        // unexpected path type.
                        self.remember_recent(self.adv_wizard.picker_kind(), &p);
                    }
                    // Change Country Code picks an EDL loader: resolve it now
                    // (e.g. upgrade a TB323FU .melf to its sibling Sahara
                    // manifest) so the confirm screen shows the loader actually
                    // used and Start needs no re-resolve.
                    if matches!(
                        self.adv_wizard.action,
                        Some(AdvAction::PatchDevinfo | AdvAction::DetectArb)
                    ) {
                        match self.resolve_loader_input(&p) {
                            Ok(resolved) => {
                                self.adv_wizard.file_path = Some(resolved);
                                self.error_msg = None;
                            }
                            Err(msg) => {
                                self.adv_wizard.file_path = None;
                                self.error_msg = Some(msg);
                            }
                        }
                        return Task::none();
                    }
                    self.adv_wizard.arb_targets = None;
                    self.adv_wizard.arb_inspect = None;
                    self.adv_wizard.file_path = Some(p);
                }
                Task::none()
            }
            AdvMsg::AdvWizBrowseManyDone(paths) => {
                if let Some(paths) = paths {
                    let paths: Vec<String> = paths
                        .into_iter()
                        .filter(|p| {
                            std::path::Path::new(p)
                                .extension()
                                .and_then(|s| s.to_str())
                                .map(|s| s.eq_ignore_ascii_case("img"))
                                .unwrap_or(false)
                        })
                        .collect();
                    for p in &paths {
                        if std::path::Path::new(p).exists() {
                            self.remember_recent(pickers::PickerKind::File, p);
                        }
                    }
                    self.adv_wizard.file_paths = paths;
                    self.adv_wizard.file_path = None;
                }
                Task::none()
            }
            AdvMsg::AdvWizOpenCountry => {
                self.adv_needs_country = true;
                self.open_country_popup();
                Task::none()
            }
            AdvMsg::AdvWizOpenRegionTarget => {
                self.region_target_popup_open = true;
                Task::none()
            }
            AdvMsg::AdvWizOpenOutputFolder => {
                if let Some(dir) = self.adv_wizard.output_dir.clone()
                    && let Err(err) = open_in_file_manager(&dir)
                {
                    // Surface the failed command + path in the log
                    // so the user can see what was tried — silent
                    // no-op was the old behaviour and made missing
                    // xdg-open invisible on Linux.
                    self.log_push(format!(
                        "[GUI] {}",
                        tr_args!("log_gui_open_folder_failed", error = err)
                    ));
                }
                Task::none()
            }
            AdvMsg::AdvExec(action) => {
                // Picker ran in AdvConfirm; replay the saved path.
                let Some(path) = self.adv_confirm_path.clone() else {
                    return Task::none();
                };
                self.update(Message::Adv(AdvMsg::AdvFileSelected(action, Some(path))))
            }
            AdvMsg::AdvFileSelected(action, path) => {
                if let Some(input_path) = path {
                    let Some(phase_kind) = OperationPhaseKind::for_advanced_file(action) else {
                        self.error_msg = Some(ltbox_core::i18n::tr("live_advanced_use_dedicated"));
                        return Task::none();
                    };
                    // See AdvWizBrowseDone — trust the action's kind over
                    // the runtime is_dir() probe.
                    self.remember_recent(self.adv_wizard.picker_kind(), &input_path);
                    let phases = self.begin_phased_op(View::Advanced, phase_kind);
                    self.error_msg = None;
                    let action_label = self.t(action.label_key()).to_string();
                    self.log_push(format!("[Advanced] {}: {}", action_label, input_path));
                    // PatchDevinfo only — unused otherwise.
                    let adv_country: Option<String> =
                        self.wf_config.country_action.target().map(str::to_string);
                    // RegionConvert only — user-picked target.
                    let adv_region_target: Option<DeviceRegion> = self.adv_wizard.region_target;
                    // PatchArb only — committed unix-timestamp index.
                    let adv_arb_index = self.adv_wizard.arb_targets;
                    let output_dir: std::path::PathBuf = self
                        .adv_wizard
                        .output_dir
                        .clone()
                        .unwrap_or_else(|| adv_output_dir(action));
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                ltbox_core::runtime::run_heavy(move || {
                                    advanced_file_worker(
                                        input_path,
                                        action,
                                        adv_country,
                                        adv_region_target,
                                        adv_arb_index,
                                        output_dir,
                                        action_label,
                                        phases,
                                    )
                                })
                                .and_then(|r| r)
                            })
                            .await
                            .unwrap_or_else(|_| Err(ltbox_core::i18n::tr("err_task_failed")))
                        },
                        |result| match result {
                            Ok(lines) => Message::Adv(AdvMsg::AdvExecDone(lines)),
                            Err(e) => Message::OperationError(e),
                        },
                    );
                }
                Task::none()
            }
            AdvMsg::AdvExecDone(lines) => {
                self.flush_exec_done_log(lines);
                // Leave adv_wizard / adv_confirm_path intact so the exec
                // screen stays visible with Done/Failed until StartOver.
                self.end_op();
                Task::none()
            }
            AdvMsg::AdvImageInfoExecStart => {
                let paths: Vec<std::path::PathBuf> = self
                    .adv_wizard
                    .file_paths
                    .iter()
                    .map(std::path::PathBuf::from)
                    .collect();
                let scanning = tr_args!("adv_image_info_scanning", count = paths.len().to_string());
                self.set_image_info_log(scanning);
                self.begin_silent_op(View::Advanced);
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            ltbox_core::runtime::run_heavy(move || {
                                ltbox_patch::avb::image_info_report(&paths)
                                    .map_err(|e| e.to_string())
                            })
                            .and_then(|r| r)
                        })
                        .await
                        .unwrap_or_else(|e| Err(tr_args!("err_task_failed_with_error", error = e)))
                    },
                    |__v| Message::Adv(AdvMsg::AdvImageInfoExecDone(__v)),
                )
            }
            AdvMsg::AdvImageInfoExecDone(result) => {
                self.end_silent_op();
                match result {
                    Ok(report) => {
                        self.error_msg = None;
                        self.set_image_info_log(report);
                    }
                    Err(e) => {
                        self.error_msg = Some(e.clone());
                        self.set_image_info_log(tr_args!(
                            "log_operation_error",
                            error = e.to_string()
                        ));
                    }
                }
                Task::none()
            }
            AdvMsg::AdvDetectArbExecStart => {
                if !self.can_query_rollback() {
                    return Task::none();
                }
                let phases = self.begin_phased_op(View::Advanced, OperationPhaseKind::DetectArb);
                self.error_msg = None;
                let conn = self.device.connection;
                let device_model = self.device.model.clone();
                let loader_path = self.adv_wizard.file_path.clone();
                let i_anti = self.t("arb_detect_is_anti_rollback").to_string();
                let i_not = self.t("arb_detect_no_anti_rollback").to_string();
                let i_reboot_fastboot = self.t("live_arb_reboot_to_fastboot").to_string();
                let i_reboot_system = self.t("live_arb_reboot_to_system").to_string();
                let i_edl_dump = self.t("live_arb_edl_dump").to_string();
                task_heavy(
                    move || {
                        let mut log = Vec::new();
                        match detect_arb_run(
                            conn,
                            device_model,
                            loader_path,
                            &i_anti,
                            &i_not,
                            &i_reboot_fastboot,
                            &i_reboot_system,
                            &i_edl_dump,
                            phases,
                            &mut log,
                        ) {
                            Ok(()) => Ok(log),
                            Err(e) => Err(e),
                        }
                    },
                    |__v| Message::Adv(AdvMsg::AdvDetectArbExecDone(__v)),
                    Err,
                )
            }
            AdvMsg::AdvDetectArbExecDone(result) => {
                match result {
                    Ok(lines) => {
                        self.flush_exec_done_log(lines);
                        self.end_op();
                    }
                    Err(e) => {
                        return self.update(Message::OperationError(e));
                    }
                }
                Task::none()
            }
        }
    }
}
