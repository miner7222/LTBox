//! Root-wizard handler (family/mode/provider/version, confirm, exec). Extracted from `main.rs`.

use crate::*;
use iced::Task;
use ltbox_core::tr_args;

impl App {
    /// What to call this root run in the log.
    ///
    /// Family alone does not identify the route: Magisk and Magisk forks are
    /// both the Magisk family but take different pipelines, so a report saying
    /// only "Magisk" left the actual route to guesswork. The provider is set
    /// exactly on the routes where it means something — GKI and SKRoot never
    /// reach a provider picker — so append it whenever there is one.
    fn root_route_label(&self) -> String {
        let family = self
            .root
            .family
            .map(|f| self.t(f.label_key()).to_string())
            .unwrap_or_else(|| "?".to_string());
        match self.root.provider {
            Some(provider) => format!("{family} · {}", self.t(provider.label_key())),
            None => family,
        }
    }

    pub(crate) fn update_root(&mut self, msg: RootMsg) -> Task<Message> {
        match msg {
            RootMsg::RootFamily(f) => {
                self.root.release_tag = None;
                self.root.release_request = None;
                self.root.release_popup_open = false;
                if !ltbox_core::model::capabilities(&self.device.model).root {
                    return Task::none();
                }
                self.root.family = Some(f);
                self.root.provider = None;
                self.root.mode = None;
                self.root.skroot_flavor = None;
                self.root.version = None;
                self.root.nightly_source = None;
                self.root.file_path = None;
                self.root.kernel_version = None;
                self.root.run_id = None;
                self.root.run_id_buffer.clear();
                Task::none()
            }
            RootMsg::RootProvider(p) => {
                self.root.release_tag = None;
                self.root.release_request = None;
                self.root.release_popup_open = false;
                self.root.provider = Some(p);
                self.root.file_path = None;
                // ReSukiSU has no Stable channel — if the user had Stable
                // picked before switching to ReSukiSU, force Nightly so the
                // hidden-Stable version step lands on the sole valid choice
                // instead of showing an orphan "no selection" state.
                if p == Provider::ReSukiSU && self.root.version == Some(VerChoice::Stable) {
                    self.root.version = Some(VerChoice::Nightly);
                    self.root.nightly_source = None;
                    self.root.run_id = None;
                    self.root.run_id_buffer.clear();
                }
                Task::none()
            }
            RootMsg::RootMode(m) => {
                if !ltbox_core::model::capabilities(&self.device.model).root {
                    return Task::none();
                }
                // TODO(root): Direct GKI installation from stock has a reported
                // TB323FU boot failure. Offline repack parity alone does not
                // establish the required GBL/init_boot state; keep this gated
                // until the device installation path is verified.
                if !ltbox_core::model::capabilities(&self.device.model).gki_root
                    && m == RootMode::Gki
                {
                    return Task::none();
                }
                self.root.mode = Some(m);
                self.root.file_path = None;
                self.root.kernel_version = None;
                Task::none()
            }
            RootMsg::RootSkrootFlavor(flavor) => {
                // Pro stays visible in the UI to set expectations, but
                // stale messages must not advance the wizard into a route
                // with no patch implementation.
                if flavor == SkrootFlavor::Pro {
                    return Task::none();
                }
                self.root.skroot_flavor = Some(flavor);
                Task::none()
            }
            RootMsg::RootVersion(v) => {
                self.root.release_tag = None;
                self.root.release_request = None;
                self.root.release_popup_open = false;
                self.root.version = Some(v);
                self.root.nightly_source = None;
                self.root.run_id = None;
                self.root.run_id_buffer.clear();
                Task::none()
            }
            RootMsg::RootNightlySource(s) => {
                self.root.nightly_source = Some(s);
                match s {
                    NightlySource::AutoDetect => {
                        // Leaving ManualInput — drop the committed run ID.
                        self.root.run_id = None;
                        self.root.run_id_buffer.clear();
                    }
                    NightlySource::ManualInput => {
                        // Prefill from any previous commit so re-entry is painless.
                        self.root.run_id_buffer = self.root.run_id.clone().unwrap_or_default();
                        self.root.run_id_popup_open = true;
                    }
                }
                Task::none()
            }
            RootMsg::RootSelectFile => {
                self.picker_target = PickerTarget::RootFile;
                let spec = if self.root.is_gki() {
                    // GKI route accepts both an AnyKernel3 zip and a raw
                    // boot.img — the patcher branches on the extension
                    // (`gki::patch_boot` unpacks the .img with
                    // magiskboot in a scratch subdir to pull the kernel
                    // out, then reuses the existing repack path).
                    pickers::FilePickSpec::single().with_filter("Kernel image", &["zip", "img"])
                } else {
                    pickers::FilePickSpec::single().with_filter("APK", &["apk"])
                };
                pickers::pick_file_for(spec, &self.recent_paths, Message::FileSelected)
            }
            RootMsg::RootSelectFolder => {
                // Historical field name; value is now a single EDL loader file.
                // A fitting Settings default loader bypasses the picker; a
                // model-mismatched (or missing) one falls through to it.
                self.pick_loader_with_default(|__v| Message::Root(RootMsg::RootLoaderChosen(__v)))
            }
            RootMsg::RootLoaderChosen(path) => {
                self.apply_loader_pick(path, |app, loader, err| {
                    app.root.folder_path = loader;
                    app.error_msg = err;
                });
                Task::none()
            }
            RootMsg::RootNext => {
                if self.root.step == 3
                    && self.root.version == Some(VerChoice::Stable)
                    && !self.root.is_forks()
                    && !self.root.is_gki()
                {
                    let Some(provider) = self.root.provider else {
                        return Task::none();
                    };
                    use ltbox_patch::root_pipeline::{RootProvider, provider_repo};
                    let provider = match provider {
                        Provider::Magisk => RootProvider::Magisk,
                        Provider::MagiskForks => RootProvider::MagiskFork,
                        Provider::KernelSU => RootProvider::KernelSU,
                        Provider::KernelSUNext => RootProvider::KernelSUNext,
                        Provider::SukiSU => RootProvider::SukiSU,
                        Provider::ReSukiSU => RootProvider::ReSukiSU,
                        Provider::APatch => RootProvider::APatch,
                        Provider::FolkPatch => RootProvider::FolkPatch,
                    };
                    let Some(repo) = provider_repo(provider) else {
                        return Task::none();
                    };
                    let request = std::time::Instant::now();
                    self.root.release_popup_open = true;
                    self.root.release_request = Some(request);
                    self.root.releases.clear();
                    self.root.release_selection = None;
                    self.root.release_error = None;
                    return task_heavy(
                        move || {
                            ltbox_core::github::GitHubClient::new(repo)
                                .and_then(|client| client.recent_published_releases())
                                .map_err(|e| e.to_string())
                        },
                        move |result| Message::Root(RootMsg::RootReleasesLoaded(request, result)),
                        |e| Err(e.to_string()),
                    );
                }
                if self.root.step == 6 {
                    if self.root.needs_ksu_lkm_kernel_version() {
                        // Reserve the global busy op *before* the blocking
                        // ADB probe so concurrent flash/root/reboot starts
                        // cannot race it, and so a late
                        // `RootKernelVersionProbeDone` cannot auto-launch
                        // over a different live operation. Use silent busy
                        // (empty `op_steps`) so the probe is distinguishable
                        // from a phased Root exec that already owns `busy`.
                        if self.operation.is_running() {
                            return Task::none();
                        }
                        self.begin_silent_op(View::Root);
                        // ADB probe is blocking — push to the heavy pool so
                        // the UI doesn't freeze on a slow / unresponsive
                        // device. Continuation lands in
                        // `RootKernelVersionProbeDone`.
                        return task_heavy(
                            || {
                                ltbox_device::adb::AdbManager::new_if_connected().and_then(
                                    |mut adb| {
                                        adb.get_kernel_version().ok().flatten().and_then(|kv| {
                                        ltbox_patch::root_pipeline::normalize_ksu_kernel_version(
                                            &kv,
                                        )
                                    })
                                    },
                                )
                            },
                            |__v| Message::Root(RootMsg::RootKernelVersionProbeDone(__v)),
                            |_e| None,
                        );
                    }
                    if self.operation.is_running() {
                        return Task::none();
                    }
                    self.root.next();
                    return self.update(Message::Root(RootMsg::RootExecStart));
                }
                // APatch KPM step: advance only after superkey confirmation.
                if self.root.step == 8 {
                    self.root.superkey_buffer.clear();
                    self.root.superkey_first_entry = None;
                    self.root.superkey_popup_open = true;
                    return Task::none();
                }
                self.root.next();
                // Skip loader step when a valid Settings default exists.
                if self.root.step == 5
                    && self.root.folder_path.is_none()
                    && let Some(path) = self.resolved_default_loader()
                {
                    self.root.folder_path = Some(path);
                    self.root.next();
                }
                Task::none()
            }
            RootMsg::RootBack => {
                self.root.release_request = None;
                self.root.release_popup_open = false;
                self.root.back();
                Task::none()
            }
            RootMsg::RootReleasesLoaded(request, result) => {
                if self.root.release_request != Some(request) || !self.root.release_popup_open {
                    return Task::none();
                }
                self.root.release_request = None;
                match result {
                    Ok(releases) => {
                        self.root.release_selection = (!releases.is_empty()).then_some(0);
                        self.root.releases = releases;
                    }
                    Err(error) => self.root.release_error = Some(error),
                }
                Task::none()
            }
            RootMsg::RootReleaseSelect(index) => {
                if self.root.release_popup_open && index < self.root.releases.len() {
                    self.root.release_selection = Some(index);
                }
                Task::none()
            }
            RootMsg::RootReleaseCancel => {
                self.root.release_popup_open = false;
                self.root.release_request = None;
                Task::none()
            }
            RootMsg::RootReleaseConfirm => {
                if !self.root.release_popup_open
                    || self.root.step != 3
                    || self.root.version != Some(VerChoice::Stable)
                {
                    return Task::none();
                }
                let Some(release) = self
                    .root
                    .release_selection
                    .and_then(|i| self.root.releases.get(i))
                else {
                    return Task::none();
                };
                self.root.release_tag = Some(release.tag.clone());
                self.root.release_popup_open = false;
                self.root.next();
                if self.root.step == 5
                    && self.root.folder_path.is_none()
                    && let Some(path) = self.resolved_default_loader()
                {
                    self.root.folder_path = Some(path);
                    self.root.next();
                }
                Task::none()
            }
            RootMsg::RootSelectKpm => {
                // Multi-select; paths merge-dedup into the list so
                // the user can Browse multiple times.
                let spec = pickers::FilePickSpec::multi().with_filter("KPM modules", &["kpm"]);
                pickers::pick_files_for(spec, &self.recent_paths, |__v| {
                    Message::Root(RootMsg::RootKpmSelected(__v))
                })
            }
            RootMsg::RootKpmSelected(paths) => {
                if let Some(paths) = paths {
                    if let Some(first) = paths.first() {
                        self.remember_recent(pickers::PickerKind::File, first);
                    }
                    for p in paths {
                        if !self.root.kpm_paths.iter().any(|existing| existing == &p) {
                            self.root.kpm_paths.push(p);
                        }
                    }
                }
                Task::none()
            }
            RootMsg::RootKpmRemove(path) => {
                self.root.kpm_paths.retain(|p| p != &path);
                Task::none()
            }
            RootMsg::RootSuperkeyInput(text) => {
                self.root.superkey_buffer = text;
                Task::none()
            }
            RootMsg::RootSuperkeyConfirm => {
                let key = self.root.superkey_buffer.trim().to_string();
                match self.root.superkey_first_entry.take() {
                    None => {
                        // Stage 1 — first entry. Validate the format
                        // up-front so the user finds out about a too-short
                        // / non-alnum key on the first round, not after
                        // re-typing it. Upstream rule: 8–63 alphanumeric.
                        let valid = (8..=63).contains(&key.len())
                            && key.chars().all(|c| c.is_ascii_alphanumeric());
                        if !valid {
                            self.error_msg = Some(self.t("apatch_superkey_invalid").to_string());
                            return Task::none();
                        }
                        // Stash the validated first entry, blank the
                        // field, and stay open for the verification
                        // round. View flips to the "re-enter" prompt
                        // because `superkey_first_entry.is_some()`.
                        self.root.superkey_first_entry = Some(key);
                        self.root.superkey_buffer.clear();
                        self.error_msg = None;
                    }
                    Some(first) => {
                        // Stage 2 — verification entry. Mismatch resets
                        // the whole flow so the user types both rounds
                        // again from scratch (no "edit second field"
                        // shortcut, since the typo could be in either).
                        if key != first {
                            self.error_msg = Some(self.t("apatch_superkey_mismatch").to_string());
                            self.root.superkey_buffer.clear();
                            // `superkey_first_entry` already cleared by
                            // the `.take()` above — stage flips back to
                            // first-entry automatically.
                            return Task::none();
                        }
                        self.root.superkey = Some(key);
                        self.root.superkey_buffer.clear();
                        self.root.superkey_popup_open = false;
                        self.error_msg = None;
                        self.root.next();
                    }
                }
                Task::none()
            }
            RootMsg::RootSuperkeyCancel => {
                self.root.superkey_buffer.clear();
                self.root.superkey_first_entry = None;
                self.root.superkey_popup_open = false;
                self.error_msg = None;
                Task::none()
            }
            RootMsg::RootRunIdInput(text) => {
                // GH Actions run IDs are 10 digits; cap at 12 for headroom.
                let filtered: String = text
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .take(12)
                    .collect();
                self.root.run_id_buffer = filtered;
                Task::none()
            }
            RootMsg::RootRunIdConfirm => {
                let id = self.root.run_id_buffer.trim().to_string();
                if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
                    self.error_msg = Some(self.t("nightly_manual_invalid").to_string());
                    return Task::none();
                }
                self.root.run_id = Some(id);
                self.root.run_id_popup_open = false;
                self.error_msg = None;
                Task::none()
            }
            RootMsg::RootRunIdCancel => {
                self.root.run_id_buffer.clear();
                self.root.run_id_popup_open = false;
                // Roll back NightlySource so the step gate forces a re-pick.
                if self.root.run_id.is_none() {
                    self.root.nightly_source = None;
                }
                Task::none()
            }
            RootMsg::RootKernelVersionInput(text) => {
                let filtered: String = text
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '.')
                    .take(16)
                    .collect();
                self.root.kernel_version_buffer = filtered;
                Task::none()
            }
            RootMsg::RootKernelVersionConfirm => {
                let input = self.root.kernel_version_buffer.trim();
                let Some(kv) = ltbox_patch::root_pipeline::normalize_ksu_kernel_version(input)
                else {
                    self.error_msg = Some(self.t("root_kernel_version_invalid").to_string());
                    return Task::none();
                };
                self.root.kernel_version = Some(kv);
                self.root.kernel_version_buffer.clear();
                self.root.kernel_version_popup_open = false;
                self.error_msg = None;
                if self.root.step == 6 {
                    self.root.next();
                    return self.update(Message::Root(RootMsg::RootExecStart));
                }
                Task::none()
            }
            RootMsg::RootKernelVersionCancel => {
                self.root.kernel_version_buffer.clear();
                self.root.kernel_version_popup_open = false;
                Task::none()
            }
            RootMsg::RootKernelVersionProbeDone(detected) => {
                // Only continue while *this* probe still owns the silent
                // Root busy reservation. A phased Root exec (non-empty
                // `op_steps`), a cleared busy flag, or a different
                // `busy_view` means the result is stale — never auto-launch.
                let probe_still_ours = self.operation.is_running()
                    && self.operation.view() == Some(View::Root)
                    && self.operation.steps.is_empty();
                if !probe_still_ours {
                    return Task::none();
                }
                // Wizard may have moved off step 6 by the time the probe
                // returns (user clicked Back); release the reservation and
                // drop the result if we are no longer at the same gate.
                if self.root.step != 6 || !self.root.needs_ksu_lkm_kernel_version() {
                    self.end_silent_op();
                    return Task::none();
                }
                if let Some(kv) = detected {
                    self.root.kernel_version = Some(kv);
                    self.root.next();
                    // Drop the probe reservation *before* RootExecStart's
                    // begin_phased_op so the phased root op starts cleanly
                    // with no nested/overlapping busy ownership.
                    self.end_silent_op();
                    return self.update(Message::Root(RootMsg::RootExecStart));
                }
                self.end_silent_op();
                self.root.kernel_version_buffer =
                    self.root.kernel_version.clone().unwrap_or_default();
                self.root.kernel_version_popup_open = true;
                Task::none()
            }
            RootMsg::RootExecStart => {
                // Refuse to start while any busy op is live (including a
                // still-held KSU probe reservation). Callers that own the
                // probe path release it before re-entering here.
                if self.operation.is_running() {
                    return Task::none();
                }
                if !ltbox_core::model::capabilities(&self.device.model).root {
                    self.error_msg =
                        Some(tr_args!("model_unsupported", model = "TB376FC / TB390FU"));
                    return Task::none();
                }
                // Direct GKI installation on TB323FU still needs device
                // verification. Reject stale selections made before the model
                // was identified, as well as disabling the mode card.
                if !ltbox_core::model::capabilities(&self.device.model).gki_root
                    && self.root.is_gki()
                {
                    self.root.mode = None;
                    self.root.step = 1; // Mode step
                    self.error_msg = Some(tr_args!(
                        "model_unsupported",
                        model = self.device.model.as_str()
                    ));
                    return Task::none();
                }
                if self
                    .validate_loader_path(&self.root.folder_path.clone())
                    .is_err()
                {
                    return Task::none();
                }
                let phases = self.begin_phased_op(View::Root, OperationPhaseKind::Root);
                self.error_msg = None;
                let family = self.root.family;
                let mode = self.root.mode;
                let provider = self.root.provider;
                let version = self.root.version;
                let file_path = self.root.file_path.clone();
                let gui_kernel_version = self.root.kernel_version.clone();
                let device_model = self.device.model.clone();
                let conn = self.device.connection;
                // Folder must contain `xbl_s_devprg_ns.melf`; optional
                // `keys/testkey_rsa{2048,4096}.pem` as KEY_MAP fallback.
                let fw_folder = self.root.folder_path.clone();
                // APatch-only; empty / default elsewhere.
                let kpm_paths: Vec<std::path::PathBuf> = self
                    .root
                    .kpm_paths
                    .iter()
                    .map(std::path::PathBuf::from)
                    .collect();
                let superkey = self.root.superkey.clone().unwrap_or_default();
                let nightly_run_id: Option<u64> =
                    if self.root.nightly_source == Some(NightlySource::ManualInput) {
                        self.root.run_id.as_deref().and_then(|s| s.parse().ok())
                    } else {
                        None
                    };
                let release_tag = self.root.release_tag.clone();

                self.log_push(format!(
                    "[Root] {}",
                    tr_args!("log_op_starting", what = self.root_route_label())
                ));
                // Resolve Magisk preinit device via /proc/self/mountinfo
                // before ADB vanishes past EDL. Gates /data on the device's
                // encryption state — metadata-encrypted devices land preinit
                // on userdata otherwise and boot-loop after first wipe.
                let preinit_device: String = if matches!(family, Some(Family::Magisk))
                    && matches!(
                        self.device.connection,
                        ConnectionStatus::Adb | ConnectionStatus::AdbRecovery
                    ) {
                    let (mountinfo, encrypt_type) = if let Some(mut adb) =
                        ltbox_device::adb::AdbManager::new_if_connected()
                    {
                        let mi = adb.shell("cat /proc/self/mountinfo").unwrap_or_default();
                        let cs = adb.shell("getprop ro.crypto.state").unwrap_or_default();
                        let ct = adb.shell("getprop ro.crypto.type").unwrap_or_default();
                        let cme = adb
                            .shell("getprop ro.crypto.metadata.enabled")
                            .unwrap_or_default();
                        (
                            mi,
                            ltbox_patch::magisk::derive_encrypt_type(&cs, &ct, &cme).to_string(),
                        )
                    } else {
                        (String::new(), String::from("file"))
                    };
                    if mountinfo.is_empty() {
                        self.log_push(format!(
                            "[Magisk] {}",
                            ltbox_core::i18n::tr("log_magisk_preinit_adb_unavailable")
                        ));
                        String::new()
                    } else {
                        self.log_push(format!(
                            "[Magisk] {}",
                            tr_args!("log_magisk_crypto_state", encrypt_type = encrypt_type)
                        ));
                        match ltbox_patch::magisk::resolve_preinit_device(&mountinfo, &encrypt_type)
                        {
                            Some(name) => {
                                self.log_push(format!(
                                    "[Magisk] {}",
                                    tr_args!("log_magisk_preinit_resolved", name = name)
                                ));
                                name
                            }
                            None => {
                                self.log_push(format!(
                                    "[Magisk] {}",
                                    ltbox_core::i18n::tr("log_magisk_preinit_none")
                                ));
                                String::new()
                            }
                        }
                    }
                } else {
                    String::new()
                };
                let ll = self.live_labels();

                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            ltbox_core::runtime::run_heavy(move || {
                                root_worker(
                                    family,
                                    mode,
                                    provider,
                                    version,
                                    file_path,
                                    gui_kernel_version,
                                    device_model,
                                    conn,
                                    fw_folder,
                                    kpm_paths,
                                    superkey,
                                    nightly_run_id,
                                    release_tag,
                                    preinit_device,
                                    ll,
                                    phases,
                                )
                            })
                            .and_then(|r| r)
                        })
                        .await
                        .unwrap_or_else(|_| Err(ltbox_core::i18n::tr("err_task_failed")))
                    },
                    |result| match result {
                        Ok(lines) => Message::Root(RootMsg::RootExecDone(lines)),
                        Err(e) => Message::OperationError(e),
                    },
                )
            }
            RootMsg::RootExecDone(lines) => {
                self.flush_exec_done_log(lines);
                self.end_op();
                Task::none()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn stable_next_requires_a_published_release_and_pins_the_selection() {
        let mut app = App {
            root: RootWizard {
                step: 3,
                family: Some(Family::Magisk),
                mode: Some(RootMode::Lkm),
                provider: Some(Provider::Magisk),
                version: Some(VerChoice::Stable),
                ..RootWizard::default()
            },
            ..App::default()
        };

        let _ = app.update_root(RootMsg::RootNext);
        let request = app.root.release_request.expect("release request");
        assert!(app.root.release_popup_open);
        assert_eq!(app.root.step, 3);

        let _ = app.update_root(RootMsg::RootReleasesLoaded(
            request,
            Ok(vec![
                ltbox_core::github::PublishedRelease {
                    tag: "v2.0.0-rc1".into(),
                    prerelease: true,
                    published_at: "2026-09-01T00:00:00Z".into(),
                },
                ltbox_core::github::PublishedRelease {
                    tag: "v1.9.0".into(),
                    prerelease: false,
                    published_at: "2026-08-01T00:00:00Z".into(),
                },
            ]),
        ));
        assert_eq!(app.root.release_selection, Some(0));

        let _ = app.update_root(RootMsg::RootReleaseSelect(1));
        let _ = app.update_root(RootMsg::RootReleaseConfirm);
        assert_eq!(app.root.release_tag.as_deref(), Some("v1.9.0"));
        assert_eq!(app.root.step, 5);
        assert!(!app.root.release_popup_open);
    }

    #[test]
    fn unsupported_model_messages_cannot_start_operations() {
        for model in ["TB376FC", "TB390FU"] {
            let mut app = App::default();
            app.device.model = model.into();
            let _ = app.update_root(RootMsg::RootFamily(Family::Magisk));
            assert!(app.root.family.is_none());
            let _ = app.update_root(RootMsg::RootExecStart);
            assert!(!app.operation.is_running());
            assert_eq!(
                app.error_msg,
                Some(tr_args!("model_unsupported", model = "TB376FC / TB390FU"))
            );
            app.error_msg = None;
            let _ = app.update_unroot(UnrootMsg::UnrootExecStart);
            assert!(!app.operation.is_running());
            assert!(app.error_msg.is_some());
            app.error_msg = None;
            let _ = app.update_konabess(KonaBessMsg::KonaBessNext);
            assert!(!app.operation.is_running());
            assert_eq!(app.konabess.step, 0);
            assert!(app.error_msg.is_some());
        }
        let mut app = App::default();
        app.device.model = "TB323FU".into();
        let _ = app.update_root(RootMsg::RootMode(RootMode::Gki));
        assert!(app.root.mode.is_none());
        app.root.mode = Some(RootMode::Gki);
        let _ = app.update_root(RootMsg::RootExecStart);
        assert!(!app.operation.is_running());
        assert!(app.root.mode.is_none());
        assert_eq!(
            app.error_msg,
            Some(tr_args!("model_unsupported", model = "TB323FU"))
        );
    }

    fn ksu_lkm_confirm_wizard() -> RootWizard {
        RootWizard {
            family: Some(Family::KernelSU),
            mode: Some(RootMode::Lkm),
            provider: Some(Provider::KernelSU),
            version: Some(VerChoice::Stable),
            step: 6,
            ..RootWizard::default()
        }
    }

    #[test]
    fn root_next_reserves_silent_busy_before_ksu_probe() {
        let mut app = App {
            root: ksu_lkm_confirm_wizard(),
            ..App::default()
        };
        assert!(!app.operation.is_running());
        let _task = app.update_root(RootMsg::RootNext);
        assert!(
            app.operation.is_running(),
            "probe must reserve busy before blocking ADB work"
        );
        assert_eq!(app.operation.view(), Some(View::Root));
        assert!(
            app.operation.steps.is_empty(),
            "probe uses silent busy so it stays distinct from phased root"
        );
    }

    #[test]
    fn root_next_skips_ksu_probe_when_already_busy() {
        let mut app = App {
            operation: OperationExecution::fixture(true, Some(View::Flash), Vec::new(), 0, None),
            root: ksu_lkm_confirm_wizard(),
            ..App::default()
        };
        let _task = app.update_root(RootMsg::RootNext);
        assert!(app.operation.is_running());
        assert_eq!(
            app.operation.view(),
            Some(View::Flash),
            "must not steal another op's busy reservation for the probe"
        );
    }

    #[test]
    fn ksu_probe_done_ignores_when_busy_reservation_lost() {
        let mut app = App {
            operation: OperationExecution::fixture(false, None, Vec::new(), 0, None),
            root: ksu_lkm_confirm_wizard(),
            ..App::default()
        };
        let _task = app.update_root(RootMsg::RootKernelVersionProbeDone(Some("6.1".to_string())));
        assert!(!app.operation.is_running());
        assert!(app.root.kernel_version.is_none());
        assert_eq!(app.root.step, 6);
        assert!(!app.root.kernel_version_popup_open);
    }

    #[test]
    fn ksu_probe_done_ignores_phased_root_busy_overlap() {
        // A late probe callback must not clear or hijack a live phased root.
        let mut app = App {
            operation: OperationExecution::fixture(
                true,
                Some(View::Root),
                vec![
                    OpStep {
                        label: "Patch".to_string(),
                    },
                    OpStep {
                        label: "Flash".to_string(),
                    },
                ],
                0,
                None,
            ),
            root: {
                let mut w = ksu_lkm_confirm_wizard();
                w.step = 7;
                w.kernel_version = Some("6.1".to_string());
                w
            },
            ..App::default()
        };
        let _task = app.update_root(RootMsg::RootKernelVersionProbeDone(Some("6.6".to_string())));
        assert!(app.operation.is_running());
        assert_eq!(app.operation.view(), Some(View::Root));
        assert_eq!(app.operation.steps.len(), 2);
        assert_eq!(app.root.kernel_version.as_deref(), Some("6.1"));
        assert_eq!(app.root.step, 7);
    }

    #[test]
    fn ksu_probe_done_releases_busy_when_wizard_left_gate() {
        let mut app = App {
            operation: OperationExecution::fixture(true, Some(View::Root), Vec::new(), 0, None),
            root: {
                let mut w = ksu_lkm_confirm_wizard();
                w.step = 5; // user backed out during probe
                w
            },
            ..App::default()
        };
        let _task = app.update_root(RootMsg::RootKernelVersionProbeDone(Some("6.1".to_string())));
        assert!(!app.operation.is_running());
        assert_eq!(app.operation.view(), None);
        assert!(app.root.kernel_version.is_none());
        assert_eq!(app.root.step, 5);
    }

    #[test]
    fn ksu_probe_done_opens_manual_popup_and_releases_busy() {
        let mut app = App {
            operation: OperationExecution::fixture(true, Some(View::Root), Vec::new(), 0, None),
            root: ksu_lkm_confirm_wizard(),
            ..App::default()
        };
        let _task = app.update_root(RootMsg::RootKernelVersionProbeDone(None));
        assert!(!app.operation.is_running());
        assert_eq!(app.operation.view(), None);
        assert!(app.root.kernel_version_popup_open);
        assert!(app.root.kernel_version.is_none());
        assert_eq!(app.root.step, 6);
    }

    #[test]
    fn ksu_probe_done_detected_releases_probe_busy_before_exec() {
        // Missing loader makes RootExecStart fail before begin_phased_op —
        // proves the probe reservation is released and no nested busy remains.
        let mut app = App {
            operation: OperationExecution::fixture(true, Some(View::Root), Vec::new(), 0, None),
            root: {
                let mut w = ksu_lkm_confirm_wizard();
                w.folder_path = None;
                w
            },
            ..App::default()
        };
        let _task = app.update_root(RootMsg::RootKernelVersionProbeDone(Some("6.1".to_string())));
        assert_eq!(app.root.kernel_version.as_deref(), Some("6.1"));
        assert_eq!(app.root.step, 7);
        assert!(!app.operation.is_running());
        assert_eq!(app.operation.view(), None);
        assert!(app.error_msg.is_some());
    }

    #[test]
    fn root_exec_start_refuses_while_busy() {
        let mut app = App {
            operation: OperationExecution::fixture(true, Some(View::Flash), Vec::new(), 0, None),
            root: {
                let mut w = ksu_lkm_confirm_wizard();
                w.kernel_version = Some("6.1".to_string());
                w.folder_path = Some("C:/no/such/loader.melf".to_string());
                w.step = 7;
                w
            },
            ..App::default()
        };
        let _task = app.update_root(RootMsg::RootExecStart);
        assert!(app.operation.is_running());
        assert_eq!(app.operation.view(), Some(View::Flash));
        // Must not have started a root op or clobbered the foreign reservation.
        assert!(app.operation.steps.is_empty());
    }

    #[test]
    fn root_route_label_separates_the_two_magisk_routes() {
        let label = |provider| {
            App {
                root: RootWizard {
                    family: Some(Family::Magisk),
                    provider,
                    ..RootWizard::default()
                },
                ..App::default()
            }
            .root_route_label()
        };
        // The pair that made #88 unreproducible from the reported log.
        assert_ne!(
            label(Some(Provider::Magisk)),
            label(Some(Provider::MagiskForks))
        );
        assert!(
            label(Some(Provider::MagiskForks))
                .contains(ltbox_core::i18n::tr("provider_magisk_forks").as_str())
        );
        // GKI and SKRoot never reach a provider picker; family stands alone.
        assert_eq!(label(None), ltbox_core::i18n::tr("family_magisk"));
    }
}
