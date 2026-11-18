//! `qsahara_device_programmer.xml` parser.
//!
//! Models with `requires_sahara_manifest` (TB323FU, TB324ZC) ship a
//! multi-image Sahara handoff: instead of a single `xbl_s_devprg_ns.melf`, the
//! programmer is split across several ELF / MBN payloads keyed by Sahara
//! image-id, and this XML maps each id to its file.
//!
//! Example:
//!
//! ```xml
//! <sahara_config>
//!   <chipset>kaanapali</chipset>
//!   <images>
//!     <image image_id="13" image_path="prog_firehose_ddr.elf"/>
//!     <image image_id="21" image_path="xbl_sc.elf"/>
//!     ...
//!   </images>
//! </sahara_config>
//! ```
//!
//! [`parse`] returns the entries in document order. [`load_image_slots`]
//! reads each file relative to the XML's directory into the slot array that
//! `qdl::sahara::sahara_run` expects.

use std::path::{Path, PathBuf};

use crate::error::{LtboxError, Result};

/// Filename Lenovo uses for the multi-image manifest.
pub const MANIFEST_FILENAME: &str = "qsahara_device_programmer.xml";

/// Encrypted form of [`MANIFEST_FILENAME`] in some Lenovo firmware packs (the
/// same `.x` container as the rawprogram XMLs); decrypt it with
/// [`crate::crypto::decrypt_file`] before use.
pub const ENCRYPTED_MANIFEST_FILENAME: &str = "qsahara_device_programmer.x";

/// One `<image>` entry from the manifest.
#[derive(Debug, Clone)]
pub struct SaharaImage {
    /// Sahara protocol image-id the device asks for during cmd-mode.
    pub image_id: u32,
    /// Filename relative to the XML's parent directory.
    pub image_path: String,
}

/// Whether `path` is named like [`MANIFEST_FILENAME`] (the only XML the EDL
/// loader picker accepts), ignoring ASCII case.
pub fn is_manifest_filename(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case(MANIFEST_FILENAME))
        .unwrap_or(false)
}

/// Whether `path` is named like [`ENCRYPTED_MANIFEST_FILENAME`], ignoring ASCII
/// case.
pub fn is_encrypted_manifest_filename(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case(ENCRYPTED_MANIFEST_FILENAME))
        .unwrap_or(false)
}

/// Parse a manifest body into entries in document order. Rejects a missing
/// `<images>`, an empty list and duplicate ids.
pub fn parse(xml: &str) -> Result<Vec<SaharaImage>> {
    let doc =
        crate::xml::parse(xml).map_err(|e| LtboxError::Other(format!("Sahara XML parse: {e}")))?;
    let images_node = doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name("images"))
        .ok_or_else(|| LtboxError::Other("Sahara XML: missing <images>".to_string()))?;
    let mut out = Vec::new();
    for child in images_node.children().filter(|n| n.has_tag_name("image")) {
        let image_id = child
            .attribute("image_id")
            .ok_or_else(|| LtboxError::Other("Sahara XML: <image> missing image_id".to_string()))?
            .parse::<u32>()
            .map_err(|e| LtboxError::Other(format!("Sahara XML image_id parse: {e}")))?;
        let image_path = child
            .attribute("image_path")
            .ok_or_else(|| LtboxError::Other("Sahara XML: <image> missing image_path".to_string()))?
            .to_string();
        // Each id is one slot in the upload table; a repeat would silently
        // replace the earlier file, so a malformed manifest must not load.
        if out
            .iter()
            .any(|seen: &SaharaImage| seen.image_id == image_id)
        {
            return Err(LtboxError::Other(format!(
                "Sahara XML: duplicate image_id {image_id}"
            )));
        }
        out.push(SaharaImage {
            image_id,
            image_path,
        });
    }
    if out.is_empty() {
        return Err(LtboxError::Other(
            "Sahara XML: <images> contained no <image> entries".to_string(),
        ));
    }
    Ok(out)
}

/// Image files the manifest at `xml_path` references that are not present
/// beside it, in document order; empty means every payload resolves.
///
/// Only stats the files, so it is cheap enough for a file-picker gate. An
/// unreadable or unparseable manifest is an `Err`, not "nothing missing".
/// Paths escaping the manifest's directory fail through
/// [`crate::safe_path::safe_join`], as at load time.
pub fn missing_image_files(xml_path: &Path) -> Result<Vec<PathBuf>> {
    let xml_body = crate::xml::read(xml_path)
        .map_err(|e| LtboxError::Other(format!("Sahara XML read: {e}")))?;
    let entries = parse(&xml_body)?;
    let parent = xml_path
        .parent()
        .ok_or_else(|| LtboxError::Other("Sahara XML has no parent directory".to_string()))?;
    let mut missing = Vec::new();
    for entry in &entries {
        let img_path = crate::safe_path::safe_join(parent, &entry.image_path)?;
        if !img_path.is_file() {
            missing.push(img_path);
        }
    }
    Ok(missing)
}

/// Slot array sized `max image-id + 1`, `Some(bytes)` at each populated id and
/// `None` elsewhere: the shape `qdl::sahara::sahara_run` expects.
pub type ImageSlots = Vec<Option<Vec<u8>>>;

/// Read the manifest at `xml_path` and each referenced image from its parent
/// directory. Returns `(slots, image_paths)` so callers can log both.
pub fn load_image_slots(xml_path: &Path) -> Result<(ImageSlots, Vec<PathBuf>)> {
    let xml_body = crate::xml::read(xml_path)
        .map_err(|e| LtboxError::Other(format!("Sahara XML read: {e}")))?;
    let entries = parse(&xml_body)?;
    let parent = xml_path
        .parent()
        .ok_or_else(|| LtboxError::Other("Sahara XML has no parent directory".to_string()))?;
    // `image_id` is a u32 from untrusted XML; without a cap, one entry could
    // make the slot vector allocate (u32::MAX + 1) * 24 bytes (~96 GiB). Real
    // Sahara image ids are small, so 256 leaves room for vendor extensions.
    const MAX_SAHARA_IMAGE_ID: u32 = 256;
    let raw_max_id = entries.iter().map(|e| e.image_id).max().unwrap_or(0);
    if raw_max_id > MAX_SAHARA_IMAGE_ID {
        return Err(LtboxError::Other(format!(
            "Sahara manifest image_id {raw_max_id} exceeds supported cap \
             {MAX_SAHARA_IMAGE_ID} ({}); manifest may be malformed",
            xml_path.display(),
        )));
    }
    let max_id = raw_max_id as usize;
    let mut slots: ImageSlots = vec![None; max_id + 1];
    let mut paths: Vec<PathBuf> = Vec::with_capacity(entries.len());
    for entry in &entries {
        let img_path = crate::safe_path::safe_join(parent, &entry.image_path)?;
        let bytes = std::fs::read(&img_path).map_err(|e| {
            LtboxError::Other(format!(
                "Sahara image read failed for id={} ({}): {e}",
                entry.image_id,
                img_path.display()
            ))
        })?;
        slots[entry.image_id as usize] = Some(bytes);
        paths.push(img_path);
    }
    Ok((slots, paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lenovo_kaanapali_manifest() {
        let xml = r#"<?xml version="1.0" ?>
<sahara_config>
    <chipset>kaanapali</chipset>
    <images>
        <image image_id="36" image_path="multi_image_qti.mbn"/>
        <image image_id="37" image_path="multi_image.mbn"/>
        <image image_id="21" image_path="xbl_sc.elf"/>
        <image image_id="13" image_path="prog_firehose_ddr.elf"/>
    </images>
</sahara_config>"#;
        let imgs = parse(xml).unwrap();
        assert_eq!(imgs.len(), 4);
        assert_eq!(imgs[0].image_id, 36);
        assert_eq!(imgs[0].image_path, "multi_image_qti.mbn");
        assert_eq!(imgs[3].image_id, 13);
    }

    #[test]
    fn parse_rejects_empty_images() {
        let xml = r#"<sahara_config><chipset>x</chipset><images/></sahara_config>"#;
        assert!(parse(xml).is_err());
    }

    #[test]
    fn parse_rejects_duplicate_image_ids() {
        let xml = r#"<sahara_config><images>
            <image image_id="13" image_path="a.elf"/>
            <image image_id="21" image_path="b.elf"/>
            <image image_id="13" image_path="c.elf"/>
        </images></sahara_config>"#;
        let error = parse(xml).unwrap_err().to_string();
        assert!(error.contains("duplicate image_id 13"), "{error}");
    }

    fn write_manifest(dir: &Path, names: &[&str]) -> PathBuf {
        let images: String = names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("<image image_id=\"{}\" image_path=\"{n}\"/>", i + 13))
            .collect();
        let path = dir.join(MANIFEST_FILENAME);
        std::fs::write(
            &path,
            format!("<sahara_config><images>{images}</images></sahara_config>"),
        )
        .unwrap();
        path
    }

    #[test]
    fn missing_image_files_reports_only_absent_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = write_manifest(dir.path(), &["prog_firehose_ddr.elf", "xbl_sc.elf"]);
        std::fs::write(dir.path().join("prog_firehose_ddr.elf"), b"elf").unwrap();

        let missing = missing_image_files(&manifest).unwrap();
        assert_eq!(missing, vec![dir.path().join("xbl_sc.elf")]);

        std::fs::write(dir.path().join("xbl_sc.elf"), b"elf").unwrap();
        assert!(missing_image_files(&manifest).unwrap().is_empty());
    }

    #[test]
    fn missing_image_files_errors_on_unparseable_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join(MANIFEST_FILENAME);
        std::fs::write(&manifest, b"not xml at all").unwrap();
        assert!(missing_image_files(&manifest).is_err());
    }

    #[test]
    fn manifest_filename_matches_case_insensitive() {
        assert!(is_manifest_filename(Path::new(
            "qsahara_device_programmer.xml"
        )));
        assert!(is_manifest_filename(Path::new(
            "QSAHARA_DEVICE_PROGRAMMER.XML"
        )));
        assert!(!is_manifest_filename(Path::new("foo.xml")));
        assert!(!is_manifest_filename(Path::new("xbl_s_devprg_ns.melf")));
    }

    #[test]
    fn encrypted_manifest_filename_matches_case_insensitive() {
        assert!(is_encrypted_manifest_filename(Path::new(
            "qsahara_device_programmer.x"
        )));
        assert!(is_encrypted_manifest_filename(Path::new(
            "QSAHARA_DEVICE_PROGRAMMER.X"
        )));
        // Plaintext and encrypted forms are mutually exclusive.
        assert!(!is_encrypted_manifest_filename(Path::new(
            "qsahara_device_programmer.xml"
        )));
        assert!(!is_manifest_filename(Path::new(
            "qsahara_device_programmer.x"
        )));
        assert!(!is_encrypted_manifest_filename(Path::new("foo.x")));
    }
}
