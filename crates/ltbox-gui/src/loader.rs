//! EDL loader discovery + validation helpers.

/// File-dialog / recent-chip extension filter for the EDL loader picker:
/// a stock `.melf` loader or a plaintext/encrypted Sahara manifest.
pub(crate) const LOADER_PICKER_EXTS: &[&str] = &["melf", "xml", "x"];

/// Extensions the loader picker offers: `.xml`/`.x` for manifest models, `.melf` for
/// other known models, all of [`LOADER_PICKER_EXTS`] when the model is unknown.
pub(crate) fn loader_picker_extensions(
    model_known: bool,
    manifest: bool,
) -> &'static [&'static str] {
    if manifest {
        &["xml", "x"]
    } else if model_known {
        &["melf"]
    } else {
        LOADER_PICKER_EXTS
    }
}

/// Locate the multi-image Sahara manifest in `dir`, case-insensitively. Prefers the
/// plaintext `qsahara_device_programmer.xml`, else the encrypted `.x` form (which
/// [`ltbox_device::edl::EdlSession::open`] decrypts at load time); `None` when neither
/// is present.
///
/// This only *locates*: it never decrypts or writes, so cheap UI gates (`can_next`)
/// can call it without side effects.
pub(crate) fn resolve_sahara_manifest(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let (mut plaintext, mut encrypted) = (None, None);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if ltbox_core::sahara_xml::is_manifest_filename(&p) {
                plaintext = Some(p);
            } else if ltbox_core::sahara_xml::is_encrypted_manifest_filename(&p) {
                encrypted = Some(p);
            }
        }
    }
    plaintext.or(encrypted)
}

/// Locate the EDL loader inside `dir`: the multi-image Sahara manifest
/// (plaintext `.xml` or encrypted `.x`) takes precedence over a single
/// `xbl_s_devprg_ns.melf`, since on a manifest device a stray `.melf` is
/// the wrong loader. Returns the path only — decryption of a `.x` manifest
/// happens in `EdlSession::open`.
pub(crate) fn find_edl_loader(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    if let Some(manifest) = resolve_sahara_manifest(dir) {
        return Some(manifest);
    }
    let candidate = dir.join("xbl_s_devprg_ns.melf");
    if candidate.exists() {
        return Some(candidate);
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("xbl_s_devprg_ns.melf")
            {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Locate the EDL loader beside a selected firmware directory or at the
/// firmware package root. Extracted firmware commonly keeps rawprogram XMLs
/// under `image/` and the loader one directory above it.
pub(crate) fn find_firmware_loader(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    find_edl_loader(dir).or_else(|| dir.parent().and_then(find_edl_loader))
}

/// Redirect a picked firmware folder to its `image/` subfolder when that is the
/// flashable one.
///
/// An extracted firmware archive is `<ROOT>/image/{rawprogram*.xml, images, loader}`
/// and LTBox flashes `image`, not `<ROOT>`, but users often pick `<ROOT>`. When the
/// selection has an immediate child directory named `image` (matched
/// case-insensitively) that ships a rawprogram pack, retarget to it. Returns `None`
/// (keep the selection) when the picked folder is itself flashable, so a valid folder
/// that merely contains an `image/` child is never hijacked, or when no `image` child
/// holds a pack.
pub(crate) fn redirect_to_image_subdir(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    // Already flashable → keep it, even if it also has an `image/` child.
    if dir_has_rawprogram_pack(dir) {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case("image")
            && entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
            && dir_has_rawprogram_pack(&entry.path())
        {
            return Some(entry.path());
        }
    }
    None
}

/// [`redirect_to_image_subdir`] as a string-in/string-out convenience for the
/// folder-pick handlers: returns the `image/` child path when a redirect
/// applies, otherwise the original `path` unchanged.
pub(crate) fn redirect_str(path: String) -> String {
    redirect_to_image_subdir(std::path::Path::new(&path))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(path)
}

/// True when `dir` directly contains a flashable rawprogram pack: a
/// `rawprogram*.xml` or its encrypted `rawprogram*.x` form. Matches what the
/// worker actually *collects* (`rawprogram*.xml`) — not every `.xml`/`.x`, so
/// a bare loader/manifest (`qsahara_device_programmer.xml`) in the extracted
/// root doesn't mask a flashable `image/` child.
fn dir_has_rawprogram_pack(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten().any(|e| {
                let path = e.path();
                let is_rawprogram = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_ascii_lowercase().starts_with("rawprogram"))
                    .unwrap_or(false);
                let ext_ok = path
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("x") || x.eq_ignore_ascii_case("xml"))
                    .unwrap_or(false);
                is_rawprogram && ext_ok
            })
        })
        .unwrap_or(false)
}

/// Key a model name is remembered under: trimmed and upper-cased, so the
/// same device identifies itself consistently however a given connection
/// state spells its model. `None` for a blank model — nothing to remember
/// against, and nothing to look up.
pub(crate) fn model_memory_key(model: &str) -> Option<String> {
    let trimmed = model.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_ascii_uppercase())
}

/// Bring a picked Sahara manifest to its plaintext form, decrypting an encrypted `.x`
/// pick to the sibling `qsahara_device_programmer.xml` **at pick time** rather than in
/// `EdlSession::open`.
///
/// The completeness gate below needs the `<image>` list, which is readable only once
/// the container is open. Non-manifest picks (`.melf` and friends) pass through
/// untouched.
pub(crate) fn decrypt_manifest_pick(
    path: &std::path::Path,
) -> std::result::Result<std::path::PathBuf, String> {
    if !ltbox_core::sahara_xml::is_encrypted_manifest_filename(path) {
        return Ok(path.to_path_buf());
    }
    let out = path.with_file_name(ltbox_core::sahara_xml::MANIFEST_FILENAME);
    ltbox_core::crypto::decrypt_file(path, &out).map_err(|e| {
        ltbox_core::tr_args!(
            "err_decrypt_file_failed",
            path = path.display(),
            error = e.to_string()
        )
    })?;
    Ok(out)
}

/// Reject a Sahara manifest whose referenced payloads are not all sitting beside it.
///
/// A manifest names several ELF/MBN images by relative path; picked from a
/// partially-extracted pack, each read would fail inside `EdlSession::open`, after the
/// device is already forced into EDL. Checking at pick time keeps the failure in the
/// file dialog, where it is recoverable.
///
/// `path` must already be plaintext (see [`decrypt_manifest_pick`]). Anything
/// that is not a manifest is accepted unchanged.
pub(crate) fn ensure_manifest_images_present(
    path: &std::path::Path,
) -> std::result::Result<(), String> {
    if !ltbox_core::sahara_xml::is_manifest_filename(path) {
        return Ok(());
    }
    let missing = ltbox_core::sahara_xml::missing_image_files(path).map_err(|e| {
        ltbox_core::tr_args!("err_loader_manifest_unreadable", error = e.to_string())
    })?;
    if missing.is_empty() {
        return Ok(());
    }
    let names = missing
        .iter()
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string())
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(ltbox_core::tr_args!(
        "err_loader_manifest_images_missing",
        count = missing.len(),
        files = names
    ))
}

/// [`decrypt_manifest_pick`] + [`ensure_manifest_images_present`]: the whole
/// pick-time manifest gate, returning the path a wizard should store.
pub(crate) fn prepare_manifest_pick(
    path: &std::path::Path,
) -> std::result::Result<std::path::PathBuf, String> {
    let resolved = decrypt_manifest_pick(path)?;
    ensure_manifest_images_present(&resolved)?;
    Ok(resolved)
}

pub(crate) fn is_loader_file(path: &std::path::Path) -> bool {
    // `.xml` covers the multi-image Sahara manifest (`qsahara_device_programmer.xml`).
    // `EdlSession::open` branches on the manifest filename (case-insensitive), so any other
    // `.xml` would fail there with a parse error rather than take the single-loader path.
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "melf" | "mbn" | "elf" | "xml"
            )
        })
        .unwrap_or(false)
}

/// Whether `path`'s extension is a single-blob loader format (`.melf` / `.mbn` /
/// `.elf`). Used by the manifest-upgrade gate for `requires_sahara_manifest` models to
/// decide whether to look for a sibling manifest; `.xml` is excluded so a manifest
/// pick isn't re-resolved.
pub(crate) fn is_melf_loader(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "melf" | "mbn" | "elf"))
        .unwrap_or(false)
}

/// True when `path`'s extension is the EDL loader form the model needs: manifest-route
/// models (`uses_efisp_gbl_route`, i.e. `requires_sahara_manifest`) → `.xml` / `.x`;
/// every other model → `.melf`. Inspects only the file's own extension, not the images
/// a manifest references.
pub(crate) fn loader_ext_fits_model(uses_efisp_gbl_route: bool, path: &std::path::Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if uses_efisp_gbl_route {
        matches!(ext.as_deref(), Some("xml") | Some("x"))
    } else {
        ext.as_deref() == Some("melf")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_manifest_images_present, find_firmware_loader, loader_ext_fits_model,
        prepare_manifest_pick, redirect_to_image_subdir,
    };
    use std::path::Path;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"x").unwrap();
    }

    /// Error strings come from the locale tables; without a translator
    /// installed `tr` echoes the key, and every message assertion below
    /// would pass vacuously.
    fn with_english_messages() {
        crate::translations::install_core_translator(crate::Language::En);
    }

    fn write_manifest(dir: &Path, names: &[&str]) -> std::path::PathBuf {
        let images: String = names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("<image image_id=\"{}\" image_path=\"{n}\"/>", i + 13))
            .collect();
        let path = dir.join(ltbox_core::sahara_xml::MANIFEST_FILENAME);
        std::fs::write(
            &path,
            format!("<sahara_config><images>{images}</images></sahara_config>"),
        )
        .unwrap();
        path
    }

    #[test]
    fn manifest_pick_requires_every_referenced_image() {
        with_english_messages();
        let dir = tempfile::tempdir().unwrap();
        let manifest = write_manifest(dir.path(), &["prog_firehose_ddr.elf", "xbl_sc.elf"]);
        touch(dir.path(), "prog_firehose_ddr.elf");

        // One payload short → rejected, and the message names the absentee so
        // the user knows what to copy in.
        let err = ensure_manifest_images_present(&manifest).expect_err("incomplete pack");
        assert!(err.contains("xbl_sc.elf"), "{err}");
        assert!(!err.contains("prog_firehose_ddr.elf"), "{err}");

        touch(dir.path(), "xbl_sc.elf");
        assert!(ensure_manifest_images_present(&manifest).is_ok());
        assert_eq!(prepare_manifest_pick(&manifest).unwrap(), manifest);
    }

    #[test]
    fn manifest_pick_rejects_unparseable_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join(ltbox_core::sahara_xml::MANIFEST_FILENAME);
        std::fs::write(&manifest, b"<not-a-sahara-config/>").unwrap();
        assert!(prepare_manifest_pick(&manifest).is_err());
    }

    #[test]
    fn manifest_pick_leaves_single_blob_loaders_alone() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "xbl_s_devprg_ns.melf");
        let melf = dir.path().join("xbl_s_devprg_ns.melf");
        assert_eq!(prepare_manifest_pick(&melf).unwrap(), melf);
    }

    #[test]
    fn encrypted_manifest_pick_decrypts_before_judging_it() {
        // A `.x` pick must be opened at pick time, since the `<image>` list the completeness
        // gate reads lives inside it. With an undecryptable container the failure must be the
        // decrypt one, not "manifest is fine" and not a deferred error at flash time. (The
        // decrypt round trip is covered in `ltbox_core::crypto`.)
        with_english_messages();
        let dir = tempfile::tempdir().unwrap();
        let encrypted = dir
            .path()
            .join(ltbox_core::sahara_xml::ENCRYPTED_MANIFEST_FILENAME);
        std::fs::write(&encrypted, vec![0u8; 64]).unwrap();
        let err = prepare_manifest_pick(&encrypted).expect_err("undecryptable container");
        assert!(err.contains(&encrypted.display().to_string()), "{err}");
        assert!(
            !dir.path()
                .join(ltbox_core::sahara_xml::MANIFEST_FILENAME)
                .exists()
        );
    }

    #[test]
    fn image_subdir_redirect() {
        // Extracted ROOT (no pack) with an `image/` child that ships a pack
        // → redirect into it.
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image");
        std::fs::create_dir(&image).unwrap();
        touch(&image, "rawprogram0.xml");
        assert_eq!(redirect_to_image_subdir(root.path()), Some(image.clone()));

        // Case-insensitive child name, encrypted `.x` pack.
        let root2 = tempfile::tempdir().unwrap();
        let upper = root2.path().join("IMAGE");
        std::fs::create_dir(&upper).unwrap();
        touch(&upper, "rawprogram0.x");
        assert_eq!(redirect_to_image_subdir(root2.path()), Some(upper));

        // The `image` folder itself (it has the pack, no `image/image`)
        // → leave as-is.
        assert_eq!(redirect_to_image_subdir(&image), None);

        // Already-flashable ROOT that ALSO has an `image/` child → never
        // hijack the valid selection.
        let root3 = tempfile::tempdir().unwrap();
        touch(root3.path(), "rawprogram0.xml");
        let side = root3.path().join("image");
        std::fs::create_dir(&side).unwrap();
        touch(&side, "rawprogram0.xml");
        assert_eq!(redirect_to_image_subdir(root3.path()), None);

        // `image/` child with no pack → not a firmware folder, no redirect.
        let root4 = tempfile::tempdir().unwrap();
        std::fs::create_dir(root4.path().join("image")).unwrap();
        assert_eq!(redirect_to_image_subdir(root4.path()), None);

        // ROOT holds only a loader/manifest XML (not a rawprogram pack) plus a
        // flashable `image/` child → must still redirect (the loose any-XML
        // gate would wrongly keep the root here).
        let root6 = tempfile::tempdir().unwrap();
        touch(root6.path(), "qsahara_device_programmer.xml");
        let img6 = root6.path().join("image");
        std::fs::create_dir(&img6).unwrap();
        touch(&img6, "rawprogram0.xml");
        assert_eq!(redirect_to_image_subdir(root6.path()), Some(img6));

        // A plain file named `image` is not a directory → no redirect.
        let root5 = tempfile::tempdir().unwrap();
        touch(root5.path(), "image");
        assert_eq!(redirect_to_image_subdir(root5.path()), None);
    }

    #[test]
    fn loader_ext_fits_model_by_device() {
        // Manifest-route models need the .xml / .x manifest.
        assert!(loader_ext_fits_model(
            true,
            Path::new("x/qsahara_device_programmer.xml")
        ));
        assert!(loader_ext_fits_model(true, Path::new("x/qsahara.x")));
        assert!(!loader_ext_fits_model(true, Path::new("x/prog.melf")));
        // Every other model needs the .melf single-blob (not .mbn / .elf / .xml).
        assert!(loader_ext_fits_model(false, Path::new("x/prog.melf")));
        assert!(!loader_ext_fits_model(false, Path::new("x/qsahara.xml")));
        assert!(!loader_ext_fits_model(false, Path::new("x/prog.mbn")));
    }

    #[test]
    fn firmware_loader_falls_back_to_the_firmware_parent() {
        let root = tempfile::tempdir().unwrap();
        let firmware = root.path().join("image");
        std::fs::create_dir(&firmware).unwrap();
        touch(root.path(), "xbl_s_devprg_ns.melf");

        assert_eq!(
            find_firmware_loader(&firmware),
            Some(root.path().join("xbl_s_devprg_ns.melf"))
        );
    }

    #[test]
    fn firmware_loader_returns_none_when_neither_location_has_one() {
        let root = tempfile::tempdir().unwrap();
        let firmware = root.path().join("image");
        std::fs::create_dir(&firmware).unwrap();

        assert_eq!(find_firmware_loader(&firmware), None);
    }
}
#[cfg(test)]
mod picker_filter_tests {
    use super::loader_picker_extensions;
    use crate::pickers::path_matches_extensions;

    #[test]
    fn switching_models_filters_shared_history_and_excludes_bootloader_paths() {
        let history = [
            "C:/Temp/session/abl.elf",
            "D:/Downloads/abl.ELF",
            "D:/Firmware/loader.MeLf",
            "D:/Firmware/manifest.XML",
            "D:/Firmware/qsahara_device_programmer.X",
            "D:/Firmware/payload.mbn",
        ];
        let visible = |known, manifest| {
            history
                .into_iter()
                .filter(|path| {
                    path_matches_extensions(path, loader_picker_extensions(known, manifest))
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            visible(false, false),
            vec![history[2], history[3], history[4]]
        );
        assert_eq!(visible(true, false), vec![history[2]]);
        assert_eq!(visible(true, true), vec![history[3], history[4]]);
    }
}
