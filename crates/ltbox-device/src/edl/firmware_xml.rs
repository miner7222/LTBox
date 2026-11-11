//! Select which rawprogram/patch XMLs from a firmware package are flashed.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawprogramFamily {
    Other,
    Persist,
    Devinfo,
}

fn rawprogram_family(name_lower: &str) -> RawprogramFamily {
    match name_lower {
        "rawprogram_unsparse0.xml"
        | "rawprogram_unsparse0-half.xml"
        | "rawprogram_write_persist_unsparse0.xml"
        | "rawprogram_save_persist_unsparse0.xml"
        | "rawprogram_save_persist_ota_unsparse0.xml" => RawprogramFamily::Persist,
        "rawprogram4.xml" | "rawprogram4_write_devinfo.xml" | "rawprogram_unsparse4.xml" => {
            RawprogramFamily::Devinfo
        }
        _ => RawprogramFamily::Other,
    }
}

fn filename_rank(path: &Path, preferred: &[&str]) -> usize {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    preferred
        .iter()
        .position(|candidate| name == *candidate)
        .unwrap_or(preferred.len())
}

fn select_one_by_name_priority(mut paths: Vec<PathBuf>, preferred: &[&str]) -> Option<PathBuf> {
    paths.sort_by(|a, b| {
        filename_rank(a, preferred)
            .cmp(&filename_rank(b, preferred))
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });
    paths.into_iter().next()
}

fn select_persist_xml(paths: Vec<PathBuf>, allow_dp_filenames: bool) -> Option<PathBuf> {
    if allow_dp_filenames {
        select_one_by_name_priority(
            paths,
            &[
                "rawprogram_write_persist_unsparse0.xml",
                "rawprogram_unsparse0.xml",
                "rawprogram_save_persist_ota_unsparse0.xml",
                "rawprogram_save_persist_unsparse0.xml",
                "rawprogram_unsparse0-half.xml",
            ],
        )
    } else {
        select_one_by_name_priority(
            paths,
            &[
                "rawprogram_save_persist_ota_unsparse0.xml",
                "rawprogram_save_persist_unsparse0.xml",
                "rawprogram_unsparse0-half.xml",
                "rawprogram_unsparse0.xml",
                "rawprogram_write_persist_unsparse0.xml",
            ],
        )
    }
}

fn select_devinfo_xml(paths: Vec<PathBuf>, allow_dp_filenames: bool) -> Option<PathBuf> {
    if allow_dp_filenames {
        select_one_by_name_priority(
            paths,
            &[
                "rawprogram4_write_devinfo.xml",
                "rawprogram4.xml",
                "rawprogram_unsparse4.xml",
            ],
        )
    } else {
        select_one_by_name_priority(
            paths,
            &[
                "rawprogram4.xml",
                "rawprogram_unsparse4.xml",
                "rawprogram4_write_devinfo.xml",
            ],
        )
    }
}

fn validate_dp_filename_usage(raw_xmls: &[PathBuf], allow_dp_filenames: bool) -> Result<()> {
    for xml_path in raw_xmls {
        let xml_content = ltbox_core::xml::read(xml_path)?;
        let doc = ltbox_core::xml::parse(&xml_content).map_err(|e| {
            EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
        })?;
        let xml_dir = xml_path.parent().unwrap_or(Path::new("."));
        for node in doc.descendants() {
            if !node.tag_name().name().eq_ignore_ascii_case("program") {
                continue;
            }
            let filename = node.attribute("filename").unwrap_or("").trim();
            let lower = filename.to_ascii_lowercase();
            if lower != "persist.img" && lower != "devinfo.img" {
                continue;
            }
            if !allow_dp_filenames {
                return Err(EdlError::Session(format!(
                    "{} references {filename}, but devinfo/persist image flashing is disabled",
                    xml_path.display()
                )));
            }
            let image_path = ltbox_core::safe_path::safe_join(xml_dir, filename)
                .map_err(|e| EdlError::Session(e.to_string()))?;
            if !image_path.exists() {
                return Err(EdlError::Session(format!(
                    "{} references {filename}, but {} is missing",
                    xml_path.display(),
                    image_path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Collect `rawprogram*.xml` and `patch*.xml` from `dir` for firmware
/// flashing. Drops v2 filter targets (WIPE/BLANK variants,
/// `rawprogram0.xml` GPT programmer) and treats devinfo/persist XML
/// variants as mutually exclusive families.
///
/// `allow_dp_filenames=false` is the normal v3 firmware path: country-code
/// images are dumped, patched, and flashed explicitly after rawprogram
/// flashing, so a selected rawprogram must not reference `persist.img` or
/// `devinfo.img`.
pub fn collect_firmware_xmls_for_flash(
    dir: &Path,
    allow_dp_filenames: bool,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut raw_xmls = Vec::new();
    let mut patch_xmls = Vec::new();
    let mut persist_xmls = Vec::new();
    let mut devinfo_xmls = Vec::new();
    // Plain `rawprogram0.xml`, deferred: used as the LUN0 source only when no
    // persist/`_unsparse0` variant supersedes it (see below).
    let mut rawprogram0: Option<PathBuf> = None;
    let entries = std::fs::read_dir(dir)?;

    for entry in entries {
        let path = entry?.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let lower = name.to_lowercase();
        if !lower.ends_with(".xml") {
            continue;
        }
        if lower.starts_with("rawprogram") {
            // v2 skips WIPE / zero-GPT variants.
            if lower.contains("wipe_partitions") || lower.contains("blank_gpt") {
                continue;
            }
            if lower == "rawprogram0.xml" {
                // Lenovo stock ships a persist/`_unsparse0` LUN0 variant that
                // supersedes the plain `rawprogram0.xml`, so defer it and use it
                // only as a fallback when no such variant exists — ported ROMs
                // ship only `rawprogram0.xml`, with `super` (the OS) on LUN0, so
                // dropping it unconditionally left `super` unflashed.
                rawprogram0 = Some(path);
                continue;
            }
            match rawprogram_family(&lower) {
                RawprogramFamily::Other => raw_xmls.push(path),
                RawprogramFamily::Persist => persist_xmls.push(path),
                RawprogramFamily::Devinfo => devinfo_xmls.push(path),
            }
        } else if lower.starts_with("patch") {
            patch_xmls.push(path);
        }
    }

    // LUN0 source: prefer the persist/`_unsparse0` variant; otherwise fall back
    // to the plain `rawprogram0.xml`. Either way `flash_rawprogram_with_wipe`'s
    // per-label keep/wipe skip still preserves userdata/metadata.
    match select_persist_xml(persist_xmls, allow_dp_filenames) {
        Some(path) => raw_xmls.push(path),
        None => {
            if let Some(path) = rawprogram0 {
                raw_xmls.push(path);
            }
        }
    }
    if let Some(path) = select_devinfo_xml(devinfo_xmls, allow_dp_filenames) {
        raw_xmls.push(path);
    }

    raw_xmls.sort();
    patch_xmls.sort();
    validate_dp_filename_usage(&raw_xmls, allow_dp_filenames)?;
    Ok((raw_xmls, patch_xmls))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edl::test_support::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn collect_firmware_xmls_handles_mixed_case() {
        let fw = TempFirmwareDir::new();
        fw.write("rawprogram0_wipe_partitions.xml", "<data/>");
        fw.write("Rawprogram0.xml", "<data/>");
        fw.write("rawprogram_unsparse0.xml", "<data/>");
        let (raw, _) = collect_firmware_xmls_for_flash(fw.path(), false).unwrap();
        assert_eq!(xml_names(&raw), vec!["rawprogram_unsparse0.xml"]);
    }

    #[test]
    fn collect_firmware_xmls_chooses_safe_dp_variants_once() {
        let fw = TempFirmwareDir::new();
        fw.write("rawprogram1.xml", "<data/>");
        fw.write(
            "rawprogram_unsparse0.xml",
            r#"<data><program label="persist" filename="persist.img" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram_unsparse0-half.xml",
            r#"<data><program label="persist" filename="" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram4.xml",
            r#"<data><program label="devinfo" filename="" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram4_write_devinfo.xml",
            r#"<data><program label="devinfo" filename="devinfo.img" num_partition_sectors="1"/></data>"#,
        );
        fw.write("patch0.xml", "<data/>");

        let (raw, patch) =
            collect_firmware_xmls_for_flash(fw.path(), false).expect("collect safe XMLs");
        let names = xml_names(&raw);

        assert!(names.contains(&"rawprogram1.xml".to_string()));
        assert!(names.contains(&"rawprogram_unsparse0-half.xml".to_string()));
        assert!(names.contains(&"rawprogram4.xml".to_string()));
        assert!(!names.contains(&"rawprogram_unsparse0.xml".to_string()));
        assert!(!names.contains(&"rawprogram4_write_devinfo.xml".to_string()));
        assert_eq!(
            names.len(),
            names.iter().collect::<std::collections::HashSet<_>>().len()
        );
        assert_eq!(xml_names(&patch), vec!["patch0.xml".to_string()]);
    }

    #[test]
    fn collect_firmware_xmls_prefers_persistless_lun0_for_simple_flash() {
        // Simple Flash reuses `collect_firmware_xmls_for_flash(dir, false)`:
        // when a firmware ships both a persist-less LUN0 rawprogram
        // (save_persist, empty persist filename) and a persist-writing one
        // (write_persist), the persist-less variant must win and be the *only*
        // LUN0 rawprogram kept.
        let fw = TempFirmwareDir::new();
        fw.write(
            "rawprogram_save_persist_unsparse0.xml",
            r#"<data><program label="persist" filename="" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram_write_persist_unsparse0.xml",
            r#"<data><program label="persist" filename="persist.img" num_partition_sectors="1"/></data>"#,
        );

        let (raw, _) =
            collect_firmware_xmls_for_flash(fw.path(), false).expect("collect persist-less LUN0");
        let names = xml_names(&raw);

        assert!(names.contains(&"rawprogram_save_persist_unsparse0.xml".to_string()));
        assert!(!names.contains(&"rawprogram_write_persist_unsparse0.xml".to_string()));
        // Exactly one LUN0 rawprogram is kept.
        let lun0 = names
            .iter()
            .filter(|n| n.contains("persist_unsparse0"))
            .count();
        assert_eq!(lun0, 1);
    }

    #[test]
    fn collect_firmware_xmls_allows_dp_xmls_when_images_exist() {
        let fw = TempFirmwareDir::new();
        fw.write("rawprogram1.xml", "<data/>");
        fw.write(
            "rawprogram_save_persist_unsparse0.xml",
            r#"<data><program label="persist" filename="" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram_write_persist_unsparse0.xml",
            r#"<data><program label="persist" filename="persist.img" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram4.xml",
            r#"<data><program label="devinfo" filename="" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram4_write_devinfo.xml",
            r#"<data><program label="devinfo" filename="devinfo.img" num_partition_sectors="1"/></data>"#,
        );
        fw.write_bytes("persist.img", b"persist");
        fw.write_bytes("devinfo.img", b"devinfo");

        let (raw, _) = collect_firmware_xmls_for_flash(fw.path(), true).expect("collect DP XMLs");
        let names = xml_names(&raw);

        assert!(names.contains(&"rawprogram_write_persist_unsparse0.xml".to_string()));
        assert!(names.contains(&"rawprogram4_write_devinfo.xml".to_string()));
        assert!(!names.contains(&"rawprogram_save_persist_unsparse0.xml".to_string()));
        assert!(!names.contains(&"rawprogram4.xml".to_string()));
        assert_eq!(
            names.len(),
            names.iter().collect::<std::collections::HashSet<_>>().len()
        );
    }

    #[test]
    fn collect_firmware_xmls_rejects_disabled_dp_references() {
        let fw = TempFirmwareDir::new();
        fw.write(
            "rawprogram_unsparse0.xml",
            r#"<data><program label="persist" filename="persist.img" num_partition_sectors="1"/></data>"#,
        );

        let err = collect_firmware_xmls_for_flash(fw.path(), false)
            .expect_err("persist.img should be rejected");
        assert!(err.to_string().contains("persist.img"));
        assert!(err.to_string().contains("disabled"));
    }

    #[test]
    fn collect_firmware_xmls_rejects_allowed_dp_when_image_missing() {
        let fw = TempFirmwareDir::new();
        fw.write(
            "rawprogram_write_persist_unsparse0.xml",
            r#"<data><program label="persist" filename="persist.img" num_partition_sectors="1"/></data>"#,
        );

        let err = collect_firmware_xmls_for_flash(fw.path(), true)
            .expect_err("persist.img image should be required");
        assert!(err.to_string().contains("persist.img"));
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn real_firmware_xml_matrix_when_available() {
        let Some(dir) = std::env::var_os("LTBOX_REAL_FIRMWARE_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        if !dir.join("rawprogram_unsparse0-half.xml").exists() {
            return;
        }

        let (raw, patch) =
            collect_firmware_xmls_for_flash(&dir, false).expect("collect real safe XMLs");
        let names = xml_names(&raw);

        assert!(names.contains(&"rawprogram_unsparse0-half.xml".to_string()));
        assert!(!names.contains(&"rawprogram_unsparse0.xml".to_string()));
        assert!(names.contains(&"rawprogram4.xml".to_string()));
        assert_eq!(
            names.len(),
            names.iter().collect::<std::collections::HashSet<_>>().len()
        );
        assert!(!patch.is_empty());

        let wipe_plan = EdlSession::collect_wipe_erase_plan(&raw).expect("collect wipe plan");
        assert_eq!(
            wipe_plan
                .iter()
                .filter(|entry| entry.label == "metadata")
                .count(),
            7
        );
        assert_eq!(
            wipe_plan
                .iter()
                .filter(|entry| entry.label == "userdata")
                .count(),
            10
        );
        assert!(!wipe_plan.iter().any(|entry| entry.label == "frp"));
    }

    #[test]
    fn rawprogram0_is_lun0_fallback_only_without_persist_variant() {
        let base = std::env::temp_dir().join(format!(
            "ltbox_xmlsel_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let has = |raw: &[PathBuf], n: &str| raw.iter().any(|p| p.file_name().unwrap() == n);
        let prog = |lun: u32, file: &str| {
            format!(
                r#"<data><program physical_partition_number="{lun}" start_sector="0" filename="{file}"/></data>"#
            )
        };
        std::fs::write(base.join("rawprogram0.xml"), prog(0, "super.img")).unwrap();
        std::fs::write(base.join("rawprogram1.xml"), prog(1, "xbl.img")).unwrap();

        // No persist / `_unsparse0` variant → rawprogram0.xml is the LUN0 source
        // (the ported-ROM case where `super` would otherwise never flash).
        let (raw, _) = collect_firmware_xmls_for_flash(&base, false).unwrap();
        assert!(has(&raw, "rawprogram0.xml"));

        // A persist variant supersedes the plain rawprogram0.xml (Lenovo stock).
        std::fs::write(base.join("rawprogram_unsparse0.xml"), prog(0, "super.img")).unwrap();
        let (raw2, _) = collect_firmware_xmls_for_flash(&base, false).unwrap();
        assert!(!has(&raw2, "rawprogram0.xml"));
        assert!(has(&raw2, "rawprogram_unsparse0.xml"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
