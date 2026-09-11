//! EDL loader discovery + validation helpers, extracted from `main.rs`.

/// File-dialog / recent-chip extension filter for the EDL loader picker:
/// a stock `.melf` Firehose loader or the `.xml` Sahara manifest.
/// Resolver compatibility with encrypted/legacy inputs is separate.
pub(crate) const LOADER_PICKER_EXTS: &[&str] = &["melf", "xml"];

/// User-facing choices, independent of the resolver's legacy compatibility.
pub(crate) fn loader_picker_extensions(
    model_known: bool,
    manifest: bool,
) -> &'static [&'static str] {
    if manifest {
        &["xml"]
    } else if model_known {
        &["melf"]
    } else {
        LOADER_PICKER_EXTS
    }
}

/// Locate the multi-image Sahara manifest in `dir`, case-insensitively.
/// Prefers the plaintext `qsahara_device_programmer.xml`; otherwise returns
/// the encrypted `qsahara_device_programmer.x` form, which
/// [`ltbox_device::edl::EdlSession::open`] decrypts at load time. `None`
/// when neither is present.
///
/// This only *locates* — it never decrypts or writes — so it is safe to
/// call from cheap UI gates (`can_next`) without side effects.
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

/// Redirect a picked firmware folder to its `image/` subfolder when that is
/// the flashable one.
///
/// The extracted firmware archive is laid out as
/// `<ROOT>/image/{rawprogram*.xml, partition images, loader}`, but LTBox
/// flashes the `image` folder, not `<ROOT>`. Users frequently pick `<ROOT>`
/// and hit a "not a valid firmware image folder" error, so when the selection
/// has an immediate child directory named `image` (matched case-insensitively
/// for case-sensitive filesystems) that ships a rawprogram pack, retarget to
/// it. Returns `None` — leave the selection as-is — when the picked folder is
/// itself flashable (so a valid folder that merely also contains an `image/`
/// child is never hijacked) or when no `image` child holds a pack.
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

pub(crate) fn is_loader_file(path: &std::path::Path) -> bool {
    // `.xml` covers TB323FU's `qsahara_device_programmer.xml` multi-
    // image manifest. `EdlSession::open` branches on the manifest
    // filename (case-insensitive) — any other `.xml` file would fail
    // there with a parse error rather than silently picking up the
    // single-loader path.
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

/// Whether `path`'s extension is one of the single-blob loader formats
/// (`.melf` / `.mbn` / `.elf`). Used by the TB323FU manifest-upgrade
/// gate to decide whether to look for a sibling manifest — `.xml` is
/// excluded so a manifest selection isn't recursively re-resolved.
pub(crate) fn is_melf_loader(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "melf" | "mbn" | "elf"))
        .unwrap_or(false)
}

/// True when `path`'s extension is the EDL loader form the given model needs:
/// Efisp/GBL-route models → `.xml` / `.x` (Sahara manifest); every other model
/// → `.melf`.
/// Inspects only the file's own extension, not the images a manifest references.
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
    use super::{find_firmware_loader, loader_ext_fits_model, redirect_to_image_subdir};
    use std::path::Path;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"x").unwrap();
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
        // Efisp/GBL-route models need the .xml / .x manifest.
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
            "D:/Firmware/manifest.x",
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
        assert_eq!(visible(false, false), vec![history[2], history[3]]);
        assert_eq!(visible(true, false), vec![history[2]]);
        assert_eq!(visible(true, true), vec![history[3]]);
    }
}
