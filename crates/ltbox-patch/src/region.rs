//! Region patching — binary pattern replacement for PRC↔ROW conversion.
//!
//! Patches vendor_boot.img and devinfo/persist country codes.

use fs_err as fs;
use std::path::{Path, PathBuf};

use crate::avb::AvbImageInfo;
use crate::{avb, key_map};
use ltbox_core::{
    LtboxError, Result,
    model::{fingerprint_model_match, fingerprint_names_known_model, known_model},
};
use tracing::info;

/// Outcome of validating an image against a device model.
/// Matches v2 `_validate_device_model` three-way result: the fingerprint
/// may match, mismatch, or be absent entirely (older firmware).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelValidation {
    /// Fingerprint present and matches `device_model`. Safe to proceed.
    Match { fingerprint: String },
    /// Fingerprint present but does NOT match `device_model`. The
    /// firmware is for a different model — caller should abort.
    Mismatch {
        fingerprint: String,
        device_model: String,
    },
    /// Fingerprint present but names no known model (a custom ROM that uses
    /// an unrecognized codename), while `device_model` is a known model. The
    /// fingerprint cannot contradict the device; the caller may proceed as the
    /// detected model, and must say so.
    Unidentified {
        fingerprint: String,
        device_model: String,
    },
    /// No fingerprint property in the AVB image. Caller decides — v2
    /// logged a warning and skipped validation.
    Missing,
}

/// Check an AVB-extracted image against the ADB-reported device model. Reads the
/// build fingerprint via [`avb::build_fingerprint`] (preferring
/// `com.android.build.system.fingerprint`, so a vbmeta_system image works
/// directly) and matches the device model as a bounded token.
///
/// A fingerprint that names no known model is [`ModelValidation::Unidentified`]
/// when `device_model` is a known model, and a mismatch otherwise. One that
/// names a different known model is always a mismatch.
///
/// Spaces in `device_model` are stripped to tolerate
/// `"TB 320FC"`-style reads from `ro.product.model`. The shared matcher also
/// accepts the model token reported by LAVIE Tab 9QHD1 as TB320FC-equivalent.
pub fn validate_device_model(info: &AvbImageInfo, device_model: &str) -> ModelValidation {
    classify_fingerprint(avb::build_fingerprint(info), device_model)
}

fn classify_fingerprint(fingerprint: Option<String>, device_model: &str) -> ModelValidation {
    let normalized = device_model.replace(' ', "");

    match fingerprint {
        None => ModelValidation::Missing,
        Some(fp) if normalized.is_empty() => ModelValidation::Match { fingerprint: fp },
        Some(fp) if fingerprint_model_match(&fp, &normalized) => {
            ModelValidation::Match { fingerprint: fp }
        }
        Some(fp) if !fingerprint_names_known_model(&fp) && known_model(&normalized).is_some() => {
            ModelValidation::Unidentified {
                fingerprint: fp,
                device_model: normalized,
            }
        }
        Some(fp) => ModelValidation::Mismatch {
            fingerprint: fp,
            device_model: normalized,
        },
    }
}

/// Direction of region conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionTarget {
    Prc,
    Row,
}

/// Detect a firmware's region from the `product_region` node in its vendor_boot
/// device-tree. TB323FU (and others) carry no `_PRC`/`_ROW` token in the AVB
/// fingerprint, but the DTB has an FDT node named `product_region` whose
/// properties carry a `PRC` / `ROW` value. The flattened device-tree lays the
/// node out as `FDT_BEGIN_NODE "product_region\0"` (padded) followed by its
/// `FDT_PROP` entries (`token, len, nameoff, value`), so this walks that node's
/// properties and returns the first `PRC` / `ROW` value. Occurrences of the same
/// string in the FDT strings block are not followed by an `FDT_PROP` token, so
/// they are skipped. Returns `None` when the marker is absent / unreadable.
pub fn detect_product_region(vendor_boot_path: &Path) -> Option<RegionTarget> {
    const FDT_PROP: u32 = 0x0000_0003;
    let data = fs::read(vendor_boot_path).ok()?;
    let name = b"product_region\0";
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let mut from = 0usize;
    while let Some(rel) = find_subslice(&data[from..], name) {
        let node_name_start = from + rel;
        // Walk the node's FDT_PROP entries from just past the (4-aligned) name.
        let mut pos = (node_name_start + name.len() + 3) & !3;
        for _ in 0..16 {
            if pos + 8 > data.len() || be32(&data[pos..]) != FDT_PROP {
                break; // END_NODE / nested BEGIN_NODE / strings block -> stop
            }
            let len = be32(&data[pos + 4..]) as usize;
            let value_start = pos + 12; // token + len + nameoff
            if value_start + len > data.len() {
                break;
            }
            let value = &data[value_start..value_start + len];
            match value.split(|&b| b == 0).next().unwrap_or(value) {
                b"PRC" => return Some(RegionTarget::Prc),
                b"ROW" => return Some(RegionTarget::Row),
                _ => {}
            }
            pos = (value_start + len + 3) & !3;
        }
        from = node_name_start + name.len();
    }
    None
}

/// First offset of `needle` in `haystack`, if present.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Vendor-boot region markers. `prc_patterns` are applied when the target is
/// ROW (PRC -> ROW); `row_patterns` are applied when the target is PRC
/// (ROW -> PRC).
#[derive(Debug, Clone)]
pub struct RegionPatternSet {
    pub prc_patterns: Vec<(Vec<u8>, Vec<u8>)>,
    pub row_patterns: Vec<(Vec<u8>, Vec<u8>)>,
}

impl Default for RegionPatternSet {
    fn default() -> Self {
        let prc_dot = b".PRC".to_vec();
        let prc_i = b"IPRC".to_vec();
        let row_dot = b".ROW".to_vec();
        let row_i = b"IROW".to_vec();
        Self {
            prc_patterns: vec![
                (prc_dot.clone(), row_dot.clone()),
                (prc_i.clone(), row_i.clone()),
            ],
            row_patterns: vec![(row_dot, prc_dot), (row_i, prc_i)],
        }
    }
}

/// Region markers currently present in `vendor_boot.img`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedRegion {
    Prc,
    Row,
    Mixed,
    Unknown,
}

impl DetectedRegion {
    fn matches_target(self, target: RegionTarget) -> bool {
        matches!(
            (self, target),
            (Self::Prc, RegionTarget::Prc) | (Self::Row, RegionTarget::Row)
        )
    }
}

/// Final region-converted vendor_boot/vbmeta AVB images written by
/// [`build_region_converted_avb_images`].
#[derive(Debug, Clone)]
pub struct RegionAvbOutput {
    pub vendor_boot: PathBuf,
    pub vbmeta: PathBuf,
    pub source_region: DetectedRegion,
    pub target: RegionTarget,
    pub replacement_count: usize,
}

/// Builder result. `Skipped` means the output folder was cleared and no files
/// were emitted because the source already matched the requested target.
#[derive(Debug, Clone)]
pub enum RegionAvbBuild {
    Built(RegionAvbOutput),
    Skipped {
        source_region: DetectedRegion,
        target: RegionTarget,
    },
}

/// Coarse build boundaries exposed to callers that present progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionBuildStage {
    Inspect,
    PatchVendorBoot,
    RebuildVbmeta,
}

/// Build the AVB-valid `vendor_boot.img` + `vbmeta.img` pair for region
/// conversion.
///
/// This is the v3 equivalent of v2 `convert_region_images()`: copy
/// `vendor_boot`, patch region bytes, add a new hash footer using the original
/// AVB parameters, then rebuild `vbmeta`, refreshing matching descriptors from
/// the patched `vendor_boot` image.
pub fn build_region_converted_avb_images(
    firmware_dir: &Path,
    output_dir: &Path,
    target: RegionTarget,
    patterns: &RegionPatternSet,
    key_override: Option<&str>,
) -> Result<RegionAvbBuild> {
    build_region_converted_avb_images_with_progress(
        firmware_dir,
        output_dir,
        target,
        patterns,
        key_override,
        |_| {},
    )
}

/// Build region-converted vendor_boot/vbmeta AVB images while reporting stable coarse stages.
pub fn build_region_converted_avb_images_with_progress(
    firmware_dir: &Path,
    output_dir: &Path,
    target: RegionTarget,
    patterns: &RegionPatternSet,
    key_override: Option<&str>,
    mut on_stage: impl FnMut(RegionBuildStage),
) -> Result<RegionAvbBuild> {
    if output_dir.exists() {
        fs::remove_dir_all(output_dir).map_err(|e| {
            LtboxError::Patch(format!(
                "Cannot clear region output {}: {e}",
                output_dir.display()
            ))
        })?;
    }
    fs::create_dir_all(output_dir).map_err(|e| {
        LtboxError::Patch(format!(
            "Cannot create region output {}: {e}",
            output_dir.display()
        ))
    })?;

    let vendor_boot_src = firmware_dir.join("vendor_boot.img");
    let vbmeta_src = firmware_dir.join("vbmeta.img");
    if !vendor_boot_src.is_file() {
        return Err(LtboxError::FileNotFound(format!(
            "{}",
            vendor_boot_src.display()
        )));
    }
    if !vbmeta_src.is_file() {
        return Err(LtboxError::FileNotFound(format!(
            "{}",
            vbmeta_src.display()
        )));
    }

    on_stage(RegionBuildStage::Inspect);
    let vendor_boot_data = fs::read(&vendor_boot_src).map_err(|e| {
        LtboxError::Patch(format!("Cannot read {}: {e}", vendor_boot_src.display()))
    })?;
    let source_region = detect_region_in_data(&vendor_boot_data, patterns);
    info!("Region source={source_region:?}, target={target:?}");
    if source_region.matches_target(target) {
        info!("Source already matches target; output folder left empty");
        return Ok(RegionAvbBuild::Skipped {
            source_region,
            target,
        });
    }

    on_stage(RegionBuildStage::PatchVendorBoot);
    let vendor_boot_out = output_dir.join("vendor_boot.img");
    let replacement_count = patch_vendor_boot(
        &vendor_boot_src,
        &vendor_boot_out,
        target,
        &patterns.prc_patterns,
        &patterns.row_patterns,
    )?;
    info!("Region replacements: {replacement_count}");
    if replacement_count == 0 {
        let _ = fs::remove_file(&vendor_boot_out);
        return Err(LtboxError::Patch(format!(
            "No region patterns were replaced in {} (source={source_region:?}, target={target:?})",
            vendor_boot_src.display()
        )));
    }

    let vendor_boot_info = avb::extract_image_avb_info(&vendor_boot_src)?;
    avb::add_hash_footer(&vendor_boot_out, &vendor_boot_info, None, None)?;
    info!(
        "Added AVB hash footer to vendor_boot: {}",
        vendor_boot_out.display()
    );

    on_stage(RegionBuildStage::RebuildVbmeta);
    let vbmeta_info = avb::extract_image_avb_info(&vbmeta_src)?;
    let vbmeta_out = output_dir.join("vbmeta.img");
    // `key_override` (testkey) re-signs the rebuilt vbmeta with that key rather
    // than the firmware's own KEY_MAP key — the key2 cross-region flash passes
    // the testkey so the converted vbmeta matches the device's root of trust.
    // Without an override, fall back to the firmware's signing key.
    let resolved = match key_override {
        Some(spec) => Some(spec),
        None => key_map::key_spec_for_signed_pubkey(vbmeta_info.public_key_sha1.as_deref())
            .map_err(|key| {
                LtboxError::Avb(key_map::unresolved_signing_key_error("vbmeta.img", &key))
            })?,
    };
    match resolved {
        Some(key_spec) => {
            // Keep the rebuild algorithm consistent with the signing key: a key
            // override (cross-region testkey) may differ in size from the source
            // vbmeta, so derive the algorithm from the override key itself —
            // otherwise avbtool rejects the key/algorithm mismatch.
            let algorithm = match key_override {
                Some(spec) => avb::algorithm_for_key_spec(spec).ok_or_else(|| {
                    LtboxError::Avb(format!("unknown AVB algorithm for key override {spec}"))
                })?,
                None => vbmeta_info.algorithm.clone(),
            };
            avb::rebuild_vbmeta_with_partition_descriptors(
                &vbmeta_out,
                &vbmeta_src,
                &[vendor_boot_out.as_path()],
                key_spec,
                Some(algorithm.as_str()),
            )?;
            info!("Refreshed vbmeta descriptors: {}", vbmeta_out.display());
        }
        None => {
            fs::copy(&vbmeta_src, &vbmeta_out)?;
            info!("vbmeta is unsigned; copied stock blob");
        }
    }

    Ok(RegionAvbBuild::Built(RegionAvbOutput {
        vendor_boot: vendor_boot_out,
        vbmeta: vbmeta_out,
        source_region,
        target,
        replacement_count,
    }))
}

/// Patch vendor_boot.img for region conversion.
/// Returns the number of pattern replacements made.
pub fn patch_vendor_boot(
    input: &Path,
    output: &Path,
    target: RegionTarget,
    prc_patterns: &[(Vec<u8>, Vec<u8>)],
    row_patterns: &[(Vec<u8>, Vec<u8>)],
) -> Result<usize> {
    let mut data = fs::read(input)
        .map_err(|e| LtboxError::Patch(format!("Cannot read {}: {e}", input.display())))?;

    let replacements: &[(Vec<u8>, Vec<u8>)] = match target {
        RegionTarget::Row => prc_patterns,
        RegionTarget::Prc => row_patterns,
    };

    let mut total_count = 0;
    for (from, to) in replacements {
        let count = replace_in_place(&mut data, from, to)?;
        if count > 0 {
            info!("Replacing pattern {} ({count} occurrences)", hex_str(from));
            total_count += count;
        }
    }

    fs::write(output, &data)
        .map_err(|e| LtboxError::Patch(format!("Cannot write {}: {e}", output.display())))?;

    Ok(total_count)
}

fn detect_region_in_data(data: &[u8], patterns: &RegionPatternSet) -> DetectedRegion {
    let prc_count: usize = patterns
        .prc_patterns
        .iter()
        .map(|(from, _)| count_occurrences(data, from))
        .sum();
    let row_count: usize = patterns
        .row_patterns
        .iter()
        .map(|(from, _)| count_occurrences(data, from))
        .sum();

    match (prc_count > 0, row_count > 0) {
        (true, false) => DetectedRegion::Prc,
        (false, true) => DetectedRegion::Row,
        (true, true) => DetectedRegion::Mixed,
        (false, false) => DetectedRegion::Unknown,
    }
}

/// Canonical Lenovo country / sales-region codes embedded in devinfo /
/// persist / oemowninfo. Single source of truth for `detect_country_code`
/// scans and the GUI country picker.
pub const KNOWN_COUNTRY_CODES: &[&str] = &[
    "CN", "KR", "JP", "US", "GB", "DE", "FR", "IT", "ES", "NL", "AT", "BE", "BG", "HR", "CY", "CZ",
    "DK", "EE", "FI", "GR", "HU", "IE", "LV", "LT", "LU", "MT", "PL", "PT", "RO", "SK", "SI", "SE",
    "AU", "CA", "IN", "RU", "BR", "MX", "SA", "AE", "WW",
];

/// EU member codes — `patch_country_code` writes the `XE` (vs `XX`) suffix
/// for these. Subset of [`KNOWN_COUNTRY_CODES`].
pub const EU_COUNTRY_CODES: &[&str] = &[
    "AT", "BE", "BG", "HR", "CY", "CZ", "DK", "EE", "FI", "FR", "DE", "GR", "HU", "IE", "IT", "LV",
    "LT", "LU", "MT", "NL", "PL", "PT", "RO", "SK", "SI", "ES", "SE",
];

/// Detect country code in a binary image (devinfo/persist/oemowninfo).
/// Scans for patterns like "CNXX", "KRXX", "CNXE" etc.
///
/// ext4 data-block size — the persist country file's content starts on a block.
const EXT4_BLOCK: usize = 4096;

/// True when a `field_only` country match at byte offset `i` (length `n`) is a
/// real stored field rather than embedded log text. The country code is a
/// NUL-terminated string at the START of an ext4 data block, so require a
/// LEADING boundary — block-aligned, or a NUL immediately before (file fields
/// are NUL-delimited) — AND a trailing NUL. Log lines such as `Update country
/// code CNXX\n` are mid-block and `code `-prefixed, so they fail the leading
/// test even if a trailing NUL (zero slack) happened to follow. Verified on
/// TB520FU / TB321FU persist dumps: the real field is 4K-aligned (TB520FU also
/// NUL-preceded, TB321FU preceded by binary), the log hits are neither.
fn is_country_field(data: &[u8], i: usize, n: usize) -> bool {
    let leading = i.is_multiple_of(EXT4_BLOCK) || data[i - 1] == 0; // i==0 ⇒ aligned
    let trailing = i + n == data.len() || data[i + n] == 0;
    leading && trailing
}

/// `field_only` restricts matches to a real country field — a NUL-terminated
/// string at an ext4 block boundary (see [`is_country_field`]). `persist` is an
/// ext4 image whose log files also contain strings like `Update country code
/// CNXX`, so a blind scan would detect (and a blind patch would corrupt) those
/// log entries; `field_only` matches only the real country-code file.
pub fn detect_country_code(
    image_path: &Path,
    known_codes: &[&str],
    field_only: bool,
) -> Result<Option<String>> {
    let data = fs::read(image_path)
        .map_err(|e| LtboxError::Patch(format!("Cannot read {}: {e}", image_path.display())))?;

    for code in known_codes {
        let code_bytes = code.as_bytes();
        // Stock firmware isn't consistent with the EU suffix rule — accept either.
        for suffix in [b"XE", b"XX"] {
            let mut pattern = code_bytes.to_vec();
            pattern.extend_from_slice(suffix);
            let found = data.windows(pattern.len()).enumerate().any(|(i, w)| {
                w == pattern.as_slice()
                    && (!field_only || is_country_field(&data, i, pattern.len()))
            });
            if found {
                return Ok(Some(code.to_string()));
            }
        }
    }

    Ok(None)
}

/// Replace `from` with `to` (equal length) only where the match is a real
/// country field — a NUL-terminated string at an ext4 block boundary (see
/// [`is_country_field`]). Skips matches embedded in text (e.g. persist log lines
/// like `Update country code CNXX\n`), so only the real country-code file is
/// edited.
fn replace_country_field(data: &mut [u8], from: &[u8], to: &[u8]) -> usize {
    debug_assert_eq!(from.len(), to.len());
    let n = from.len();
    if n == 0 || data.len() < n {
        return 0;
    }
    let mut count = 0;
    let mut i = 0;
    while i + n <= data.len() {
        if &data[i..i + n] == from && is_country_field(data, i, n) {
            data[i..i + n].copy_from_slice(to);
            count += 1;
            i += n;
        } else {
            i += 1;
        }
    }
    count
}

/// Patch country code in a binary image.
/// Returns true if any replacement was made. `field_only` (see
/// [`detect_country_code`]) edits only the null-terminated country field, so a
/// persist ext4 image's log entries are left untouched.
pub fn patch_country_code(
    input: &Path,
    output: &Path,
    old_code: &str,
    new_code: &str,
    eu_codes: &[&str],
    field_only: bool,
) -> Result<bool> {
    let mut data = fs::read(input)
        .map_err(|e| LtboxError::Patch(format!("Cannot read {}: {e}", input.display())))?;

    // Write-suffix is EU-aware: EU new_code → `XE`, else `XX`.
    let new_suffix = if eu_codes.contains(&new_code) {
        "XE"
    } else {
        "XX"
    };
    let to = format!("{new_code}{new_suffix}");

    // Field replacement is in-place, so the old/new patterns must be the same
    // length. Return a recoverable error rather than panicking in
    // `copy_from_slice` if the codes differ in length.
    if field_only && old_code.len() != new_code.len() {
        return Err(LtboxError::Patch(format!(
            "field-only country patch needs equal-length codes ({old_code} -> {new_code})"
        )));
    }

    // Scan both `XE` and `XX` for old_code — stock Lenovo firmware mixes them
    // (e.g. `FRXX` in the wild). Widen unconditionally; false positives are free.
    let mut total_count = 0usize;
    for old_suffix in ["XE", "XX"] {
        let from = format!("{old_code}{old_suffix}");
        let n = if field_only {
            replace_country_field(&mut data, from.as_bytes(), to.as_bytes())
        } else {
            replace_in_place(&mut data, from.as_bytes(), to.as_bytes())?
        };
        if n > 0 {
            info!("Replacing country code {from} → {to} ({n} occurrences)");
            total_count += n;
        }
    }
    if total_count == 0 {
        fs::copy(input, output).map_err(|e| LtboxError::Patch(format!("Copy failed: {e}")))?;
        return Ok(false);
    }

    // Bail if expected suffix isn't present — better than silently shipping wrong-suffix devinfo.
    let written = count_occurrences(&data, to.as_bytes());
    if written == 0 {
        return Err(LtboxError::Patch(format!(
            "Post-patch verification failed: expected `{to}` not present in output"
        )));
    }

    fs::write(output, &data)
        .map_err(|e| LtboxError::Patch(format!("Cannot write {}: {e}", output.display())))?;

    Ok(true)
}

/// Replace the Xiaoxin Pro 13 / Idea Tab Pro Gen 2 `proinfo` channel field
/// from `consumer` to
/// `commercial`. The shorter source token must be NUL-delimited and have two
/// further NUL bytes after its terminator, leaving a NUL terminator after the
/// longer replacement. An already-`commercial` image is a successful no-op.
pub fn patch_proinfo_channel(input: &Path, output: &Path) -> Result<bool> {
    const FROM: &[u8] = b"consumer";
    const TO: &[u8] = b"commercial";

    let mut data = fs::read(input)
        .map_err(|e| LtboxError::Patch(format!("Cannot read {}: {e}", input.display())))?;

    let commercial_present = data.windows(TO.len()).enumerate().any(|(i, window)| {
        window == TO && i > 0 && data[i - 1] == 0 && data.get(i + TO.len()) == Some(&0)
    });

    let consumer = data
        .windows(FROM.len())
        .enumerate()
        .find_map(|(i, window)| {
            (window == FROM && i > 0 && data[i - 1] == 0 && data.get(i + FROM.len()) == Some(&0))
                .then_some(i)
        });

    let Some(offset) = consumer else {
        if commercial_present {
            if input != output {
                fs::copy(input, output)
                    .map_err(|e| LtboxError::Patch(format!("Copy failed: {e}")))?;
            }
            return Ok(false);
        }
        return Err(LtboxError::Patch(
            "proinfo channel field not found".to_string(),
        ));
    };

    let terminator = offset + FROM.len();
    if data.get(terminator + 1) != Some(&0) || data.get(terminator + 2) != Some(&0) {
        return Err(LtboxError::Patch(
            "proinfo consumer channel lacks two trailing NUL bytes".to_string(),
        ));
    }

    data[offset..offset + TO.len()].copy_from_slice(TO);
    if &data[offset..offset + TO.len()] != TO || data.get(offset + TO.len()) != Some(&0) {
        return Err(LtboxError::Patch(
            "proinfo channel post-patch verification failed".to_string(),
        ));
    }

    fs::write(output, &data)
        .map_err(|e| LtboxError::Patch(format!("Cannot write {}: {e}", output.display())))?;
    Ok(true)
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || needle.len() > haystack.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

/// In-place same-length pattern substitution; returns the replacement count.
///
/// Mutates `data` directly (no per-pattern clone) and counts matches in the
/// same pass, so a multi-pattern run over a large `vendor_boot` makes one
/// scan per pattern instead of a count scan plus a clone-and-replace scan.
///
/// Unequal-length replacement would shift every byte after the match and
/// break AVB digests of the containing image — safer to refuse than to let
/// the caller ship a corrupt vendor_boot. Python v2 used `bytes.replace`,
/// which accepts unequal lengths silently; returning an error here instead
/// of asserting avoids taking down the GUI thread over a user-edited
/// `config.json`.
fn replace_in_place(data: &mut [u8], from: &[u8], to: &[u8]) -> Result<usize> {
    if from.len() != to.len() {
        return Err(LtboxError::Patch(format!(
            "region pattern length mismatch: from={} to={}",
            from.len(),
            to.len()
        )));
    }
    if from.is_empty() || from.len() > data.len() {
        return Ok(0);
    }
    let mut count = 0;
    let mut pos = 0;
    while pos + from.len() <= data.len() {
        if &data[pos..pos + from.len()] == from {
            data[pos..pos + from.len()].copy_from_slice(to);
            pos += from.len();
            count += 1;
        } else {
            pos += 1;
        }
    }
    Ok(count)
}

fn hex_str(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02X}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_fingerprint_separates_match_unidentified_and_mismatch() {
        let stock = "Lenovo/TB323FU/TB323FU:16/BQ2A/x:user/release-keys";
        let baldur = "Lenovo/lineage_baldur/baldur:17/CP2A/eng:userdebug/test-keys";
        let unknown = "Lenovo/lineage_foo/foo:17/CP2A/eng:userdebug/test-keys";
        let classify = |fp: &str, model: &str| classify_fingerprint(Some(fp.to_string()), model);

        assert!(matches!(
            classify(stock, "TB323FU"),
            ModelValidation::Match { .. }
        ));
        assert!(matches!(
            classify(baldur, "TB323FU"),
            ModelValidation::Match { .. }
        ));
        // Spaces in the device model are tolerated.
        assert!(matches!(
            classify(unknown, "TB 323FU"),
            ModelValidation::Unidentified { ref device_model, .. } if device_model == "TB323FU"
        ));
        // A fingerprint naming a different known model never falls back.
        assert!(matches!(
            classify(stock, "TB320FC"),
            ModelValidation::Mismatch { .. }
        ));
        assert!(matches!(
            classify(baldur, "TB320FC"),
            ModelValidation::Mismatch { .. }
        ));
        // Unidentified needs a known device model.
        assert!(matches!(
            classify(unknown, "Legion Y700"),
            ModelValidation::Mismatch { .. }
        ));
        assert!(matches!(
            classify(unknown, ""),
            ModelValidation::Match { .. }
        ));
        assert_eq!(
            classify_fingerprint(None, "TB323FU"),
            ModelValidation::Missing
        );
    }

    #[test]
    fn replace_in_place_works() {
        let mut data = b"hello.PRC.world.PRC.end".to_vec();
        let n = replace_in_place(&mut data, b".PRC", b".ROW").unwrap();
        assert_eq!(n, 2);
        assert_eq!(&data, b"hello.ROW.world.ROW.end");
    }

    #[test]
    fn detect_product_region_reads_dtb_marker() {
        // Mimic the FDT layout: `product_region` name, FDT_PROP header
        // (token, len, nameoff), then the "PRC\0" / "ROW\0" value.
        let build = |value: &[u8]| -> Vec<u8> {
            let mut v = b"product_region\0\0".to_vec();
            v.extend_from_slice(&[0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0x26, 0xb7]);
            v.extend_from_slice(value);
            v
        };
        let dir = std::env::temp_dir().join(format!("ltbox_region_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let prc = dir.join("vb_prc.img");
        std::fs::write(&prc, build(b"PRC\0")).unwrap();
        assert_eq!(detect_product_region(&prc), Some(RegionTarget::Prc));

        let row = dir.join("vb_row.img");
        std::fs::write(&row, build(b"ROW\0")).unwrap();
        assert_eq!(detect_product_region(&row), Some(RegionTarget::Row));

        // A strings-block-only `product_region` (no adjacent value) -> None.
        let none = dir.join("vb_none.img");
        std::fs::write(&none, b"product_region\0model\0compatible\0").unwrap();
        assert_eq!(detect_product_region(&none), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_in_place_rejects_length_mismatch() {
        let mut data = b"hello.PRC.end".to_vec();
        let err = replace_in_place(&mut data, b".PRC", b".RO").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("length mismatch"), "unexpected: {msg}");
    }

    #[test]
    fn count_occurrences_works() {
        assert_eq!(count_occurrences(b"AABBAABB", b"BB"), 2);
        assert_eq!(count_occurrences(b"AAAA", b"BB"), 0);
    }

    #[test]
    fn default_patterns_detect_region() {
        let patterns = RegionPatternSet::default();
        assert_eq!(
            detect_region_in_data(b"abc IPRC def", &patterns),
            DetectedRegion::Prc
        );
        assert_eq!(
            detect_region_in_data(b"abc IROW def", &patterns),
            DetectedRegion::Row
        );
        assert_eq!(
            detect_region_in_data(b".PRC and .ROW", &patterns),
            DetectedRegion::Mixed
        );
        assert_eq!(
            detect_region_in_data(b"no marker", &patterns),
            DetectedRegion::Unknown
        );
    }

    #[test]
    fn region_build_progress_stops_after_inspection_when_target_matches() {
        let firmware = tempfile::tempdir().unwrap();
        fs::write(
            firmware.path().join("vendor_boot.img"),
            b"header IROW footer",
        )
        .unwrap();
        fs::write(firmware.path().join("vbmeta.img"), b"unused for no-op").unwrap();
        let output = tempfile::tempdir().unwrap();
        let mut stages = Vec::new();

        let result = build_region_converted_avb_images_with_progress(
            firmware.path(),
            output.path(),
            RegionTarget::Row,
            &RegionPatternSet::default(),
            None,
            |stage| stages.push(stage),
        )
        .unwrap();

        assert!(matches!(result, RegionAvbBuild::Skipped { .. }));
        assert_eq!(stages, vec![RegionBuildStage::Inspect]);
    }

    #[test]
    fn detect_country_in_buffer() {
        let data = b"\x00\x00CNXX\x00\x00";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.img");
        fs::write(&path, data).unwrap();

        let code = detect_country_code(&path, &["CN", "KR", "US"], false).unwrap();
        assert_eq!(code, Some("CN".to_string()));
    }

    #[test]
    fn field_only_skips_log_country_codes() {
        // Mimics a persist ext4: a hwdiag log line carrying the code
        // (text-delimited) plus the real country file (null-terminated).
        let data = b"Update country code CNXX\njunk\x00CNXX\x00\x00";
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("persist.img");
        fs::write(&src, data).unwrap();

        assert_eq!(
            detect_country_code(&src, &["CN"], true).unwrap(),
            Some("CN".to_string())
        );

        // Field-only patch edits only the null-terminated field; the log entry
        // keeps its CNXX.
        let out = dir.path().join("persist.patched.img");
        let changed = patch_country_code(&src, &out, "CN", "KR", EU_COUNTRY_CODES, true).unwrap();
        assert!(changed);
        let patched = fs::read(&out).unwrap();
        assert!(patched.windows(6).any(|w| w == b"\x00KRXX\x00"));
        assert!(
            patched
                .windows("country code CNXX\n".len())
                .any(|w| w == b"country code CNXX\n")
        );
    }

    #[test]
    fn field_only_requires_leading_boundary() {
        let dir = tempfile::tempdir().unwrap();

        // A log string with zero slack — `...code CNXX\0\0\0`. Mid-block and
        // `code `-prefixed, so the trailing NUL alone must NOT make it look
        // like a field.
        let log_with_nul = b"xx Update country code CNXX\x00\x00\x00".to_vec();
        let src = dir.path().join("log.img");
        fs::write(&src, &log_with_nul).unwrap();
        assert_eq!(detect_country_code(&src, &["CN"], true).unwrap(), None);
        let out = dir.path().join("log.patched.img");
        assert!(!patch_country_code(&src, &out, "CN", "KR", EU_COUNTRY_CODES, true).unwrap());

        // A block-aligned field NOT preceded by a NUL (mimics the TB321FU persist
        // dump: the code starts a 4K block, preceded by the prior block's binary
        // tail). Must still be detected + patched.
        let mut blk = vec![0xABu8; EXT4_BLOCK];
        blk.extend_from_slice(b"CNXX");
        blk.extend_from_slice(&[0u8; 4]);
        let src2 = dir.path().join("aligned.img");
        fs::write(&src2, &blk).unwrap();
        assert_eq!(
            detect_country_code(&src2, &["CN"], true).unwrap(),
            Some("CN".to_string())
        );
        let out2 = dir.path().join("aligned.patched.img");
        assert!(patch_country_code(&src2, &out2, "CN", "KR", EU_COUNTRY_CODES, true).unwrap());
        let patched = fs::read(&out2).unwrap();
        assert_eq!(&patched[EXT4_BLOCK..EXT4_BLOCK + 4], b"KRXX");
    }

    #[test]
    fn field_only_matches_xiaoxin_pro13_real_layouts_without_log_decoys() {
        let dir = tempfile::tempdir().unwrap();

        let mut proinfo = vec![0u8; 0x2000];
        proinfo[0x0e10..0x0e14].copy_from_slice(b"CNXX");
        let proinfo_path = dir.path().join("proinfo.img");
        fs::write(&proinfo_path, &proinfo).unwrap();
        assert_eq!(
            detect_country_code(&proinfo_path, &["CN"], true).unwrap(),
            Some("CN".to_string())
        );

        let mut persist = vec![0xabu8; EXT4_BLOCK * 3];
        let real = EXT4_BLOCK * 2;
        persist[real..real + 4].copy_from_slice(b"CNXX");
        persist[real + 4..real + EXT4_BLOCK].fill(0);
        let decoy = b"countryCode = CNXX\0";
        persist[128..128 + decoy.len()].copy_from_slice(decoy);
        let persist_path = dir.path().join("persist.img");
        fs::write(&persist_path, &persist).unwrap();
        assert_eq!(
            detect_country_code(&persist_path, &["CN"], true).unwrap(),
            Some("CN".to_string())
        );

        let patched_path = dir.path().join("persist.patched.img");
        assert!(
            patch_country_code(
                &persist_path,
                &patched_path,
                "CN",
                "KR",
                EU_COUNTRY_CODES,
                true,
            )
            .unwrap()
        );
        let patched = fs::read(patched_path).unwrap();
        assert_eq!(&patched[real..real + 4], b"KRXX");
        assert_eq!(&patched[128..128 + decoy.len()], decoy);
    }

    #[test]
    fn patch_proinfo_channel_flips_consumer_to_commercial() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("proinfo.img");
        let out = dir.path().join("proinfo.patched.img");
        let mut data = vec![0u8; 0x2000];
        let build_marker = b"TB376FC_PRC/TB376FC_CN_OPEN_USER_release_build!";
        data[..build_marker.len()].copy_from_slice(build_marker);
        data[0x0e1a..0x0e22].copy_from_slice(b"consumer");
        fs::write(&src, &data).unwrap();

        assert!(patch_proinfo_channel(&src, &out).unwrap());
        let patched = fs::read(&out).unwrap();
        assert_eq!(&patched[0x0e1a..0x0e24], b"commercial");
        assert_eq!(patched[0x0e24], 0);
        assert_eq!(&patched[..build_marker.len()], &data[..build_marker.len()]);
    }

    #[test]
    fn patch_proinfo_channel_is_idempotent_for_commercial() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("proinfo.img");
        let out = dir.path().join("proinfo.patched.img");
        let mut data = vec![0u8; 256];
        data[32..42].copy_from_slice(b"commercial");
        fs::write(&src, &data).unwrap();

        assert!(!patch_proinfo_channel(&src, &out).unwrap());
        assert_eq!(fs::read(out).unwrap(), data);
    }

    #[test]
    fn patch_proinfo_channel_refuses_non_nul_trailing_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("proinfo.img");
        let out = dir.path().join("proinfo.patched.img");
        let mut data = vec![0u8; 256];
        data[32..40].copy_from_slice(b"consumer");
        data[41] = 1;
        fs::write(&src, data).unwrap();

        let error = patch_proinfo_channel(&src, &out).unwrap_err().to_string();
        assert!(error.contains("trailing NUL"), "got: {error}");
        assert!(!out.exists());
    }

    #[test]
    fn real_firmware_builds_region_avb_images_when_available() {
        let Some(dir) = std::env::var_os("LTBOX_REAL_FIRMWARE_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let vendor_boot_src = dir.join("vendor_boot.img");
        let vbmeta_src = dir.join("vbmeta.img");
        if !vendor_boot_src.is_file() || !vbmeta_src.is_file() {
            return;
        }

        let patterns = RegionPatternSet::default();
        let data = fs::read(&vendor_boot_src).unwrap();
        if detect_region_in_data(&data, &patterns) != DetectedRegion::Row {
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let output_dir = tmp.path().join("region");
        fs::create_dir_all(&output_dir).unwrap();
        fs::write(output_dir.join("vendor_boot.patched.img"), b"stale").unwrap();

        let built = build_region_converted_avb_images(
            &dir,
            &output_dir,
            RegionTarget::Prc,
            &patterns,
            None,
        )
        .unwrap();

        let RegionAvbBuild::Built(output) = built else {
            panic!("ROW firmware should build a PRC output pair");
        };
        assert!(output.vendor_boot.is_file());
        assert!(output.vbmeta.is_file());
        assert!(output.replacement_count > 0);
        assert!(!output_dir.join("vendor_boot.patched.img").exists());

        let source_vendor_boot = fs::read(&vendor_boot_src).unwrap();
        let output_vendor_boot = fs::read(&output.vendor_boot).unwrap();
        assert_ne!(source_vendor_boot, output_vendor_boot);

        assert_eq!(
            fs::metadata(&output.vbmeta).unwrap().len(),
            fs::metadata(&vbmeta_src).unwrap().len()
        );
        let report = avb::image_info_report(&[output.vbmeta]).unwrap();
        assert!(report.contains("vendor_boot"));
    }
}
