use std::path::{Path, PathBuf};

fn copy_model(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_model(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

fn decrypt_and_collect_xmls(dir: &Path, xmls: &mut Vec<PathBuf>) {
    let entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    for path in entries {
        if path.is_dir() {
            decrypt_and_collect_xmls(&path, xmls);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("x"))
        {
            let output = path.with_extension("xml");
            ltbox_core::crypto::decrypt_file(&path, &output)
                .unwrap_or_else(|e| panic!("decrypt {}: {e}", path.display()));
        }
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
        {
            xmls.push(path);
        }
    }
}

#[test]
#[ignore = "requires LTBOX_RAWPROGRAM_CORPUS=<dir of per-model rawprogram folders>"]
fn stock_rawprogram_luns_and_keep_data_selection() {
    let corpus = std::env::var_os("LTBOX_RAWPROGRAM_CORPUS")
        .expect("LTBOX_RAWPROGRAM_CORPUS must be set when running this ignored test");
    let mut models: Vec<_> = std::fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    models.sort();
    assert!(
        !models.is_empty(),
        "rawprogram corpus contains no model folders"
    );
    let mut mismatches = Vec::new();
    let mut model_count = 0;
    let mut program_count = 0;
    let mut selected_count = 0;
    for model in models {
        let name = model.file_name().unwrap().to_string_lossy();
        let temp = tempfile::tempdir().unwrap();
        copy_model(&model, temp.path());
        let mut xmls = Vec::new();
        decrypt_and_collect_xmls(temp.path(), &mut xmls);
        xmls.sort();
        if xmls.is_empty() {
            mismatches.push(format!("{name}: no XML files"));
        }
        for path in &xmls {
            let content = ltbox_core::xml::read(path).unwrap();
            let doc = ltbox_core::xml::parse(&content).unwrap();
            for node in doc
                .descendants()
                .filter(|n| n.tag_name().name().eq_ignore_ascii_case("program"))
            {
                let label = node.attribute("label").unwrap_or("").trim();
                if label.is_empty() {
                    continue;
                }
                program_count += 1;
                if let Some(expected) = ltbox_core::partition_lun::lun_for_partition(label) {
                    let actual = node.attribute("physical_partition_number").unwrap_or("");
                    if actual.parse::<u8>().ok() != Some(expected) {
                        mismatches.push(format!(
                            "{name}/{}: {label}: expected LUN {expected}, got {actual:?}",
                            path.strip_prefix(temp.path()).unwrap().display()
                        ));
                    }
                }
            }
        }
        match ltbox_device::edl::collect_firmware_xmls_for_flash(temp.path(), false) {
            // A package whose only LUN0 XML writes persist.img is refused by
            // design; that is the guard working, not a corpus mismatch.
            Err(error) if error.to_string().contains("image flashing is disabled") => {
                println!("{name}: selection refused: {error}");
            }
            Err(error) => mismatches.push(format!("{name}: XML selection failed: {error}")),
            Ok((raw, patch)) => {
                selected_count += raw.len() + patch.len();
                for path in raw.iter().chain(&patch) {
                    let content = ltbox_core::xml::read(path).unwrap();
                    let doc = ltbox_core::xml::parse(&content).unwrap();
                    if doc
                        .descendants()
                        .any(|n| n.tag_name().name().eq_ignore_ascii_case("erase"))
                    {
                        mismatches.push(format!(
                            "{name}/{}: selected XML contains <erase>",
                            path.strip_prefix(temp.path()).unwrap().display()
                        ));
                    }
                }
            }
        }
        model_count += 1;
    }
    println!(
        "Checked {model_count} models, {program_count} labeled program nodes, {selected_count} selected XMLs"
    );
    assert!(
        mismatches.is_empty(),
        "rawprogram corpus mismatches:\n{}",
        mismatches.join("\n")
    );
}
