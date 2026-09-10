//! Real-image KonaBess import coverage.
//!
//! The repository does not carry firmware or export fixtures. Opt in with:
//!
//! ```powershell
//! $env:KONABESS_TB322_IMAGE = 'D:\fixtures\tb322\vendor_boot.img'
//! $env:KONABESS_TB322_EXPORT = 'D:\fixtures\konabess-sun.txt'
//! $env:KONABESS_TB321_IMAGE = 'D:\fixtures\tb321\vendor_boot.img'
//! $env:KONABESS_TB321_EXPORTS = 'D:\fixtures\a.txt;D:\fixtures\b.txt'
//! cargo test -p ltbox-patch --test konabess_fixtures -- --ignored --nocapture
//! ```

use std::collections::BTreeSet;
use std::env;
use std::path::PathBuf;

use ltbox_patch::avb;
use ltbox_patch::konabess::{
    GpuTable, KonaBessExport, build_konabess_avb_images, classify_vendor_boot_dtbs,
    extract_vendor_boot_dtbs, inspect_vendor_boot_gpu_candidates, parse_fdt_gpu_info, read_export,
    replace_fdt_gpu_table_from_table, replace_vendor_boot_dtb, replace_vendor_boot_gpu_table,
};

fn required_path(name: &str) -> PathBuf {
    PathBuf::from(env::var_os(name).unwrap_or_else(|| panic!("{name} must be set")))
}

fn changed_cell_count(before: &GpuTable, after: &GpuTable) -> usize {
    before
        .groups
        .iter()
        .map(|before_group| {
            let after_group = after
                .groups
                .iter()
                .find(|group| group.id == before_group.id)
                .expect("matching group id");
            before_group
                .header_properties
                .iter()
                .map(|property| {
                    (
                        property,
                        after_group
                            .header_properties
                            .iter()
                            .find(|candidate| candidate.name == property.name)
                            .expect("matching header property"),
                    )
                })
                .chain(before_group.levels.iter().flat_map(|before_level| {
                    let after_level = after_group
                        .levels
                        .iter()
                        .find(|level| level.id == before_level.id)
                        .expect("matching level id");
                    before_level.properties.iter().map(|property| {
                        (
                            property,
                            after_level
                                .properties
                                .iter()
                                .find(|candidate| candidate.name == property.name)
                                .expect("matching level property"),
                        )
                    })
                }))
                .map(|(before_property, after_property)| {
                    assert_eq!(before_property.cells.len(), after_property.cells.len());
                    before_property
                        .cells
                        .iter()
                        .zip(&after_property.cells)
                        .filter(|(before, after)| before != after)
                        .count()
                })
                .sum::<usize>()
        })
        .sum()
}

fn heterogeneous_group_count(table: &GpuTable) -> usize {
    table
        .groups
        .iter()
        .filter(|group| {
            let Some(first) = group.levels.first() else {
                return false;
            };
            let expected = first
                .properties
                .iter()
                .map(|property| property.name.as_str())
                .collect::<BTreeSet<_>>();
            group.levels.iter().skip(1).any(|level| {
                level
                    .properties
                    .iter()
                    .map(|property| property.name.as_str())
                    .collect::<BTreeSet<_>>()
                    != expected
            })
        })
        .count()
}

#[test]
#[ignore = "requires LTBOX_TEST_KONABESS_VENDOR_BOOT_DIR with local real images"]
fn every_real_gpu_dtb_round_trips_and_in_memory_edits_preserve_other_dtbs() {
    let fixture_dir = required_path("LTBOX_TEST_KONABESS_VENDOR_BOOT_DIR");
    let cases = [
        ("tb320fc.img", &[3usize, 4, 5, 6][..]),
        ("tb321fu.img", &[4usize, 5, 6, 7, 8, 9, 10][..]),
        (
            "tb322fc.img",
            &[2usize, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12][..],
        ),
        ("tb323fu.img", &[6usize, 7, 8, 9, 12, 13][..]),
    ];

    let mut heterogeneous_stock_groups = 0;
    for (file_name, expected_indices) in cases {
        let image = std::fs::read(fixture_dir.join(file_name)).unwrap();
        let original_blobs = extract_vendor_boot_dtbs(&image).unwrap();
        let candidates = inspect_vendor_boot_gpu_candidates(&image).unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.index)
                .collect::<Vec<_>>(),
            expected_indices
        );

        for candidate in &candidates {
            let chip = candidate
                .chip
                .as_deref()
                .expect("candidate chip must be enabled");
            let table = candidate.table.as_ref().expect("candidate table");
            assert_eq!(candidate.gpu_shape, Some(table.into()));
            let rebuilt =
                replace_fdt_gpu_table_from_table(&original_blobs[candidate.index], chip, table)
                    .unwrap();
            assert_eq!(
                rebuilt, original_blobs[candidate.index],
                "{file_name} DTB {} ({chip}) was not byte-identical",
                candidate.index
            );
            heterogeneous_stock_groups += heterogeneous_group_count(table);
        }

        let target = &candidates[0];
        let chip = target.chip.as_deref().unwrap();
        let mut edited = target.table.clone().unwrap();
        let level = &mut edited.groups[0].levels[0];
        let frequency = level
            .properties
            .iter_mut()
            .find(|property| property.name == "qcom,gpu-freq")
            .unwrap();
        frequency.cells[0] = frequency.cells[0].checked_add(1).unwrap();
        let expected_frequency = frequency.cells[0];
        let level_vote = level
            .properties
            .iter_mut()
            .find(|property| property.name == "qcom,level")
            .unwrap();
        level_vote.cells[0] = level_vote.cells[0].checked_add(1).unwrap();
        let expected_level_vote = level_vote.cells[0];

        let rebuilt_image =
            replace_vendor_boot_gpu_table(&image, target.index, chip, &edited).unwrap();
        let rebuilt_blobs = extract_vendor_boot_dtbs(&rebuilt_image).unwrap();
        for (index, (before, after)) in original_blobs.iter().zip(&rebuilt_blobs).enumerate() {
            if index != target.index {
                assert_eq!(before, after, "{file_name} non-target DTB {index} changed");
            }
        }
        let applied = parse_fdt_gpu_info(&rebuilt_blobs[target.index])
            .unwrap()
            .table
            .unwrap();
        let applied_level = &applied.groups[0].levels[0];
        assert_eq!(
            applied_level
                .properties
                .iter()
                .find(|property| property.name == "qcom,gpu-freq")
                .unwrap()
                .cells,
            [expected_frequency]
        );
        assert_eq!(
            applied_level
                .properties
                .iter()
                .find(|property| property.name == "qcom,level")
                .unwrap()
                .cells,
            [expected_level_vote]
        );

        let chips = candidates
            .iter()
            .map(|candidate| candidate.chip.as_deref().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        let image_heterogeneous_groups = candidates
            .iter()
            .map(|candidate| heterogeneous_group_count(candidate.table.as_ref().unwrap()))
            .sum::<usize>();
        println!(
            "{file_name}: byte-identical all candidates; chips={chips:?}; heterogeneous_groups={image_heterogeneous_groups}"
        );
    }
    assert!(
        heterogeneous_stock_groups > 0,
        "real stock tables unexpectedly became homogeneous"
    );
}

#[test]
#[ignore = "requires locally supplied vendor_boot and KonaBess exports"]
fn real_vendor_boot_images_match_known_shapes_and_apply_exactly() {
    let sun_image = std::fs::read(required_path("KONABESS_TB322_IMAGE")).unwrap();
    let sun_export = read_export(&required_path("KONABESS_TB322_EXPORT")).unwrap();
    let sun_classified = classify_vendor_boot_dtbs(&sun_image, &sun_export).unwrap();
    let sun_matches = sun_classified
        .iter()
        .filter(|blob| blob.structurally_matches)
        .map(|blob| blob.info.index)
        .collect::<Vec<_>>();
    assert_eq!(sun_classified.len(), 13);
    assert_eq!(sun_matches, [2, 4, 6, 8]);
    assert!(
        sun_classified
            .iter()
            .all(|blob| blob.info.chip.as_deref() == Some("sun"))
    );
    println!("TB322FC: blobs=13 structural_matches={sun_matches:?}");

    let original_blobs = extract_vendor_boot_dtbs(&sun_image).unwrap();
    let original_info = parse_fdt_gpu_info(&original_blobs[2]).unwrap();
    let original_table = original_info.table.unwrap();
    let self_export = KonaBessExport {
        chip: "sun".into(),
        description: "fixture round-trip".into(),
        table: original_table.clone(),
        import_warnings: vec![],
    };
    let round_trip = replace_vendor_boot_dtb(&sun_image, 2, &self_export).unwrap();
    let round_trip_blobs = extract_vendor_boot_dtbs(&round_trip).unwrap();
    assert_eq!(round_trip_blobs[2], original_blobs[2]);
    println!("TB322FC blob 2: same-table round-trip is byte-identical");

    let applied = replace_vendor_boot_dtb(&sun_image, 2, &sun_export).unwrap();
    let applied_blobs = extract_vendor_boot_dtbs(&applied).unwrap();
    let applied_table = parse_fdt_gpu_info(&applied_blobs[2])
        .unwrap()
        .table
        .unwrap();
    assert_eq!(applied_table, sun_export.table);
    let changed_cells = changed_cell_count(&original_table, &applied_table);
    assert_eq!(changed_cells, 6);
    assert_eq!(
        original_blobs
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 2)
            .map(|(_, blob)| blob)
            .collect::<Vec<_>>(),
        applied_blobs
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 2)
            .map(|(_, blob)| blob)
            .collect::<Vec<_>>()
    );
    println!("TB322FC blob 2: export applied, changed_u32_cells={changed_cells}");

    let sun_image_path = required_path("KONABESS_TB322_IMAGE");
    let sun_firmware_dir = sun_image_path.parent().unwrap();
    let avb_temp = tempfile::tempdir().unwrap();
    let avb_output = build_konabess_avb_images(
        sun_firmware_dir,
        &avb_temp.path().join("konabess"),
        &required_path("KONABESS_TB322_EXPORT"),
        2,
    )
    .unwrap();
    let produced_vendor = std::fs::read(&avb_output.vendor_boot).unwrap();
    let produced_table =
        parse_fdt_gpu_info(&extract_vendor_boot_dtbs(&produced_vendor).unwrap()[2])
            .unwrap()
            .table
            .unwrap();
    assert_eq!(produced_table, sun_export.table);
    let produced_info = avb::extract_image_avb_info(&avb_output.vendor_boot).unwrap();
    assert_eq!(produced_info.partition_name.as_deref(), Some("vendor_boot"));
    assert!(produced_info.original_image_size.is_some());

    let vendor_descriptor = vendor_boot_hash_descriptor(&avb_output.vendor_boot);
    let rebuilt_vbmeta = avb_output.vbmeta.as_ref().expect("vbmeta rebuilt");
    let vbmeta_descriptor = vendor_boot_hash_descriptor(rebuilt_vbmeta);
    assert_eq!(vendor_descriptor, vbmeta_descriptor);
    avbtool_rs::verify::verify_image(
        &avb_output.vendor_boot,
        &avbtool_rs::verify::VerifyImageOptions {
            key_blob: None,
            expected_chain_partitions: Vec::new(),
            follow_chain_partitions: false,
            accept_zeroed_hashtree: false,
        },
    )
    .unwrap();
    println!(
        "TB322FC blob 2 AVB build: partition=vendor_boot footer=valid vbmeta_descriptor=matching"
    );

    let pineapple_image = std::fs::read(required_path("KONABESS_TB321_IMAGE")).unwrap();
    let export_paths = env::split_paths(
        &env::var_os("KONABESS_TB321_EXPORTS").expect("KONABESS_TB321_EXPORTS must be set"),
    )
    .collect::<Vec<_>>();
    assert_eq!(export_paths.len(), 4);
    for export_path in export_paths {
        let export = read_export(&export_path).unwrap();
        let classified = classify_vendor_boot_dtbs(&pineapple_image, &export).unwrap();
        assert_eq!(classified.len(), 11);
        assert!(classified.iter().all(|blob| !blob.structurally_matches));
        assert!(classified.iter().all(|blob| {
            blob.info.chip.as_deref() == Some("pineapple") || blob.info.gpu_shape.is_none()
        }));
        println!(
            "TB321FU {}: blobs=11 structural_matches=[]",
            export_path.display()
        );
    }
}

fn vendor_boot_hash_descriptor(path: &std::path::Path) -> avbtool_rs::info::DescriptorInfo {
    avbtool_rs::image::inspect_avb_image(path)
        .unwrap()
        .descriptors
        .into_iter()
        .find(|descriptor| {
            matches!(
                descriptor,
                avbtool_rs::info::DescriptorInfo::Hash { partition_name, .. }
                    if partition_name == "vendor_boot"
            )
        })
        .expect("vendor_boot hash descriptor")
}
