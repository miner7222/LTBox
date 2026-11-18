//! AVB patching — wraps avbtool-rs library for image signing operations.

use fs_err as fs;
use std::path::{Path, PathBuf};

use ltbox_core::{LtboxError, Result};
use tracing::info;

/// Parent image carrying the root partition's Hash descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VbmetaOwner {
    Vbmeta,
    VbmetaSystem,
}

impl VbmetaOwner {
    pub const fn partition_base(self) -> &'static str {
        match self {
            Self::Vbmeta => "vbmeta",
            Self::VbmetaSystem => "vbmeta_system",
        }
    }

    pub const fn filename(self) -> &'static str {
        match self {
            Self::Vbmeta => "vbmeta.img",
            Self::VbmetaSystem => "vbmeta_system.img",
        }
    }

    pub fn lun(self) -> Result<u8> {
        ltbox_core::partition_lun::lun_for_partition(self.partition_base())
            .ok_or_else(|| LtboxError::Avb(format!("No LUN for {}", self.partition_base())))
    }
}

fn target_hash_count(descriptors: &[avbtool_rs::info::DescriptorInfo], target: &str) -> usize {
    descriptors.iter().filter(|descriptor| matches!(descriptor,
        avbtool_rs::info::DescriptorInfo::Hash { partition_name, .. } if partition_name == target
    )).count()
}

/// Number of hashes for a root partition, retaining parse failures as errors.
pub fn root_hash_descriptor_count(image: &Path, target: &str) -> Result<usize> {
    let info = avbtool_rs::image::inspect_avb_image(image)
        .map_err(|error| LtboxError::Avb(format!("inspect {}: {error}", image.display())))?;
    Ok(target_hash_count(&info.descriptors, target))
}

/// Pure ownership policy. Every chained vbmeta_system must be inspected even
/// when vbmeta already hashes the target, to rule out two owners.
pub fn decide_root_vbmeta_owner(
    target: &str,
    vbmeta: &[avbtool_rs::info::DescriptorInfo],
    vbmeta_system: Option<&[avbtool_rs::info::DescriptorInfo]>,
) -> Result<VbmetaOwner> {
    let chains: Vec<_> = vbmeta
        .iter()
        .filter_map(|descriptor| match descriptor {
            avbtool_rs::info::DescriptorInfo::ChainPartition {
                partition_name,
                flags,
                ..
            } if partition_name == "vbmeta_system" => Some(*flags),
            _ => None,
        })
        .collect();
    if !chains.is_empty() && vbmeta_system.is_none() {
        return Err(LtboxError::Avb(
            "Chained vbmeta_system must be inspected for the root hash owner".into(),
        ));
    }
    let top = target_hash_count(vbmeta, target);
    let system = vbmeta_system.map_or(0, |descriptors| target_hash_count(descriptors, target));
    if top == 1 && system == 0 {
        return Ok(VbmetaOwner::Vbmeta);
    }
    if chains.len() > 1 || chains.first().is_some_and(|flags| *flags != 0) {
        return Err(LtboxError::Avb(
            "Ambiguous or non-A/B vbmeta_system chain".into(),
        ));
    }
    match (top, system, chains.len()) {
        (0, 1, 1) => Ok(VbmetaOwner::VbmetaSystem),
        (0, 0, _) => Err(LtboxError::Avb(format!(
            "Missing root hash owner for {target}"
        ))),
        _ => Err(LtboxError::Avb(format!(
            "Ambiguous root hash owner for {target}: vbmeta={top}, vbmeta_system={system}"
        ))),
    }
}

/// Inspect the actual parent images; parse failures never become a fallback.
pub fn detect_root_vbmeta_owner(work_dir: &Path, target: &str) -> Result<VbmetaOwner> {
    let top = avbtool_rs::image::inspect_avb_image(&work_dir.join("vbmeta.img"))
        .map_err(|error| LtboxError::Avb(format!("inspect vbmeta.img: {error}")))?;
    let system = if top.descriptors.iter().any(|descriptor| matches!(descriptor,
        avbtool_rs::info::DescriptorInfo::ChainPartition { partition_name, .. } if partition_name == "vbmeta_system"
    )) {
        Some(avbtool_rs::image::inspect_avb_image(&work_dir.join("vbmeta_system.img"))
            .map_err(|error| LtboxError::Avb(format!("inspect vbmeta_system.img: {error}")))?)
    } else {
        None
    };
    decide_root_vbmeta_owner(
        target,
        &top.descriptors,
        system.as_ref().map(|info| info.descriptors.as_slice()),
    )
}

/// Parsed AVB image metadata.
#[derive(Debug, Clone)]
pub struct AvbImageInfo {
    pub partition_size: u64,
    /// Size of the authenticated image payload before an appended AVB footer.
    /// `None` for standalone vbmeta images without a footer.
    pub original_image_size: Option<u64>,
    pub algorithm: String,
    pub rollback_index: u64,
    /// AVB rollback-index *slot* this image's index is checked against.
    /// Preserved when re-adding a hash footer so the bootloader keeps
    /// reading the device-committed value from the original location.
    pub rollback_index_location: u32,
    pub flags: u32,
    pub partition_name: Option<String>,
    pub salt: Option<Vec<u8>>,
    pub public_key_sha1: Option<String>,
    pub props: Vec<(String, Vec<u8>)>,
    // Recorded now because the footer API cannot recover it from the modeled
    // fields once the image has been copied or patched.
    source_image_path: PathBuf,
    hash_descriptor_algorithm: Option<String>,
    hash_descriptor_count: usize,
    unreproducible_descriptor_kinds: Vec<String>,
}

/// The Android build fingerprint embedded in an image's AVB property
/// descriptors (`com.android.build.<part>.fingerprint`, e.g.
/// `qti/TB323FU/...:user/release-keys`), if present. Identifies the firmware an
/// image belongs to.
///
/// Custom ROMs mostly carry the device codename instead of the model, while a
/// few props (a stock vendor blob, `pvmfw`) keep the model token, so the pick
/// prefers a value that names a known model:
/// 1. `com.android.build.system.fingerprint` (carried by vbmeta_system, the
///    unified identity source) when it names a known model;
/// 2. otherwise the first `.fingerprint` prop, in descriptor order, that does;
/// 3. otherwise `com.android.build.system.fingerprint`;
/// 4. otherwise the first `.fingerprint` prop.
///
/// Empty values are ignored throughout.
pub fn build_fingerprint(info: &AvbImageInfo) -> Option<String> {
    let system = info
        .props
        .iter()
        .filter(|(k, _)| k == "com.android.build.system.fingerprint")
        .find_map(|(_, v)| fingerprint_value(v));
    if let Some(system) = &system
        && ltbox_core::model::fingerprint_names_known_model(system)
    {
        return Some(system.clone());
    }
    let mut first = None;
    for (_, value) in info
        .props
        .iter()
        .filter(|(k, _)| k.ends_with(".fingerprint"))
    {
        let Some(value) = fingerprint_value(value) else {
            continue;
        };
        if ltbox_core::model::fingerprint_names_known_model(&value) {
            return Some(value);
        }
        first.get_or_insert(value);
    }
    system.or(first)
}

fn fingerprint_value(raw: &[u8]) -> Option<String> {
    std::str::from_utf8(raw)
        .ok()
        .map(|s| s.trim_end_matches('\0').trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Render `avbtool info_image`-style metadata for one or more images.
pub fn image_info_report(image_paths: &[PathBuf]) -> Result<String> {
    if image_paths.is_empty() {
        return Err(LtboxError::Avb("No image files selected".to_string()));
    }

    let mut reports = Vec::with_capacity(image_paths.len());
    for path in image_paths {
        let report = avbtool_rs::info::generate_info_report(path)
            .map_err(|e| LtboxError::Avb(format!("info_image {}: {e}", path.display())))?;
        reports.push(report.trim_end().to_string());
    }
    Ok(reports.join("\n================================================================\n\n"))
}

/// Extract AVB metadata from an image.
pub fn extract_image_avb_info(image_path: &Path) -> Result<AvbImageInfo> {
    let info = avbtool_rs::image::inspect_avb_image(image_path)
        .map_err(|e| LtboxError::Avb(format!("inspect {}: {e}", image_path.display())))?;

    let file_size = fs::metadata(image_path).map(|m| m.len()).unwrap_or(0);
    let partition_size = if info.footer.is_some() {
        file_size
    } else {
        avbtool_rs::image::compute_vbmeta_blob_size(&info.header).unwrap_or(0)
    };

    let mut partition_name = None;
    let mut salt = None;
    let mut props = Vec::new();
    let mut hash_descriptor_algorithm = None;
    let mut hash_descriptor_count = 0;
    let mut unreproducible_descriptor_kinds = Vec::new();

    for desc in &info.descriptors {
        match desc {
            avbtool_rs::info::DescriptorInfo::Hash {
                hash_algorithm,
                partition_name: pn,
                salt: s,
                ..
            } => {
                hash_descriptor_count += 1;
                if hash_descriptor_algorithm.is_none() {
                    hash_descriptor_algorithm = Some(hash_algorithm.clone());
                }
                if partition_name.is_none() {
                    partition_name = Some(pn.clone());
                    salt = Some(s.clone());
                }
            }
            avbtool_rs::info::DescriptorInfo::Hashtree {
                partition_name: pn,
                salt: s,
                ..
            } => {
                if partition_name.is_none() {
                    partition_name = Some(pn.clone());
                    salt = Some(s.clone());
                }
                unreproducible_descriptor_kinds.push("Hashtree".to_string());
            }
            avbtool_rs::info::DescriptorInfo::Property { key, value } => {
                props.push((key.clone(), value.clone()));
            }
            avbtool_rs::info::DescriptorInfo::ChainPartition { .. } => {
                unreproducible_descriptor_kinds.push("ChainPartition".to_string());
            }
            avbtool_rs::info::DescriptorInfo::KernelCmdline { .. } => {
                unreproducible_descriptor_kinds.push("KernelCmdline".to_string());
            }
            avbtool_rs::info::DescriptorInfo::Unknown { tag, .. } => {
                unreproducible_descriptor_kinds.push(format!("Unknown (tag {tag})"));
            }
        }
    }

    Ok(AvbImageInfo {
        partition_size,
        original_image_size: info
            .footer
            .as_ref()
            .map(|footer| footer.original_image_size),
        algorithm: info.algorithm_name.clone(),
        rollback_index: info.header.rollback_index,
        rollback_index_location: info.header.rollback_index_location,
        flags: info.header.flags,
        partition_name,
        salt,
        public_key_sha1: info.public_key_sha1.clone(),
        props,
        source_image_path: image_path.to_path_buf(),
        hash_descriptor_algorithm,
        hash_descriptor_count,
        unreproducible_descriptor_kinds,
    })
}

/// A chain partition descriptor a vbmeta declares: the partition name and
/// whether it is flagged `DO_NOT_USE_AB` (AVB verifies the unsuffixed name, so
/// the image is flashed to `<name>` rather than `<name>_a`).
#[derive(Debug, Clone)]
pub struct ChainPartitionDescriptor {
    pub name: String,
    pub do_not_use_ab: bool,
}

/// The chain partition descriptors a vbmeta image declares, in descriptor order
/// (e.g. `boot`, `recovery`, `vbmeta_system`). Drives a layout-aware re-sign:
/// only partitions this vbmeta chains are re-signed and get their chain public
/// keys updated, and each carries its `DO_NOT_USE_AB` flag so the caller flashes
/// the right GPT label. Packages without recovery or with non-A/B chains work.
pub fn chain_partition_descriptors(vbmeta_path: &Path) -> Result<Vec<ChainPartitionDescriptor>> {
    // libavb `AVB_CHAIN_PARTITION_DESCRIPTOR_FLAGS_DO_NOT_USE_AB`.
    const DO_NOT_USE_AB: u32 = 1;
    let info = avbtool_rs::image::inspect_avb_image(vbmeta_path)
        .map_err(|e| LtboxError::Avb(format!("inspect {}: {e}", vbmeta_path.display())))?;
    Ok(info
        .descriptors
        .iter()
        .filter_map(|d| match d {
            avbtool_rs::info::DescriptorInfo::ChainPartition {
                partition_name,
                flags,
                ..
            } => Some(ChainPartitionDescriptor {
                name: partition_name.clone(),
                do_not_use_ab: *flags & DO_NOT_USE_AB != 0,
            }),
            _ => None,
        })
        .collect())
}

/// The Hash descriptor an image declares for `partition_name` — from a vbmeta
/// image (the digest vbmeta pins) or from a partition image's own footer.
/// Comparing the two proves a rebuilt vbmeta adopted the image it was rebuilt
/// from rather than keeping a stale digest the bootloader would reject.
pub fn hash_descriptor(
    image: &Path,
    partition_name: &str,
) -> Result<avbtool_rs::info::DescriptorInfo> {
    let info = avbtool_rs::image::inspect_avb_image(image)
        .map_err(|e| LtboxError::Avb(format!("inspect {}: {e}", image.display())))?;
    let mut matches = info.descriptors.into_iter().filter(|descriptor| {
        matches!(
            descriptor,
            avbtool_rs::info::DescriptorInfo::Hash {
                partition_name: name,
                ..
            } if name == partition_name
        )
    });
    let descriptor = matches.next().ok_or_else(|| {
        LtboxError::Avb(format!(
            "{} has no Hash descriptor for {partition_name}",
            image.display()
        ))
    })?;
    if matches.next().is_some() {
        return Err(LtboxError::Avb(format!(
            "{} has duplicate Hash descriptors for {partition_name}",
            image.display()
        )));
    }
    Ok(descriptor)
}

/// Verify the root payload against its own AVB hash and, when restored,
/// require vbmeta to contain the exact same root hash descriptor. This is
/// integrity validation, not proof that the signer is trusted by a device.
pub fn verify_root_backup(image: &Path, vbmeta: Option<&Path>, partition: &str) -> Result<()> {
    let info =
        avbtool_rs::image::inspect_avb_image(image).map_err(|e| LtboxError::Avb(e.to_string()))?;
    let expected_name = format!("{partition}.img");
    if image.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str()) {
        return Err(LtboxError::Avb(
            "Root backup filename does not match its partition".into(),
        ));
    }
    // The upstream verifier resolves descriptors via sibling filenames; limit
    // this self-check to the selected root image.
    if info.descriptors.iter().any(|d| matches!(d,
        avbtool_rs::info::DescriptorInfo::Hash { partition_name, .. } if partition_name != partition)
        || matches!(d, avbtool_rs::info::DescriptorInfo::Hashtree { .. } | avbtool_rs::info::DescriptorInfo::ChainPartition { .. })) {
        return Err(LtboxError::Avb("Unexpected descriptor in root backup".into()));
    }
    let descriptor = hash_descriptor(image, partition)?;
    if matches!(&descriptor, avbtool_rs::info::DescriptorInfo::Hash { digest, .. } if digest.is_empty())
    {
        return Err(LtboxError::Avb(
            "Root backup has no verifiable digest".into(),
        ));
    }
    avbtool_rs::verify::verify_image(
        image,
        &avbtool_rs::verify::VerifyImageOptions {
            key_blob: None,
            expected_chain_partitions: Vec::new(),
            follow_chain_partitions: false,
            accept_zeroed_hashtree: false,
        },
    )
    .map_err(|e| LtboxError::Avb(e.to_string()))?;
    if let Some(vbmeta) = vbmeta
        && hash_descriptor(vbmeta, partition)? != descriptor
    {
        return Err(LtboxError::Avb(
            "Root backup and vbmeta hash descriptors do not match".into(),
        ));
    }
    Ok(())
}

/// The AVB algorithm name for a signing-key spec (bundled name or PEM path),
/// e.g. `testkey_rsa4096` -> `SHA256_RSA4096`, derived from the key's size.
pub fn algorithm_for_key_spec(key_spec: &str) -> Option<String> {
    let key = avbtool_rs::crypto::load_key_from_spec(key_spec).ok()?;
    key.algorithm().ok()
}

/// Resign an image. `key_spec` → bundled name (`testkey_rsa2048` / …)
/// or filesystem path to a PEM; passed to `avbtool_rs::crypto::load_key_from_spec`.
pub fn resign_image(
    image_path: &Path,
    key_spec: &str,
    algorithm: &str,
    rollback_index: Option<u64>,
) -> Result<()> {
    avbtool_rs::resign::resign_image_with_options(
        image_path,
        key_spec,
        Some(algorithm),
        false,
        rollback_index,
        false,
    )
    .map_err(|e| LtboxError::Avb(format!("resign failed: {e}")))?;
    Ok(())
}

/// Maximum authenticated payload size that still leaves room for a hash
/// footer in `partition_size`.
pub fn max_hash_footer_image_size(partition_size: u64) -> Result<u64> {
    avbtool_rs::footer::calc_max_hash_footer_image_size(partition_size)
        .map_err(|e| LtboxError::Avb(format!("calculate maximum hash-footer image size: {e}")))
}

/// Rebuild `vbmeta.img` using the original as a template, refreshing matching
/// Hash / Hashtree descriptors from `partition_images`. Chain partition
/// descriptors and public keys are unchanged. Partition images must already
/// carry current descriptors; no footers are added or updated on them.
/// `key_spec` follows the [`resign_image`] convention.
pub fn rebuild_vbmeta_with_partition_descriptors(
    output_path: &Path,
    original_vbmeta_path: &Path,
    partition_images: &[&Path],
    key_spec: &str,
    algorithm: Option<&str>,
) -> Result<()> {
    avbtool_rs::builder::rebuild_vbmeta_image(
        output_path,
        original_vbmeta_path,
        partition_images,
        key_spec,
        algorithm,
    )
    .map_err(|e| LtboxError::Avb(format!("rebuild_vbmeta_image: {e}")))?;
    preserve_original_vbmeta_size(output_path, original_vbmeta_path)?;
    Ok(())
}

/// Compare descriptor values with multiplicity, independent of builder order.
fn descriptor_values_match(
    expected: &[avbtool_rs::info::DescriptorInfo],
    actual: &[avbtool_rs::info::DescriptorInfo],
) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    // The builder sorts descriptors, so compare values with multiplicity
    // (duplicate properties, unknown descriptors), not as a set or by position.
    let mut remaining: Vec<_> = expected.iter().collect();
    for descriptor in actual {
        let Some(index) = remaining.iter().position(|value| *value == descriptor) else {
            return false;
        };
        remaining.swap_remove(index);
    }
    remaining.is_empty()
}

/// Rebuild a root hash owner with its original signing algorithm, retaining
/// every unrelated descriptor value and the owner's rollback metadata.
pub fn rebuild_root_vbmeta_owner(
    output: &Path,
    original: &Path,
    root_image: &Path,
    partition: &str,
    key: &str,
) -> Result<()> {
    let before = avbtool_rs::image::inspect_avb_image(original)
        .map_err(|error| LtboxError::Avb(error.to_string()))?;
    // Both sides must carry exactly one hash, not a first-match approximation.
    hash_descriptor(original, partition)?;
    let replacement = hash_descriptor(root_image, partition)?;
    let mut expected = before.descriptors.clone();
    for descriptor in &mut expected {
        if matches!(descriptor, avbtool_rs::info::DescriptorInfo::Hash { partition_name, .. } if partition_name == partition)
        {
            *descriptor = replacement.clone();
        }
    }
    rebuild_vbmeta_with_partition_descriptors(
        output,
        original,
        &[root_image],
        key,
        Some(&before.algorithm_name),
    )?;
    let after = avbtool_rs::image::inspect_avb_image(output)
        .map_err(|error| LtboxError::Avb(error.to_string()))?;
    if !descriptor_values_match(&expected, &after.descriptors) {
        return Err(LtboxError::Avb(
            "Rebuilt root hash owner changed descriptor values".into(),
        ));
    }
    if after.header.rollback_index != before.header.rollback_index
        || after.header.rollback_index_location != before.header.rollback_index_location
        || after.header.flags != before.header.flags
        || after.algorithm_name != before.algorithm_name
        || after.public_key_sha1 != before.public_key_sha1
    {
        return Err(LtboxError::Avb(
            "Rebuilt root hash owner changed unrelated AVB metadata or signing key".into(),
        ));
    }
    Ok(())
}

/// Rebuild `vbmeta.img` from the original template, re-signing with
/// `vbmeta_key_spec` and updating the **chain partition descriptor public
/// keys** of `chain_partition_names` to `chain_key_spec`'s public key.
///
/// Needed when chained partitions (boot / recovery / vbmeta_system) are
/// re-signed with a non-stock key: their chain descriptors in vbmeta carry the
/// *public key*, so the bootloader rejects the images unless vbmeta points at
/// the new one. Unlike [`rebuild_vbmeta_with_partition_descriptors`], this
/// rewrites the chain descriptors; Hash / Hashtree descriptors and properties
/// of untouched partitions are preserved verbatim.
pub fn rebuild_vbmeta_with_chain_key_overrides(
    output_path: &Path,
    original_vbmeta_path: &Path,
    chain_partition_names: &[&str],
    chain_key_spec: &str,
    vbmeta_key_spec: &str,
    algorithm: &str,
) -> Result<()> {
    use avbtool_rs::builder::{ChainPartitionSpec, PropertySpec, VbmetaImageArgs};
    use avbtool_rs::info::DescriptorInfo;

    let info = avbtool_rs::image::inspect_avb_image(original_vbmeta_path)
        .map_err(|e| LtboxError::Avb(format!("inspect {}: {e}", original_vbmeta_path.display())))?;
    let blob = avbtool_rs::image::load_vbmeta_blob(original_vbmeta_path)
        .map_err(|e| LtboxError::Avb(format!("load vbmeta blob: {e}")))?;
    let pkmd = avbtool_rs::image::extract_public_key_metadata(&info.header, &blob)
        .map_err(|e| LtboxError::Avb(format!("public key metadata: {e}")))?;
    let new_pubkey = avbtool_rs::crypto::extract_public_key(chain_key_spec)
        .map_err(|e| LtboxError::Avb(format!("extract public key {chain_key_spec}: {e}")))?;

    let mut properties = Vec::new();
    let mut extra_descriptors = Vec::new();
    let mut chain_partitions = Vec::new();
    let mut updated_chain_keys = 0usize;
    for desc in &info.descriptors {
        match desc {
            DescriptorInfo::Property { key, value } => properties.push(PropertySpec {
                key: key.clone(),
                value: value.clone(),
            }),
            DescriptorInfo::ChainPartition {
                rollback_index_location,
                partition_name,
                public_key,
                flags,
            } => {
                let swap = chain_partition_names
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(partition_name));
                if swap {
                    updated_chain_keys += 1;
                }
                chain_partitions.push(ChainPartitionSpec {
                    partition_name: partition_name.clone(),
                    rollback_index_location: *rollback_index_location,
                    public_key: if swap {
                        new_pubkey.clone()
                    } else {
                        public_key.clone()
                    },
                    flags: *flags,
                });
            }
            // Carried through verbatim; KernelCmdline goes via extra_descriptors
            // (not `kernel_cmdlines`) so its flags survive.
            DescriptorInfo::Hash { .. }
            | DescriptorInfo::Hashtree { .. }
            | DescriptorInfo::KernelCmdline { .. }
            | DescriptorInfo::Unknown { .. } => extra_descriptors.push(desc.clone()),
        }
    }
    if updated_chain_keys != chain_partition_names.len() {
        return Err(LtboxError::Avb(format!(
            "rebuild_vbmeta_with_chain_key_overrides: expected to update chain keys for {} partitions {:?} but matched {} chain partition descriptors in {}",
            chain_partition_names.len(),
            chain_partition_names,
            updated_chain_keys,
            original_vbmeta_path.display()
        )));
    }

    let args = VbmetaImageArgs {
        algorithm_name: algorithm.to_string(),
        key_spec: Some(vbmeta_key_spec.to_string()),
        public_key_metadata: (!pkmd.is_empty()).then_some(pkmd),
        rollback_index: info.header.rollback_index,
        flags: info.header.flags,
        rollback_index_location: info.header.rollback_index_location,
        properties,
        kernel_cmdlines: Vec::new(),
        extra_descriptors,
        include_descriptors_from_images: Vec::new(),
        chain_partitions,
        release_string: Some(info.header.release_string.clone()),
        append_to_release_string: None,
        padding_size: 0,
    };
    avbtool_rs::builder::make_vbmeta_image(output_path, &args)
        .map_err(|e| LtboxError::Avb(format!("make_vbmeta_image: {e}")))?;
    preserve_original_vbmeta_size(output_path, original_vbmeta_path)?;
    Ok(())
}

fn preserve_original_vbmeta_size(output_path: &Path, original_vbmeta_path: &Path) -> Result<()> {
    let original_size = fs::metadata(original_vbmeta_path)?.len();
    let output_size = fs::metadata(output_path)?.len();
    if output_size < original_size {
        let file = fs::OpenOptions::new().write(true).open(output_path)?;
        file.set_len(original_size)?;
    }
    Ok(())
}

/// Restore the complete signed stock blob without changing its authenticated
/// descriptors. GBL tolerates the stale content digest but requires the stock
/// key and consistent authentication data. Keeps the repacked payload boundary
/// and relocates only the blob; never copies the stock payload size back.
pub fn preserve_stock_vbmeta(stock: &Path, repacked: &Path) -> Result<()> {
    use avbtool_rs::parser::{AVB_FOOTER_SIZE, AvbFooter};
    use std::io::{Read, Seek, SeekFrom, Write};
    let blob = avbtool_rs::image::load_vbmeta_blob(stock)
        .map_err(|e| LtboxError::Avb(format!("load stock vbmeta: {e}")))?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(repacked)?;
    let size = file.metadata()?.len();
    let footer_offset = size
        .checked_sub(AVB_FOOTER_SIZE)
        .ok_or_else(|| LtboxError::Avb("Repacked image has no AVB footer".into()))?;
    file.seek(SeekFrom::Start(footer_offset))?;
    let mut bytes = vec![0; AVB_FOOTER_SIZE as usize];
    file.read_exact(&mut bytes)?;
    let mut footer = AvbFooter::from_reader(bytes.as_slice())
        .map_err(|e| LtboxError::Avb(format!("read repacked footer: {e}")))?;
    let end = footer.vbmeta_offset.checked_add(blob.len() as u64);
    if footer.original_image_size > footer.vbmeta_offset
        || footer
            .vbmeta_offset
            .checked_add(footer.vbmeta_size)
            .is_none_or(|end| end > footer_offset)
        || end.is_none_or(|end| end > footer_offset)
    {
        return Err(LtboxError::Avb(
            "Stock vbmeta does not fit the repacked image footer layout".into(),
        ));
    }
    footer.vbmeta_size = blob.len() as u64;
    // Bounds are checked before the output is modified. Old metadata is truncated
    // and padding zero-filled so no stale bytes survive.
    file.set_len(footer.vbmeta_offset)?;
    file.seek(SeekFrom::Start(footer.vbmeta_offset))?;
    file.write_all(&blob)?;
    file.set_len(size)?;
    file.seek(SeekFrom::Start(footer_offset))?;
    file.write_all(&avbtool_rs::image::encode_footer(&footer))?;
    Ok(())
}

/// Add hash footer. `key_spec` follows [`resign_image`]; pass `None`
/// for the NONE-algorithm path (no signing).
pub fn add_hash_footer(
    image_path: &Path,
    info: &AvbImageInfo,
    key_spec: Option<&str>,
    new_rollback_index: Option<u64>,
) -> Result<()> {
    const HASH_ALGORITHM: &str = "sha256";

    let rollback = new_rollback_index.unwrap_or(info.rollback_index);
    // Must bail loudly: the Hash descriptor records partition_name and the
    // bootloader refuses a partition whose name differs.
    let name = info.partition_name.as_deref().ok_or_else(|| {
        LtboxError::Avb(format!(
            "Cannot add AVB hash footer to {}: no partition_name in AVB info (source image has no Hash/Hashtree descriptor)",
            image_path.display()
        ))
    })?;

    // avbtool-rs cannot carry arbitrary source descriptors through this footer
    // API; refuse while the destination is untouched rather than emit metadata
    // that no longer matches the source.
    if let Some(kind) = info.unreproducible_descriptor_kinds.first() {
        return Err(LtboxError::Avb(format!(
            "Cannot add AVB hash footer to {}: source image {} contains a {kind} descriptor, which the hash-footer rebuild cannot reproduce",
            image_path.display(),
            info.source_image_path.display()
        )));
    }
    if info.hash_descriptor_count > 1 {
        return Err(LtboxError::Avb(format!(
            "Cannot add AVB hash footer to {}: source image {} contains {} Hash descriptors, but the hash-footer rebuild can reproduce only one",
            image_path.display(),
            info.source_image_path.display(),
            info.hash_descriptor_count
        )));
    }
    if let Some(source_algorithm) = info.hash_descriptor_algorithm.as_deref()
        && source_algorithm != HASH_ALGORITHM
    {
        return Err(LtboxError::Avb(format!(
            "Cannot add AVB hash footer to {}: source image {} uses Hash descriptor algorithm {source_algorithm}, but the hash-footer rebuild writes {HASH_ALGORITHM}",
            image_path.display(),
            info.source_image_path.display()
        )));
    }
    info!("Adding AVB hash footer: partition={name}, rollback={rollback}");

    let salt_bytes = info.salt.clone();

    let properties = info
        .props
        .iter()
        .map(|(k, v)| avbtool_rs::builder::PropertySpec {
            key: k.clone(),
            value: v.clone(),
        })
        .collect();

    let args = avbtool_rs::footer::HashFooterArgs {
        partition_size: Some(info.partition_size),
        dynamic_partition_size: false,
        partition_name: name.to_string(),
        hash_algorithm: HASH_ALGORITHM.to_string(),
        salt: salt_bytes,
        chain_partitions: Vec::new(),
        algorithm_name: info.algorithm.clone(),
        key_spec: key_spec.map(|s| s.to_string()),
        public_key_metadata: None,
        rollback_index: rollback,
        flags: info.flags,
        rollback_index_location: info.rollback_index_location,
        properties,
        kernel_cmdlines: Vec::new(),
        include_descriptors_from_images: Vec::new(),
        release_string: None,
        append_to_release_string: None,
        output_vbmeta_image: None,
        do_not_append_vbmeta_image: false,
        use_persistent_digest: false,
        do_not_use_ab: false,
    };

    avbtool_rs::footer::add_hash_footer(image_path, &args)
        .map_err(|e| LtboxError::Avb(format!("add_hash_footer failed: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{key_map, region};

    fn write_hash_footer_fixture(
        image_path: &Path,
        hash_algorithm: &str,
        kernel_cmdlines: Vec<String>,
    ) {
        fs::write(image_path, vec![0x41; 4096]).unwrap();
        avbtool_rs::footer::add_hash_footer(
            image_path,
            &avbtool_rs::footer::HashFooterArgs {
                partition_size: Some(128 * 1024),
                dynamic_partition_size: false,
                partition_name: "vendor_boot".to_string(),
                hash_algorithm: hash_algorithm.to_string(),
                salt: Some(vec![0x11, 0x22]),
                chain_partitions: Vec::new(),
                algorithm_name: "NONE".to_string(),
                key_spec: None,
                public_key_metadata: None,
                rollback_index: 7,
                flags: 0,
                rollback_index_location: 3,
                properties: vec![avbtool_rs::builder::PropertySpec {
                    key: "com.android.build.vendor_boot.os_version".to_string(),
                    value: b"16".to_vec(),
                }],
                kernel_cmdlines,
                include_descriptors_from_images: Vec::new(),
                release_string: None,
                append_to_release_string: None,
                output_vbmeta_image: None,
                do_not_append_vbmeta_image: false,
                use_persistent_digest: false,
                do_not_use_ab: false,
            },
        )
        .unwrap();
    }

    #[test]
    fn stock_signed_blob_survives_payload_growth_without_resigning() {
        for key in ["testkey_rsa2048", "testkey_rsa4096"] {
            let temp = tempfile::tempdir().unwrap();
            let stock = temp.path().join("stock.img");
            let output = temp.path().join("output.img");
            write_hash_footer_fixture(&stock, "sha256", Vec::new());
            let mut info = extract_image_avb_info(&stock).unwrap();
            info.algorithm = algorithm_for_key_spec(key).unwrap();
            add_hash_footer(&stock, &info, Some(key), None).unwrap();
            let original = avbtool_rs::image::load_vbmeta_blob(&stock).unwrap();
            // A larger replacement payload moves the footer boundary. Rebuild
            // unsigned metadata to also exercise a different vbmeta size.
            let payload = vec![0x42; 8192];
            fs::write(&output, &payload).unwrap();
            info.algorithm = "NONE".into();
            add_hash_footer(&output, &info, None, None).unwrap();
            let before = avbtool_rs::image::inspect_avb_image(&output)
                .unwrap()
                .footer
                .unwrap();
            preserve_stock_vbmeta(&stock, &output).unwrap();
            assert_eq!(
                avbtool_rs::image::load_vbmeta_blob(&output).unwrap(),
                original
            );
            assert_eq!(&fs::read(&output).unwrap()[..payload.len()], &payload);
            let after = avbtool_rs::image::inspect_avb_image(&output)
                .unwrap()
                .footer
                .unwrap();
            assert_eq!(after.original_image_size, before.original_image_size);
            assert_eq!(after.vbmeta_offset, before.vbmeta_offset);
            assert_eq!(after.vbmeta_size, original.len() as u64);
            assert_eq!(fs::metadata(&output).unwrap().len(), info.partition_size);
        }
    }

    #[test]
    fn stock_blob_restore_rejects_missing_footer_without_modifying_payload() {
        let temp = tempfile::tempdir().unwrap();
        let stock = temp.path().join("stock.img");
        let output = temp.path().join("output.img");
        write_hash_footer_fixture(&stock, "sha256", Vec::new());
        let payload = vec![0x42; 8192];
        fs::write(&output, &payload).unwrap();
        assert!(preserve_stock_vbmeta(&stock, &output).is_err());
        assert_eq!(fs::read(&output).unwrap(), payload);
    }

    #[test]
    fn backup_verification_rejects_payload_corruption_that_metadata_inspection_accepts() {
        use std::io::{Seek, Write};
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("vendor_boot.img");
        let paired = dir.path().join("vbmeta.img");
        write_hash_footer_fixture(&image, "sha256", Vec::new());
        fs::write(
            &paired,
            avbtool_rs::image::load_vbmeta_blob(&image).unwrap(),
        )
        .unwrap();
        verify_root_backup(&image, Some(&paired), "vendor_boot").unwrap();
        let before = extract_image_avb_info(&image).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&image)
            .unwrap();
        file.seek(std::io::SeekFrom::Start(0)).unwrap();
        file.write_all(&[0x42]).unwrap();
        drop(file);
        let after = extract_image_avb_info(&image).unwrap();
        assert_eq!(before.rollback_index, after.rollback_index);
        assert_eq!(build_fingerprint(&before), build_fingerprint(&after));
        assert!(verify_root_backup(&image, Some(&paired), "vendor_boot").is_err());
        // Rebuild a valid but different payload while retaining the old pair.
        add_hash_footer(&image, &before, None, None).unwrap();
        verify_root_backup(&image, None, "vendor_boot").unwrap();
        assert!(verify_root_backup(&image, Some(&paired), "vendor_boot").is_err());
    }

    #[test]
    fn build_fingerprint_reads_property_descriptor() {
        let make = |props: Vec<(String, Vec<u8>)>| AvbImageInfo {
            partition_size: 0,
            original_image_size: None,
            algorithm: "SHA256_RSA4096".into(),
            rollback_index: 0,
            rollback_index_location: 0,
            flags: 0,
            partition_name: Some("init_boot".into()),
            salt: None,
            public_key_sha1: None,
            props,
            source_image_path: PathBuf::from("init_boot.img"),
            hash_descriptor_algorithm: Some("sha256".into()),
            hash_descriptor_count: 1,
            unreproducible_descriptor_kinds: Vec::new(),
        };
        let mut value = b"Lenovo/TB323FU/TB323FU:16/BQ2A/x:user/release-keys".to_vec();
        value.push(0); // trailing NUL like the on-disk descriptor
        let info = make(vec![
            (
                "com.android.build.init_boot.os_version".into(),
                b"16".to_vec(),
            ),
            ("com.android.build.init_boot.fingerprint".into(), value),
        ]);
        assert_eq!(
            build_fingerprint(&info).as_deref(),
            Some("Lenovo/TB323FU/TB323FU:16/BQ2A/x:user/release-keys")
        );
        // No fingerprint property → None.
        assert_eq!(
            build_fingerprint(&make(vec![(
                "com.android.build.init_boot.os_version".into(),
                b"16".to_vec()
            )])),
            None
        );
    }

    #[test]
    fn build_fingerprint_prefers_a_prop_naming_a_known_model() {
        let info = |props: &[(&str, &str)]| AvbImageInfo {
            partition_size: 0,
            original_image_size: None,
            algorithm: "SHA256_RSA4096".into(),
            rollback_index: 0,
            rollback_index_location: 0,
            flags: 0,
            partition_name: None,
            salt: None,
            public_key_sha1: None,
            props: props
                .iter()
                .map(|(k, v)| (format!("com.android.build.{k}"), v.as_bytes().to_vec()))
                .collect(),
            source_image_path: PathBuf::from("vbmeta.img"),
            hash_descriptor_algorithm: None,
            hash_descriptor_count: 0,
            unreproducible_descriptor_kinds: Vec::new(),
        };
        let baldur = "Lenovo/lineage_baldur/baldur:17/CP2A/eng:userdebug/test-keys";
        let stock = "Lenovo/TB323FU/TB323FU:17/CP2A/x:user/release-keys";
        let pvmfw = "qti/TB323FU/TB323FU:16/BQ2A/x:user/release-keys";
        let unknown = "Lenovo/lineage_foo/foo:17/CP2A/eng:userdebug/test-keys";

        // The system fingerprint names a model: it wins even when listed last.
        assert_eq!(
            build_fingerprint(&info(&[
                ("boot.fingerprint", unknown),
                ("system.fingerprint", stock),
            ]))
            .as_deref(),
            Some(stock)
        );
        // System names no known model; the first prop that does is chosen.
        assert_eq!(
            build_fingerprint(&info(&[
                ("system.fingerprint", unknown),
                ("system_ext.fingerprint", unknown),
                ("pvmfw.fingerprint", pvmfw),
                ("dataext.fingerprint", stock),
            ]))
            .as_deref(),
            Some(pvmfw)
        );
        // A codename counts as naming a known model.
        assert_eq!(
            build_fingerprint(&info(&[
                ("system.fingerprint", unknown),
                ("product.fingerprint", baldur),
            ]))
            .as_deref(),
            Some(baldur)
        );
        // Nothing names a model: system, regardless of order.
        assert_eq!(
            build_fingerprint(&info(&[
                ("boot.fingerprint", "other/x/x:1/b"),
                ("system.fingerprint", unknown),
            ]))
            .as_deref(),
            Some(unknown)
        );
        // No system prop: the first prop. Empty values are ignored.
        assert_eq!(
            build_fingerprint(&info(&[
                ("boot.fingerprint", " "),
                ("vendor.fingerprint", unknown),
                ("dtbo.fingerprint", "other/x/x:1/b"),
            ]))
            .as_deref(),
            Some(unknown)
        );
    }

    #[test]
    fn image_info_report_accepts_non_avb_img() {
        let tmp = tempfile::tempdir().unwrap();
        let image = tmp.path().join("plain.img");
        fs::write(&image, [0u8; 16]).unwrap();

        let report = image_info_report(&[image]).unwrap();

        assert!(report.contains("AVB image type:"));
        assert!(report.contains("No AVB metadata found."));
    }

    #[test]
    fn image_info_report_requires_selection() {
        let err = image_info_report(&[]).unwrap_err().to_string();
        assert!(err.contains("No image files selected"));
    }

    #[test]
    fn add_hash_footer_rejects_unreproducible_descriptor_before_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let image = tmp.path().join("vendor_boot.img");
        write_hash_footer_fixture(&image, "sha256", vec!["console=ttyS0".to_string()]);
        let info = extract_image_avb_info(&image).unwrap();
        let original = fs::read(&image).unwrap();

        let err = add_hash_footer(&image, &info, None, None).unwrap_err();

        assert!(matches!(err, LtboxError::Avb(_)));
        let message = err.to_string();
        assert!(message.contains(image.to_string_lossy().as_ref()));
        assert!(message.contains("KernelCmdline"));
        assert_eq!(fs::read(&image).unwrap(), original);
    }

    #[test]
    fn add_hash_footer_rejects_mismatched_hash_algorithm_before_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let image = tmp.path().join("vendor_boot.img");
        write_hash_footer_fixture(&image, "sha512", Vec::new());
        let info = extract_image_avb_info(&image).unwrap();
        let original = fs::read(&image).unwrap();

        let err = add_hash_footer(&image, &info, None, None).unwrap_err();

        assert!(matches!(err, LtboxError::Avb(_)));
        let message = err.to_string();
        assert!(message.contains(image.to_string_lossy().as_ref()));
        assert!(message.contains("sha512"));
        assert!(message.contains("sha256"));
        assert_eq!(fs::read(&image).unwrap(), original);
    }

    #[test]
    fn add_hash_footer_accepts_hash_and_property_descriptors() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("vendor_boot.stock.img");
        let output = tmp.path().join("vendor_boot.patched.img");
        write_hash_footer_fixture(&source, "sha256", Vec::new());
        let info = extract_image_avb_info(&source).unwrap();
        fs::write(&output, vec![0x41; 4096]).unwrap();

        add_hash_footer(&output, &info, None, None).unwrap();

        let rebuilt = avbtool_rs::image::inspect_avb_image(&output).unwrap();
        assert_eq!(rebuilt.descriptors.len(), 2);
        assert!(matches!(
            &rebuilt.descriptors[0],
            avbtool_rs::info::DescriptorInfo::Hash {
                hash_algorithm,
                partition_name,
                salt,
                ..
            } if hash_algorithm == "sha256"
                && partition_name == "vendor_boot"
                && salt == &[0x11, 0x22]
        ));
        assert!(matches!(
            &rebuilt.descriptors[1],
            avbtool_rs::info::DescriptorInfo::Property { key, value }
                if key == "com.android.build.vendor_boot.os_version" && value == b"16"
        ));
    }

    #[test]
    fn preserve_original_vbmeta_size_pads_short_rebuild_output() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("vbmeta.img");
        let output = tmp.path().join("vbmeta.rebuilt.img");
        fs::write(&original, vec![0u8; 8192]).unwrap();
        fs::write(&output, vec![1u8; 4096]).unwrap();

        preserve_original_vbmeta_size(&output, &original).unwrap();

        assert_eq!(fs::metadata(&output).unwrap().len(), 8192);
        let data = fs::read(&output).unwrap();
        assert!(data[..4096].iter().all(|b| *b == 1));
        assert!(data[4096..].iter().all(|b| *b == 0));
    }

    #[test]
    fn preserve_original_vbmeta_size_never_truncates_larger_output() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("vbmeta.img");
        let output = tmp.path().join("vbmeta.rebuilt.img");
        fs::write(&original, vec![0u8; 4096]).unwrap();
        fs::write(&output, vec![1u8; 8192]).unwrap();

        preserve_original_vbmeta_size(&output, &original).unwrap();

        assert_eq!(fs::metadata(&output).unwrap().len(), 8192);
    }

    #[test]
    fn real_firmware_avb_matrix_when_available() {
        let Some(dir) = std::env::var_os("LTBOX_REAL_FIRMWARE_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let original_vbmeta = dir.join("vbmeta.img");
        let original_vendor_boot = dir.join("vendor_boot.img");
        let original_boot = dir.join("boot.img");
        let original_vbmeta_system = dir.join("vbmeta_system.img");
        if !original_vbmeta.exists()
            || !original_vendor_boot.exists()
            || !original_boot.exists()
            || !original_vbmeta_system.exists()
        {
            return;
        }

        for target_rollback in [None, Some(1_800_000_000u64)] {
            let tmp = tempfile::tempdir().unwrap();
            let vbmeta = tmp.path().join("vbmeta.img");
            let vendor_boot = tmp.path().join("vendor_boot.img");
            let patched_vendor_boot = tmp.path().join("vendor_boot.patched.img");
            let rebuilt_vbmeta = tmp.path().join("vbmeta.rebuilt.img");
            let boot = tmp.path().join("boot.img");
            let vbmeta_system = tmp.path().join("vbmeta_system.img");
            fs::copy(&original_vbmeta, &vbmeta).unwrap();
            fs::copy(&original_vendor_boot, &vendor_boot).unwrap();
            fs::copy(&original_boot, &boot).unwrap();
            fs::copy(&original_vbmeta_system, &vbmeta_system).unwrap();

            for image in [&boot, &vbmeta_system] {
                let info = extract_image_avb_info(image).unwrap();
                if let Some(target) = target_rollback {
                    let key_spec = key_map::key_spec_for_pubkey(info.public_key_sha1.as_deref())
                        .expect("real fixture rollback key should be known");
                    resign_image(image, key_spec, &info.algorithm, Some(target)).unwrap();
                    assert_eq!(
                        extract_image_avb_info(image).unwrap().rollback_index,
                        target
                    );
                } else {
                    assert_eq!(
                        extract_image_avb_info(image).unwrap().rollback_index,
                        info.rollback_index
                    );
                }
            }

            let vendor_boot_info = extract_image_avb_info(&vendor_boot).unwrap();
            let patterns = region::RegionPatternSet::default();
            let replaced = region::patch_vendor_boot(
                &vendor_boot,
                &patched_vendor_boot,
                region::RegionTarget::Prc,
                &patterns.prc_patterns,
                &patterns.row_patterns,
            )
            .unwrap();
            assert!(replaced > 0);
            add_hash_footer(&patched_vendor_boot, &vendor_boot_info, None, None).unwrap();

            let vbmeta_info = extract_image_avb_info(&vbmeta).unwrap();
            let key_spec = key_map::key_spec_for_pubkey(vbmeta_info.public_key_sha1.as_deref())
                .expect("real fixture vbmeta key should be known");
            rebuild_vbmeta_with_partition_descriptors(
                &rebuilt_vbmeta,
                &vbmeta,
                &[patched_vendor_boot.as_path()],
                key_spec,
                Some(&vbmeta_info.algorithm),
            )
            .unwrap();

            assert_eq!(
                fs::metadata(&rebuilt_vbmeta).unwrap().len(),
                fs::metadata(&vbmeta).unwrap().len()
            );
            let report = image_info_report(&[rebuilt_vbmeta]).unwrap();
            assert!(report.contains("vendor_boot"));
        }
    }
}
