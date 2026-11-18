//! EDL loader picking, validation and per-model memory.

use crate::*;

impl App {
    /// Open the EDL loader picker, filtered to the forms the connected model
    /// accepts.
    ///
    /// Always shows the dialog, even with a remembered loader: reaching a
    /// loader step a remembered loader would have skipped means the user
    /// walked Back to choose a different file.
    pub(crate) fn pick_loader<F>(&mut self, on_chosen: F) -> Task<Message>
    where
        F: 'static + Send + Fn(Option<String>) -> Message,
    {
        pickers::pick_file_for(self.model_loader_file_spec(), &self.recent_paths, on_chosen)
    }

    /// The remembered EDL loader for the connected model, when still usable;
    /// wizards use it to fill in and skip their loader step.
    ///
    /// `None` (so the picker shows) when the feature is off, no model is
    /// identified, the model has no entry, or the remembered file moved, no
    /// longer fits the model, or lost its manifest payloads. Silent by design:
    /// the path was learned, not configured, so a stale entry just falls back
    /// to the picker.
    pub(crate) fn remembered_loader_for_model(&self) -> Option<String> {
        if !self.remember_edl_loader {
            return None;
        }
        let stored = self
            .remembered_edl_loaders
            .get(&model_memory_key(self.device.model.as_str())?)?;
        let path = std::path::Path::new(stored);
        if !path.is_file() || !self.loader_fits_model(path) {
            return None;
        }
        // Same gate as the picker: a manifest that lost its payloads falls
        // back to the picker instead of skipping onto a loader that cannot load.
        prepare_manifest_pick(path)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// Note which model the operation about to start belongs to, and discard
    /// any unread upload record from the previous one.
    ///
    /// Captured now because `device.model` can be blank or reset once the
    /// device re-enumerates into EDL.
    pub(crate) fn arm_loader_memory(&mut self) {
        ltbox_device::edl::clear_uploaded_loader();
        self.loader_memory_model = model_memory_key(self.device.model.as_str());
    }

    /// Record a loader that just completed its Sahara upload against the model
    /// the running operation was armed with. No-op when the feature is off,
    /// when no model was known, or when the entry is already what we'd write.
    pub(crate) fn remember_uploaded_loader(&mut self, loader: &std::path::Path) {
        if !self.remember_edl_loader {
            return;
        }
        let Some(model) = self.loader_memory_model.clone() else {
            return;
        };
        let path = loader.to_string_lossy().into_owned();
        if self.remembered_edl_loaders.get(&model) == Some(&path) {
            return;
        }
        self.remembered_edl_loaders.insert(model, path);
        self.persist_settings();
    }

    /// Drain a completed loader upload published by the EDL layer; called from
    /// the tick that drains worker log output.
    pub(crate) fn drain_uploaded_loader(&mut self) {
        if let Some(loader) = ltbox_device::edl::take_uploaded_loader() {
            self.remember_uploaded_loader(&loader);
        }
    }

    /// Apply the remembered loader to the open advanced wizard's loader step.
    /// Pre-fills `loader_path`, then either advances to the Select step
    /// (DumpPhys / FlashPhys, no scan) or starts the GPT scan (FlashParts /
    /// DumpParts, whose Select step needs populated rows). Called from
    /// `AdvConfirm` after a wizard's `_open` flag flips.
    ///
    /// Returns `Task::none()` when there is no usable remembered loader, so
    /// the loader step shows as usual.
    pub(crate) fn apply_remembered_loader_to_advanced_wizard(&mut self) -> Task<Message> {
        let Some(path) = self.remembered_loader_for_model() else {
            return Task::none();
        };
        if self.advanced_wizard_open.is_flash_parts() {
            // Stay at step 0: FlashPartsScanDone advances to Select, so
            // advancing here would skip past it.
            self.flash_parts.loader_path = Some(path);
            return self.update(Message::FlashParts(FlashPartsMsg::FlashPartsScanStart));
        } else if self.advanced_wizard_open.is_dump_parts() {
            self.dump_parts.loader_path = Some(path);
            return self.update(Message::DumpParts(DumpPartsMsg::DumpPartsScanStart));
        } else if self.advanced_wizard_open.is_dump_phys() {
            // Whole-LUN: no scan, go straight to Select.
            self.dump_phys.loader_path = Some(path);
            self.dump_phys.step = 1;
        } else if self.advanced_wizard_open.is_flash_phys() {
            self.flash_phys.loader_path = Some(path);
            self.flash_phys.step = 1;
        }
        Task::none()
    }

    /// Pre-fill the KonaBess wizard from the model's remembered EDL loader.
    pub(crate) fn apply_remembered_loader_to_konabess(&mut self) {
        let Some(path) = self.remembered_loader_for_model() else {
            return;
        };
        // The path is already fit-checked and gated; resolve only for the
        // recents bookkeeping a pick would do.
        if let Ok(loader) = self.resolve_loader_input(&path) {
            self.konabess.loader_path = Some(loader);
            self.konabess.loader_error = None;
            self.konabess.step = 0;
        }
    }

    /// Validate a picked/default EDL loader before device work starts.
    pub(crate) fn validate_loader_path(&mut self, path: &Option<String>) -> Result<String, ()> {
        let Some(p) = path.as_deref() else {
            self.error_msg = Some(self.t("err_loader_not_selected").to_string());
            return Err(());
        };
        let pb = std::path::Path::new(p);
        if !pb.is_file() {
            let msg = tr_args!("err_loader_missing", path = p);
            self.error_msg = Some(msg);
            return Err(());
        }
        // Backstop for loaders that skipped the picker's gate (recents,
        // restored state): decrypt a `.x` manifest and require its images
        // before the device is forced into EDL.
        let prepared = prepare_manifest_pick(pb).map_err(|msg| {
            self.error_msg = Some(msg);
        })?;
        Ok(prepared.to_string_lossy().into_owned())
    }

    /// Route a picked loader into one wizard's `(loader_path, loader_error)`
    /// pair. Cancelling changes nothing; a resolved pick clears the error; a
    /// rejected pick clears the path, so a wizard can never advance on the
    /// loader it just refused.
    pub(crate) fn apply_loader_pick<F>(&mut self, picked: Option<String>, set: F)
    where
        F: FnOnce(&mut Self, Option<String>, Option<String>),
    {
        let Some(p) = picked else { return };
        match self.resolve_loader_input(&p) {
            Ok(loader) => set(self, Some(loader), None),
            Err(msg) => set(self, None, Some(msg)),
        }
    }

    /// Resolve loader input from the unified picker path. The picker offers
    /// `.melf` or a Sahara `.xml`/`.x` manifest according to the connected
    /// model; a directory is also accepted, for older recents entries, and
    /// resolved via [`find_edl_loader`].
    pub(crate) fn resolve_loader_input(
        &mut self,
        selected_path: &str,
    ) -> std::result::Result<String, String> {
        let path = std::path::Path::new(selected_path);
        if path.is_file() {
            self.remember_recent(pickers::PickerKind::File, selected_path);
            // Manifest-route models (`requires_sahara_manifest`) cannot load a
            // lone `.melf`: swap in the `qsahara_device_programmer.xml` beside
            // it, or abort now rather than fail mid-Sahara. Done here so the
            // Confirm step shows the final path.
            if self.requires_sahara_manifest()
                && is_melf_loader(path)
                && let Some(parent) = path.parent()
            {
                if let Some(manifest) = resolve_sahara_manifest(parent) {
                    return prepare_manifest_pick(&manifest)
                        .map(|p| p.to_string_lossy().to_string());
                }
                return Err(tr_args!(
                    "err_efisp_loader_manifest_required",
                    model = self.device.model.as_str(),
                    path = path.display()
                ));
            }
            // Manifest picked directly (`.xml` or encrypted `.x`). A `.x` is
            // decrypted to its sibling `.xml` here, not in `EdlSession::open`,
            // so the completeness check can read the `<image>` list while still
            // in the picker.
            if ltbox_core::sahara_xml::is_encrypted_manifest_filename(path)
                || ltbox_core::sahara_xml::is_manifest_filename(path)
            {
                return prepare_manifest_pick(path).map(|p| p.to_string_lossy().to_string());
            }
            if is_loader_file(path) {
                return Ok(selected_path.to_string());
            }
            return Err(tr_args!(
                "err_unsupported_loader_file",
                path = selected_path
            ));
        }

        if path.is_dir() {
            self.remember_recent(pickers::PickerKind::LoaderFolder, selected_path);
            return find_edl_loader(path)
                .map(|p| p.to_string_lossy().to_string())
                .ok_or_else(|| tr_args!("err_loader_not_found_in_path", path = selected_path));
        }

        Err(tr_args!("err_path_missing", path = selected_path))
    }

    /// Whether the polled model's capabilities require the multi-image Sahara
    /// manifest (`qsahara_device_programmer.xml` plus the ELF / MBN payloads it
    /// references) instead of a single `.melf`. See [`Self::resolve_loader_input`].
    pub(crate) fn requires_sahara_manifest(&self) -> bool {
        self.model_capabilities().requires_sahara_manifest
    }

    /// True when `path`'s extension is the EDL loader form the connected device
    /// needs: manifest-route models load `.xml` / `.x`; other connected models
    /// load `.melf`. With no connected device, either loader form is valid.
    /// Inspects only the picked file's own extension, never the `.mbn` / `.elf`
    /// images a manifest references internally.
    pub(crate) fn loader_fits_model(&self, path: &std::path::Path) -> bool {
        if self.device.connection == ConnectionStatus::None {
            return loader_ext_fits_model(false, path) || loader_ext_fits_model(true, path);
        }
        loader_ext_fits_model(self.model_capabilities().requires_sahara_manifest, path)
    }

    pub(crate) fn loader_picker_exts(&self) -> &'static [&'static str] {
        loader::loader_picker_extensions(
            self.device.connection != ConnectionStatus::None && !self.device.model.is_empty(),
            self.device.connection != ConnectionStatus::None && self.requires_sahara_manifest(),
        )
    }

    pub(crate) fn model_loader_file_spec(&self) -> pickers::FilePickSpec {
        let exts = self.loader_picker_exts();
        pickers::FilePickSpec::single()
            .with_filter(format!("EDL loader ({})", exts.join(" / ")), exts)
    }
}
