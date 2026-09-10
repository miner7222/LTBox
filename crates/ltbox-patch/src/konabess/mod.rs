//! Independent pure-Rust implementation of KonaBess-style GPU-table editing
//! directly on flattened-device-tree bytes.
//!
//! Upstream: [KonaBess](https://github.com/libxzr/KonaBess) by libxzr,
//! licensed GPL-3. This module reads and writes KonaBess's settings export
//! format for interoperability.
//!
//! KonaBess replaces the complete contiguous `qcom,gpu-pwrlevels-*` sibling
//! set. This module mirrors that behavior directly in flattened-device-tree
//! bytes and deliberately does not perform AVB, EDL, or GUI work.

mod export;
mod fdt;
mod regulator_levels;
mod vendor_boot;

use std::path::{Path, PathBuf};

use fs_err as fs;
use ltbox_core::{LtboxError, Result};
use tracing::info;

use crate::{avb, key_map};

pub use export::{
    GpuGroup, GpuLevel, GpuProperty, GpuTable, GpuTableIssue, GpuTableValidation, KonaBessExport,
    build_gpu_level_from_template, parse_export, parse_gpu_cell, validate_gpu_table,
};
pub use fdt::{
    FdtGpuInfo, GpuTableNormalization, chip_names_match, normalize_edited_gpu_table,
    parse_fdt_gpu_info, replace_fdt_gpu_table, replace_fdt_gpu_table_from_table,
};
pub use regulator_levels::{regulator_level_name, regulator_level_votes};
pub use vendor_boot::{
    ClassifiedDtb, GpuGroupShape, GpuTableShape, VendorBootDtbInfo, classify_vendor_boot_dtbs,
    extract_vendor_boot_dtbs, inspect_vendor_boot_dtbs, inspect_vendor_boot_gpu_candidates,
    replace_vendor_boot_dtb, replace_vendor_boot_gpu_table,
};

/// Final AVB-valid KonaBess image pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonaBessAvbOutput {
    pub vendor_boot: PathBuf,
    /// `None` when the device verifies through the GBL on `efisp`: nothing
    /// rebuilds the AVB chain there, so there is no vbmeta to flash.
    pub vbmeta: Option<PathBuf>,
    pub target_index: usize,
}

/// Stable coarse stages for callers presenting KonaBess build progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KonaBessBuildStage {
    Inspect,
    PatchVendorBoot,
    RebuildVbmeta,
}

/// Read and parse a KonaBess export file.
pub fn read_export(path: &Path) -> Result<KonaBessExport> {
    let text = fs::read_to_string(path).map_err(|e| {
        LtboxError::Patch(format!(
            "cannot read KonaBess export {}: {e}",
            path.display()
        ))
    })?;
    parse_export(&text)
}

/// Apply an export to one DTB and write a rebuilt vendor-boot image once all
/// validation and in-memory rebuilding has succeeded.
///
/// In particular, an incompatible chip cannot create or truncate `output`.
pub fn apply_export_to_vendor_boot_file(
    input: &Path,
    output: &Path,
    target_index: usize,
    export: &KonaBessExport,
) -> Result<()> {
    let image = fs::read(input).map_err(|e| {
        LtboxError::Patch(format!(
            "cannot read vendor_boot image {}: {e}",
            input.display()
        ))
    })?;
    let rebuilt = replace_vendor_boot_dtb(&image, target_index, export)?;
    fs::write(output, rebuilt).map_err(|e| {
        LtboxError::Patch(format!(
            "cannot write vendor_boot image {}: {e}",
            output.display()
        ))
    })
}

/// Build an AVB-valid `vendor_boot.img` + `vbmeta.img` pair from the stock
/// images in `firmware_dir` and a KonaBess export file.
pub fn build_konabess_avb_images(
    firmware_dir: &Path,
    output_dir: &Path,
    export_path: &Path,
    target_index: usize,
) -> Result<KonaBessAvbOutput> {
    build_konabess_avb_images_with_progress(
        firmware_dir,
        output_dir,
        export_path,
        target_index,
        |_| {},
    )
}

/// Build a KonaBess AVB image pair while reporting stable coarse stages.
pub fn build_konabess_avb_images_with_progress(
    firmware_dir: &Path,
    output_dir: &Path,
    export_path: &Path,
    target_index: usize,
    mut on_stage: impl FnMut(KonaBessBuildStage),
) -> Result<KonaBessAvbOutput> {
    let vendor_boot_src = firmware_dir.join("vendor_boot.img");
    let vbmeta_src = firmware_dir.join("vbmeta.img");
    require_source_image(&vendor_boot_src)?;
    require_source_image(&vbmeta_src)?;

    on_stage(KonaBessBuildStage::Inspect);
    let export = read_export(export_path)?;
    build_konabess_avb_images_from_export(
        &vendor_boot_src,
        &vbmeta_src,
        output_dir,
        target_index,
        &export,
        false,
        &mut on_stage,
    )
}

/// Build AVB-valid images from an edited table while reporting stable stages.
pub fn build_konabess_avb_images_from_table_with_progress(
    firmware_dir: &Path,
    output_dir: &Path,
    target_index: usize,
    chip: &str,
    table: &GpuTable,
    gbl_verified: bool,
    mut on_stage: impl FnMut(KonaBessBuildStage),
) -> Result<KonaBessAvbOutput> {
    let vendor_boot_src = firmware_dir.join("vendor_boot.img");
    let vbmeta_src = firmware_dir.join("vbmeta.img");
    require_source_image(&vendor_boot_src)?;
    require_source_image(&vbmeta_src)?;

    on_stage(KonaBessBuildStage::Inspect);
    let export = KonaBessExport {
        chip: chip.to_string(),
        description: String::new(),
        table: table.clone(),
        import_warnings: vec![],
    };
    build_konabess_avb_images_from_export(
        &vendor_boot_src,
        &vbmeta_src,
        output_dir,
        target_index,
        &export,
        gbl_verified,
        &mut on_stage,
    )
}

/// `gbl_verified` marks a device whose boot chain is verified by the GBL EFI on
/// `efisp` instead of by AVB. The root pipeline already leaves the chain alone
/// there, and the same applies here: it is what lets a Lenovo-key vbmeta
/// through, since that key cannot be re-signed and nothing on these devices
/// needs it to be.
fn build_konabess_avb_images_from_export(
    vendor_boot_src: &Path,
    vbmeta_src: &Path,
    output_dir: &Path,
    target_index: usize,
    export: &KonaBessExport,
    gbl_verified: bool,
    on_stage: &mut impl FnMut(KonaBessBuildStage),
) -> Result<KonaBessAvbOutput> {
    let vendor_boot_info = avb::extract_image_avb_info(vendor_boot_src)?;
    if vendor_boot_info.partition_name.as_deref() != Some("vendor_boot") {
        return Err(error(format!(
            "{} AVB partition is {:?}, expected `vendor_boot`",
            vendor_boot_src.display(),
            vendor_boot_info.partition_name
        )));
    }
    if vendor_boot_info.algorithm != "NONE" {
        return Err(error(format!(
            "{} AVB algorithm is {}, expected NONE",
            vendor_boot_src.display(),
            vendor_boot_info.algorithm
        )));
    }
    let original_image_size = vendor_boot_info.original_image_size.ok_or_else(|| {
        error(format!(
            "{} has no appended AVB footer",
            vendor_boot_src.display()
        ))
    })?;

    // Resolve the vbmeta key before patching or touching the output directory.
    // A present-but-unknown key must never leave a tempting unsigned artifact.
    // Skipped when the GBL verifies the device: no vbmeta is produced, so
    // demanding a usable signing key would turn away a device that needs none.
    let vbmeta_rebuild = if gbl_verified {
        None
    } else {
        let info = avb::extract_image_avb_info(vbmeta_src)?;
        let key = key_map::key_spec_for_signed_pubkey(info.public_key_sha1.as_deref()).map_err(
            |key| LtboxError::Avb(key_map::unresolved_signing_key_error("vbmeta.img", &key)),
        )?;
        Some((info, key))
    };

    let mut patchable = fs::read(vendor_boot_src).map_err(|e| {
        error(format!(
            "cannot read vendor_boot image {}: {e}",
            vendor_boot_src.display()
        ))
    })?;
    let partition_size = u64::try_from(patchable.len())
        .map_err(|_| error("vendor_boot partition size does not fit u64"))?;
    if partition_size != vendor_boot_info.partition_size {
        return Err(error(
            "vendor_boot AVB partition size changed during inspection",
        ));
    }
    let original_size = usize::try_from(original_image_size)
        .map_err(|_| error("vendor_boot AVB original image size does not fit usize"))?;
    if original_size > patchable.len() {
        return Err(error(format!(
            "vendor_boot AVB payload size {original_size} exceeds partition size {}",
            patchable.len()
        )));
    }
    let source_payload_end = vendor_boot::vendor_boot_payload_end(&patchable)?;
    if source_payload_end > original_size {
        return Err(error(format!(
            "vendor_boot sections end at {source_payload_end}, beyond authenticated AVB payload size {original_size}"
        )));
    }

    // Only the authenticated AVB tail is disposable. Bytes before
    // original_image_size remain intact, so the raw rebuilder's nonzero-tail
    // guard still rejects growth into genuine vendor data.
    patchable[original_size..].fill(0);

    on_stage(KonaBessBuildStage::PatchVendorBoot);
    let mut rebuilt = replace_vendor_boot_dtb(&patchable, target_index, export)?;
    let payload_end = vendor_boot::vendor_boot_payload_end(&rebuilt)?;
    let rebuilt_image_size = original_size.max(payload_end);
    let max_image_size = usize::try_from(avb::max_hash_footer_image_size(partition_size)?)
        .map_err(|_| error("maximum vendor_boot payload size does not fit usize"))?;
    if rebuilt_image_size > max_image_size {
        return Err(error(format!(
            "rebuilt vendor_boot needs {rebuilt_image_size} payload bytes but AVB leaves {max_image_size} bytes in partition capacity {partition_size}"
        )));
    }
    rebuilt.truncate(rebuilt_image_size);

    // All required validation, including key lookup and capacity, has now
    // succeeded. Only from here onward may output paths be changed.
    if output_dir.exists() {
        fs::remove_dir_all(output_dir).map_err(|e| {
            error(format!(
                "cannot clear KonaBess output {}: {e}",
                output_dir.display()
            ))
        })?;
    }
    fs::create_dir_all(output_dir).map_err(|e| {
        error(format!(
            "cannot create KonaBess output {}: {e}",
            output_dir.display()
        ))
    })?;

    let vendor_boot_out = output_dir.join("vendor_boot.img");
    fs::write(&vendor_boot_out, rebuilt).map_err(|e| {
        error(format!(
            "cannot write vendor_boot image {}: {e}",
            vendor_boot_out.display()
        ))
    })?;
    // A GBL-verified device takes the repacked image as-is, the same way the
    // root pipeline flashes its repacked boot image without re-adding a footer.
    if !gbl_verified {
        avb::add_hash_footer(&vendor_boot_out, &vendor_boot_info, None, None)?;
    }
    info!(
        "Applied KonaBess export to DTB {target_index} and rebuilt {}",
        vendor_boot_out.display()
    );

    on_stage(KonaBessBuildStage::RebuildVbmeta);
    let vbmeta_out = match vbmeta_rebuild {
        None => {
            info!("GBL verifies this device; leaving the AVB chain untouched");
            None
        }
        Some((vbmeta_info, key)) => {
            let out = output_dir.join("vbmeta.img");
            match key {
                Some(key_spec) => {
                    avb::rebuild_vbmeta_with_partition_descriptors(
                        &out,
                        vbmeta_src,
                        &[vendor_boot_out.as_path()],
                        key_spec,
                        Some(&vbmeta_info.algorithm),
                    )?;
                    info!("Refreshed vbmeta descriptors: {}", out.display());
                }
                None => {
                    fs::copy(vbmeta_src, &out)?;
                    info!("vbmeta is unsigned; copied stock blob");
                }
            }
            Some(out)
        }
    };

    Ok(KonaBessAvbOutput {
        vendor_boot: vendor_boot_out,
        vbmeta: vbmeta_out,
        target_index,
    })
}

fn require_source_image(path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(LtboxError::FileNotFound(path.display().to_string()))
    }
}

fn error(message: impl Into<String>) -> LtboxError {
    LtboxError::Patch(format!("KonaBess: {}", message.into()))
}
