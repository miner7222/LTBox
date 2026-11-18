//! Rawprogram/patch XML flashing: node planning, the pre-write geometry
//! preflight, LUN capacity bounds and the Firehose transfer itself.

use super::*;

/// Parsed before any destructive command; device-side expressions stay verbatim.
struct PatchNodePlan {
    byte_off: u64,
    slot: u8,
    lun: u8,
    size: u64,
    start_sector: String,
    value: String,
}

fn plan_patch_xml(xml_path: &Path) -> Result<Vec<PatchNodePlan>> {
    let xml_content = ltbox_core::xml::read(xml_path)?;
    let doc = ltbox_core::xml::parse(&xml_content).map_err(|e| {
        EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
    })?;
    let mut plans = Vec::new();
    for node in doc.descendants().filter(|node| {
        node.tag_name().name().eq_ignore_ascii_case("patch")
            && node.attribute("filename") == Some("DISK")
    }) {
        let ctx = "<patch>";
        require_destructive_coords(&node, ctx)?;
        plans.push(PatchNodePlan {
            byte_off: parse_xml_attr(&node, "byte_offset", 0, ctx)?,
            lun: parse_xml_attr(&node, "physical_partition_number", 0, ctx)?,
            slot: parse_xml_attr(&node, "slot", 0, ctx)?,
            size: parse_xml_attr(&node, "size_in_bytes", 0, ctx)?,
            start_sector: node.attribute("start_sector").unwrap().to_owned(),
            value: node.attribute("value").unwrap_or("").to_owned(),
        });
    }
    Ok(plans)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WipeErasePlanEntry {
    pub(super) label: String,
    pub(super) lun: u8,
    pub(super) start_sector: String,
    pub(super) num_sectors: usize,
}

impl WipeErasePlanEntry {
    fn log_line(&self) -> String {
        self.log_line_with_template(&tr("log_edl_pre_erase_cmd"))
    }

    fn log_line_with_template(&self, template: &str) -> String {
        format!(
            "[EDL] {}",
            ltbox_core::i18n::format_template(
                template,
                &[
                    ("label", self.label.clone()),
                    ("lun", self.lun.to_string()),
                    ("start", self.start_sector.clone()),
                    ("sectors", self.num_sectors.to_string()),
                ]
            )
        )
    }
}

/// A rawprogram `<program>` node resolved against its image file, with no
/// device I/O. Shared by the preflight and the transfer.
#[derive(Debug, PartialEq, Eq)]
enum ProgramNodePlan {
    /// GPT placeholder or empty entry (qdl CLI `parse_program_cmd`).
    Skip,
    /// The package does not ship the referenced image.
    MissingImage {
        label: String,
        image_path: PathBuf,
    },
    Write(ProgramWrite),
}

#[derive(Debug, PartialEq, Eq)]
struct ProgramWrite {
    label: String,
    image_path: PathBuf,
    lun: u8,
    slot: u8,
    start_sector: String,
    num_sectors: usize,
    /// Byte offset into `image_path`, already bounded by the file length.
    byte_offset: u64,
}

fn plan_program_node(
    node: &roxmltree::Node<'_, '_>,
    xml_dir: &Path,
    sector_size: u64,
) -> Result<ProgramNodePlan> {
    let label = node.attribute("label").unwrap_or("").trim().to_string();
    let filename = node.attribute("filename").unwrap_or("").trim();
    let ctx = format!("<program label={label}>");
    let num_sectors: usize = parse_xml_attr(node, "num_partition_sectors", 0usize, &ctx)?;
    if filename.is_empty() || num_sectors == 0 {
        return Ok(ProgramNodePlan::Skip);
    }

    require_destructive_coords(node, &ctx)?;
    let lun: u8 = parse_xml_attr(node, "physical_partition_number", 0u8, &ctx)?;
    let slot: u8 = parse_xml_attr(node, "slot", 0u8, &ctx)?;
    let start_sector = node.attribute("start_sector").unwrap_or("0").to_string();
    let file_sector_offset: u64 = parse_xml_attr(node, "file_sector_offset", 0u64, &ctx)?;

    let image_path = ltbox_core::safe_path::safe_join(xml_dir, filename)
        .map_err(|e| EdlError::Session(e.to_string()))?;
    if !image_path.exists() {
        return Ok(ProgramNodePlan::MissingImage { label, image_path });
    }
    // The transfer zero-pads to `num_sectors`, so an empty file would zero the
    // whole partition and a directory would fail mid-flash. Refuse both now.
    let file_len = {
        let meta = std::fs::metadata(&image_path)?;
        if !meta.is_file() {
            return Err(EdlError::Session(format!(
                "{ctx}: {} is not a regular file",
                image_path.display()
            )));
        }
        meta.len()
    };
    if file_len == 0 {
        return Err(EdlError::Session(format!(
            "{ctx}: {} is empty; refusing to write {num_sectors} zero-filled sectors",
            image_path.display()
        )));
    }

    let mut byte_offset = 0;
    if file_sector_offset > 0 {
        // Untrusted XML: a wrapped product would flash the wrong region of the
        // image, and an offset past EOF would feed Firehose stale data.
        byte_offset = sector_size.checked_mul(file_sector_offset).ok_or_else(|| {
            EdlError::Session(format!(
                "{ctx}: file_sector_offset {file_sector_offset} \
                     × sector_size {sector_size} overflows u64"
            ))
        })?;
        if byte_offset >= file_len {
            return Err(EdlError::Session(format!(
                "{ctx}: file_sector_offset {file_sector_offset} \
                 (byte offset {byte_offset}) >= image length {file_len} \
                 for {}",
                image_path.display(),
            )));
        }
    }

    Ok(ProgramNodePlan::Write(ProgramWrite {
        label,
        image_path,
        lun,
        slot,
        start_sector,
        num_sectors,
        byte_offset,
    }))
}

#[derive(Debug, PartialEq, Eq)]
struct EraseNodePlan {
    lun: u8,
    start_sector: String,
    num_sectors: usize,
    efisp: bool,
}

/// A rawprogram `<erase>` node's coordinates, or `None` for an empty erase.
fn plan_erase_node(node: &roxmltree::Node<'_, '_>) -> Result<Option<EraseNodePlan>> {
    let ctx = "<erase>";
    let num_sectors: usize = parse_xml_attr(node, "num_partition_sectors", 0usize, ctx)?;
    if num_sectors == 0 {
        return Ok(None);
    }
    require_destructive_coords(node, ctx)?;
    let lun: u8 = parse_xml_attr(node, "physical_partition_number", 0u8, ctx)?;
    Ok(Some(EraseNodePlan {
        lun,
        start_sector: node.attribute("start_sector").unwrap_or("0").to_string(),
        num_sectors,
        efisp: node
            .attribute("label")
            .is_some_and(|label| label.trim() == "efisp"),
    }))
}

#[derive(Debug)]
struct ProtectedRange {
    lun: u8,
    start: u64,
    num_sectors: u64,
    label: String,
}

fn ranges_overlap(start: u64, num: u64, other_start: u64, other_num: u64) -> Result<bool> {
    let end = start
        .checked_add(num)
        .ok_or_else(|| EdlError::Session("sector range overflow".into()))?;
    let other_end = other_start
        .checked_add(other_num)
        .ok_or_else(|| EdlError::Session("sector range overflow".into()))?;
    Ok(num != 0 && other_num != 0 && start < other_end && other_start < end)
}

fn collect_protected_ranges(program_xmls: &[PathBuf]) -> Result<Vec<ProtectedRange>> {
    let mut ranges = Vec::new();
    for path in program_xmls {
        let content = ltbox_core::xml::read(path)?;
        let doc = ltbox_core::xml::parse(&content).map_err(|e| {
            EdlError::Session(format!("XML parse error in {}: {e}", path.display()))
        })?;
        for node in doc
            .descendants()
            .filter(|n| n.tag_name().name().eq_ignore_ascii_case("program"))
        {
            let label = node.attribute("label").unwrap_or("").trim();
            if !EdlSession::keep_data_skip_labels(label) {
                continue;
            }
            let Ok(start) = node
                .attribute("start_sector")
                .unwrap_or("")
                .trim()
                .parse::<u64>()
            else {
                continue;
            };
            require_destructive_coords(&node, "<program>")?;
            let lun = parse_xml_attr(&node, "physical_partition_number", 0u8, "<program>")?;
            let num_sectors = parse_xml_attr(&node, "num_partition_sectors", 0u64, "<program>")?;
            ranges_overlap(start, num_sectors, start, num_sectors)?;
            ranges.push(ProtectedRange {
                lun,
                start,
                num_sectors,
                label: label.to_string(),
            });
        }
    }
    Ok(ranges)
}

fn keep_data_erase_label<'a>(
    node: &'a roxmltree::Node<'_, '_>,
    wipe: bool,
    ranges: &'a [ProtectedRange],
) -> Result<Option<String>> {
    if wipe {
        return Ok(None);
    }
    let label = node.attribute("label").unwrap_or("").trim();
    if EdlSession::keep_data_skip_labels(label) {
        return Ok(Some(label.to_string()));
    }
    let Ok(start) = node
        .attribute("start_sector")
        .unwrap_or("")
        .trim()
        .parse::<u64>()
    else {
        return Ok(None);
    };
    let Some(plan) = plan_erase_node(node)? else {
        return Ok(None);
    };
    for range in ranges.iter().filter(|range| range.lun == plan.lun) {
        if ranges_overlap(
            start,
            plan.num_sectors as u64,
            range.start,
            range.num_sectors,
        )? {
            return Ok(Some(if label.is_empty() {
                range.label.clone()
            } else {
                label.to_string()
            }));
        }
    }
    Ok(None)
}

/// Parse an XML attribute: absent → `default`; malformed → `EdlError::Session`.
/// Defaulting a malformed value would let a corrupt XML steer a flash at
/// sector 0 or skip a real partition.
fn parse_xml_attr<T>(
    node: &roxmltree::Node<'_, '_>,
    attr: &str,
    default: T,
    context: &str,
) -> Result<T>
where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    match node.attribute(attr) {
        None => Ok(default),
        Some(raw) => raw
            .trim()
            .parse::<T>()
            .map_err(|e| EdlError::Session(format!("{context}: invalid {attr}='{raw}': {e}"))),
    }
}

/// Refuse a destructive node (`<program>`, `<erase>`, DISK `<patch>`) without
/// `physical_partition_number` or `start_sector`: defaulting to 0 would write
/// the primary GPT. Presence-only, as `start_sector` may be a formula
/// (`NUM_DISK_SECTORS-33.`).
fn require_destructive_coords(node: &roxmltree::Node<'_, '_>, context: &str) -> Result<()> {
    for attr in ["physical_partition_number", "start_sector"] {
        if node.attribute(attr).is_none() {
            return Err(EdlError::Session(format!(
                "{context}: missing required {attr} on a destructive node"
            )));
        }
    }
    Ok(())
}

/// Bound one destructive rawprogram node against the target LUN's capacity.
///
/// `num_partition_sectors` is untrusted, and qdl zero-pads a short image up to
/// it, so an inflated count zeroes whatever follows. The XML may be rewriting
/// the GPT, so the disk size is the only bound that stays valid. A decimal
/// `start_sector` is checked end to end; a device-side expression
/// (`NUM_DISK_SECTORS-33.`) only has its count checked.
fn ensure_node_within_lun(
    ctx: &str,
    lun: u8,
    start_sector: &str,
    num_sectors: usize,
    lun_sectors: u64,
) -> Result<()> {
    let num = u64::try_from(num_sectors)
        .map_err(|_| EdlError::Session(format!("{ctx}: num_partition_sectors exceeds u64")))?;
    if num > lun_sectors {
        return Err(EdlError::Session(format!(
            "{ctx}: {num} sectors requested but LUN {lun} holds only {lun_sectors}"
        )));
    }
    if let Ok(start) = start_sector.trim().parse::<u64>() {
        let end = start.checked_add(num).ok_or_else(|| {
            EdlError::Session(format!(
                "{ctx}: start_sector {start} + {num} sectors overflows u64"
            ))
        })?;
        if end > lun_sectors {
            return Err(EdlError::Session(format!(
                "{ctx}: sectors {start}..{end} run past the end of LUN {lun} ({lun_sectors} sectors)"
            )));
        }
    }
    Ok(())
}

impl EdlSession {
    /// Memoised [`Self::physical_lun_sector_count`] for the rawprogram bounds
    /// check; probed once per LUN. `None` (no readable GPT header) skips the
    /// bound: a blank or corrupted device is when EDL flashing is the only way back.
    fn lun_sector_count_for_guard(&mut self, lun: u8, log: &mut Vec<String>) -> Option<u64> {
        if let Some(cached) = self.lun_sectors.get(&lun) {
            return *cached;
        }
        let probed = self.physical_lun_sector_count(lun, log).ok();
        self.lun_sectors.insert(lun, probed);
        probed
    }

    /// Apply [`ensure_node_within_lun`] when the LUN capacity is knowable.
    fn guard_node_within_lun(
        &mut self,
        ctx: &str,
        lun: u8,
        start_sector: &str,
        num_sectors: usize,
        log: &mut Vec<String>,
    ) -> Result<()> {
        match self.lun_sector_count_for_guard(lun, log) {
            Some(total) => ensure_node_within_lun(ctx, lun, start_sector, num_sectors, total),
            None => Ok(()),
        }
    }

    /// Erased on wipe=true. frp is intentionally not erased.
    const WIPE_ERASE_BASES: &'static [&'static str] = &["userdata", "metadata"];

    /// Skipped on wipe=false.
    const KEEP_DATA_SKIP_BASES: &'static [&'static str] = &["userdata", "metadata"];

    /// Match `label` against bases, with or without `_a`/`_b` suffix.
    fn label_matches_base(label: &str, bases: &[&str]) -> bool {
        let l = label.to_ascii_lowercase();
        bases
            .iter()
            .any(|b| l == *b || l.starts_with(&format!("{b}_")))
    }

    fn wipe_labels(label: &str) -> bool {
        Self::label_matches_base(label, Self::WIPE_ERASE_BASES)
    }

    fn keep_data_skip_labels(label: &str) -> bool {
        Self::label_matches_base(label, Self::KEEP_DATA_SKIP_BASES)
    }

    fn is_split_super_image(filename: &str) -> bool {
        Path::new(filename).file_name().is_some_and(|name| {
            let name = name.to_string_lossy().to_ascii_lowercase();
            name.starts_with("super_") && name.ends_with(".img")
        })
    }

    /// Require every referenced split-super image before any device mutation.
    /// Other missing images are logged and skipped by `flash_program_node`.
    fn preflight_rawprogram_super_images(program_xmls: &[PathBuf]) -> Result<()> {
        for xml_path in program_xmls {
            let xml_content = ltbox_core::xml::read(xml_path)?;
            let doc = ltbox_core::xml::parse(&xml_content).map_err(|e| {
                EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
            })?;
            let xml_dir = xml_path.parent().unwrap_or(Path::new("."));

            for node in doc
                .descendants()
                .filter(|node| node.tag_name().name().eq_ignore_ascii_case("program"))
            {
                let filename = node.attribute("filename").unwrap_or("").trim();
                if filename.is_empty() || !Self::is_split_super_image(filename) {
                    continue;
                }

                let label = node.attribute("label").unwrap_or("").trim();
                let ctx = format!("{} <program label={label}>", xml_path.display());
                let num_sectors: usize =
                    parse_xml_attr(&node, "num_partition_sectors", 0usize, &ctx)?;
                if num_sectors == 0 {
                    continue;
                }

                let image_path =
                    ltbox_core::safe_path::safe_join(xml_dir, filename).map_err(|e| {
                        EdlError::Session(format!(
                            "invalid split-super image reference `{filename}` in {}: {e}",
                            xml_path.display()
                        ))
                    })?;
                if !image_path.exists() {
                    return Err(EdlError::Session(format!(
                        "required split-super image is missing: {} (referenced by {})",
                        image_path.display(),
                        xml_path.display()
                    )));
                }
            }
        }
        Ok(())
    }

    /// Flash rawprograms, then apply patches. `wipe=true` first erases
    /// userdata/metadata (and slot variants); `wipe=false` skips those
    /// `<program>` nodes and any `<erase>` overlapping them.
    pub fn flash_rawprogram_with_wipe(
        &mut self,
        program_xmls: &[PathBuf],
        patch_xmls: &[PathBuf],
        wipe: bool,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let patch_plans = patch_xmls
            .iter()
            .map(|path| plan_patch_xml(path))
            .collect::<Result<Vec<_>>>()?;
        Self::preflight_rawprogram_super_images(program_xmls)?;
        let protected_ranges = if wipe {
            Vec::new()
        } else {
            collect_protected_ranges(program_xmls)?
        };
        self.preflight_rawprogram_nodes(program_xmls, !wipe, wipe, &protected_ranges, log)?;
        register_flash_bytes(self.rawprogram_transfer_bytes(program_xmls, wipe)?);

        if wipe {
            ltbox_core::live!(log, "[Flash] {}", tr("log_flash_wipe_enabled"));
            self.pre_erase_wipe_labels(program_xmls, log)?;
        } else {
            ltbox_core::live!(log, "[Flash] {}", tr("log_flash_wipe_disabled"));
        }

        for xml_path in program_xmls {
            ltbox_core::live!(
                log,
                "[EDL] {}",
                ltbox_core::tr_args!("log_edl_flash_cmd", path = xml_path.display())
            );
            self.flash_one_rawprogram(xml_path, wipe, &protected_ranges, log)?;
        }
        for (xml_path, plans) in patch_xmls.iter().zip(&patch_plans) {
            // File name only; the firmware folder is already known.
            let display_name = xml_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| xml_path.display().to_string());
            ltbox_core::live!(
                log,
                "[EDL] {}",
                ltbox_core::tr_args!("log_edl_patch_xml_cmd", path = display_name)
            );
            self.apply_patch_plans(plans, log)?;
        }
        Ok(())
    }

    /// Flash every `<program>` / `<erase>` node as listed, then apply patch
    /// XMLs, with no keep-data skip and no pre-erase. Like a stock Lenovo
    /// script, the data outcome is decided by which rawprogram was selected
    /// (`save_persist` vs `write_persist`). For a keep/wipe policy use
    /// [`Self::flash_rawprogram_with_wipe`].
    pub fn flash_rawprogram_verbatim(
        &mut self,
        program_xmls: &[PathBuf],
        patch_xmls: &[PathBuf],
        log: &mut Vec<String>,
    ) -> Result<()> {
        let patch_plans = patch_xmls
            .iter()
            .map(|path| plan_patch_xml(path))
            .collect::<Result<Vec<_>>>()?;
        Self::preflight_rawprogram_super_images(program_xmls)?;
        self.preflight_rawprogram_nodes(program_xmls, false, false, &[], log)?;
        register_flash_bytes(self.rawprogram_transfer_bytes(program_xmls, true)?);

        for xml_path in program_xmls {
            ltbox_core::live!(
                log,
                "[EDL] {}",
                ltbox_core::tr_args!("log_edl_flash_cmd", path = xml_path.display())
            );
            // `wipe = true` only disables the keep-data skip; no pre-erase runs.
            self.flash_one_rawprogram(xml_path, true, &[], log)?;
        }
        for (xml_path, plans) in patch_xmls.iter().zip(&patch_plans) {
            let display_name = xml_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| xml_path.display().to_string());
            ltbox_core::live!(
                log,
                "[EDL] {}",
                ltbox_core::tr_args!("log_edl_patch_xml_cmd", path = display_name)
            );
            self.apply_patch_plans(plans, log)?;
        }
        Ok(())
    }

    /// Total padded bytes the rawprogram pass will write, registered up front
    /// so the progress denominator stays stable.
    fn rawprogram_transfer_bytes(&self, program_xmls: &[PathBuf], wipe: bool) -> Result<u64> {
        let sector_size = self.dev.fh_config().storage_sector_size;
        let mut total = 0u64;
        for xml_path in program_xmls {
            let xml_content = ltbox_core::xml::read(xml_path)?;
            let doc = ltbox_core::xml::parse(&xml_content).map_err(|e| {
                EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
            })?;
            let xml_dir = xml_path.parent().unwrap_or(Path::new("."));
            for node in doc
                .descendants()
                .filter(|node| node.tag_name().name().eq_ignore_ascii_case("program"))
            {
                let label = node.attribute("label").unwrap_or("").trim();
                if !wipe && Self::keep_data_skip_labels(label) {
                    continue;
                }
                let filename = node.attribute("filename").unwrap_or("").trim();
                let ctx = format!("{} <program label={label}>", xml_path.display());
                let num_sectors: usize =
                    parse_xml_attr(&node, "num_partition_sectors", 0usize, &ctx)?;
                if filename.is_empty() || num_sectors == 0 {
                    continue;
                }
                let image_path = ltbox_core::safe_path::safe_join(xml_dir, filename)
                    .map_err(|e| EdlError::Session(e.to_string()))?;
                if image_path.exists() {
                    total = total.saturating_add(padded_transfer_bytes(num_sectors, sector_size)?);
                }
            }
        }
        Ok(total)
    }

    /// Erase every `<program>` whose label is in `WIPE_ERASE_BASES`
    /// using XML-reported coordinates. Skips partitions absent from the XMLs.
    fn pre_erase_wipe_labels(
        &mut self,
        program_xmls: &[PathBuf],
        log: &mut Vec<String>,
    ) -> Result<()> {
        for entry in Self::collect_wipe_erase_plan(program_xmls)? {
            // The pre-erase takes the same untrusted XML geometry as the
            // `<program>` pass, so it gets the same capacity bound.
            let ctx = format!("<program label={}> pre-erase", entry.label);
            self.guard_node_within_lun(
                &ctx,
                entry.lun,
                &entry.start_sector,
                entry.num_sectors,
                log,
            )?;
            ltbox_core::live!(log, "{}", entry.log_line());
            send_firehose_erase(
                &mut self.dev,
                entry.num_sectors,
                entry.lun,
                &entry.start_sector,
            )
            .map_err(|e| EdlError::Session(format!("Erase {} failed: {e}", entry.label)))?;
        }
        Ok(())
    }

    pub(super) fn collect_wipe_erase_plan(
        program_xmls: &[PathBuf],
    ) -> Result<Vec<WipeErasePlanEntry>> {
        let mut plan = Vec::new();
        for xml_path in program_xmls {
            let xml_content = ltbox_core::xml::read(xml_path)?;
            let doc = ltbox_core::xml::parse(&xml_content).map_err(|e| {
                EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
            })?;
            for node in doc.descendants() {
                if !node.tag_name().name().eq_ignore_ascii_case("program") {
                    continue;
                }
                let label = node.attribute("label").unwrap_or("").trim();
                if !Self::wipe_labels(label) {
                    continue;
                }
                let ctx = format!("{} <program label={label}>", xml_path.display());
                let num_sectors: usize =
                    parse_xml_attr(&node, "num_partition_sectors", 0usize, &ctx)?;
                if num_sectors == 0 {
                    continue;
                }
                // A missing LUN or start would default to LUN 0 / LBA 0 (primary GPT).
                require_destructive_coords(&node, &ctx)?;
                let lun: u8 = parse_xml_attr(&node, "physical_partition_number", 0u8, &ctx)?;
                let start_sector = node.attribute("start_sector").unwrap_or("0");
                plan.push(WipeErasePlanEntry {
                    label: label.to_string(),
                    lun,
                    start_sector: start_sector.to_string(),
                    num_sectors,
                });
            }
        }
        Ok(plan)
    }

    fn flash_one_rawprogram(
        &mut self,
        xml_path: &Path,
        wipe: bool,
        protected_ranges: &[ProtectedRange],
        log: &mut Vec<String>,
    ) -> Result<()> {
        let xml_content = ltbox_core::xml::read(xml_path)?;
        let doc = ltbox_core::xml::parse(&xml_content).map_err(|e| {
            EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
        })?;
        let xml_dir = xml_path.parent().unwrap_or(Path::new("."));

        for node in doc.descendants() {
            match node.tag_name().name().to_lowercase().as_str() {
                "program" => {
                    // Keep-data: skip userdata/metadata entries.
                    if !wipe {
                        let label = node.attribute("label").unwrap_or("").trim();
                        if Self::keep_data_skip_labels(label) {
                            ltbox_core::live!(
                                log,
                                "[EDL] {}",
                                ltbox_core::tr_args!("log_edl_skip_keep_data", label = label)
                            );
                            continue;
                        }
                    }
                    self.flash_program_node(&node, xml_dir, log)?;
                }
                "erase" => {
                    if let Some(label) = keep_data_erase_label(&node, wipe, protected_ranges)? {
                        ltbox_core::live!(
                            log,
                            "[EDL] {}",
                            ltbox_core::tr_args!("log_edl_skip_keep_data", label = label)
                        );
                        continue;
                    }
                    self.erase_program_node(&node, log)?;
                }
                _ => continue,
            }
        }
        Ok(())
    }

    /// Resolve and bound every node a rawprogram pass will touch before the
    /// first write, so a bad node in a later XML cannot abort a half-written
    /// flash. The transfer runs the same planners, so the two cannot drift.
    fn preflight_rawprogram_nodes(
        &mut self,
        program_xmls: &[PathBuf],
        skip_keep_data: bool,
        pre_erase: bool,
        protected_ranges: &[ProtectedRange],
        log: &mut Vec<String>,
    ) -> Result<()> {
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        for xml_path in program_xmls {
            let xml_content = ltbox_core::xml::read(xml_path)?;
            let doc = ltbox_core::xml::parse(&xml_content).map_err(|e| {
                EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
            })?;
            let xml_dir = xml_path.parent().unwrap_or(Path::new("."));
            for node in doc.descendants() {
                match node.tag_name().name().to_lowercase().as_str() {
                    "program" => {
                        let label = node.attribute("label").unwrap_or("").trim();
                        if skip_keep_data && Self::keep_data_skip_labels(label) {
                            continue;
                        }
                        if let ProgramNodePlan::Write(write) =
                            plan_program_node(&node, xml_dir, sector_size)?
                        {
                            let ctx =
                                format!("{} <program label={}>", xml_path.display(), write.label);
                            self.guard_node_within_lun(
                                &ctx,
                                write.lun,
                                &write.start_sector,
                                write.num_sectors,
                                log,
                            )?;
                        }
                    }
                    "erase" => {
                        if keep_data_erase_label(&node, !skip_keep_data, protected_ranges)?
                            .is_some()
                        {
                            continue;
                        }
                        if let Some(erase) = plan_erase_node(&node)? {
                            let ctx = format!("{} <erase>", xml_path.display());
                            self.guard_node_within_lun(
                                &ctx,
                                erase.lun,
                                &erase.start_sector,
                                erase.num_sectors,
                                log,
                            )?;
                        }
                    }
                    _ => {}
                }
            }
        }
        if pre_erase {
            for entry in Self::collect_wipe_erase_plan(program_xmls)? {
                let ctx = format!("<program label={}> pre-erase", entry.label);
                self.guard_node_within_lun(
                    &ctx,
                    entry.lun,
                    &entry.start_sector,
                    entry.num_sectors,
                    log,
                )?;
            }
        }
        Ok(())
    }

    fn flash_program_node(
        &mut self,
        node: &roxmltree::Node<'_, '_>,
        xml_dir: &Path,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        let write = match plan_program_node(node, xml_dir, sector_size)? {
            ProgramNodePlan::Skip => return Ok(()),
            ProgramNodePlan::MissingImage { label, image_path } => {
                ltbox_core::live!(
                    log,
                    "[EDL] {}",
                    ltbox_core::tr_args!(
                        "log_edl_skip_image_missing",
                        label = label,
                        path = image_path.display()
                    )
                );
                return Ok(());
            }
            ProgramNodePlan::Write(write) => write,
        };
        let ProgramWrite {
            label,
            image_path,
            lun,
            slot,
            start_sector,
            num_sectors,
            byte_offset,
        } = write;
        let ctx = format!("<program label={label}>");

        let mut file = std::fs::File::open(&image_path)?;
        if byte_offset > 0 {
            file.seek(SeekFrom::Start(byte_offset))?;
        }

        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!(
                "log_edl_flash_program_cmd",
                label = label,
                image = image_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(""),
                lun = lun,
                start = start_sector,
                sectors = num_sectors
            )
        );

        self.guard_node_within_lun(&ctx, lun, &start_sector, num_sectors, log)?;

        let transfer_bytes =
            padded_transfer_bytes(num_sectors, self.dev.fh_config().storage_sector_size)?;
        begin_partition_progress(&label, transfer_bytes, false);
        let mut last_percent: Option<u8> = None;
        qdl::firehose_program_storage_with_progress(
            &mut self.dev,
            &mut file,
            &label,
            num_sectors,
            slot,
            lun,
            &start_sector,
            |completed, total| update_flash_progress(&mut last_percent, &label, completed, total),
        )
        .map_err(|e| EdlError::Session(format!("Program {label} failed: {e}")))?;
        Ok(())
    }

    pub(super) fn erase_program_node(
        &mut self,
        node: &roxmltree::Node<'_, '_>,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let Some(EraseNodePlan {
            lun,
            start_sector,
            num_sectors,
            efisp,
        }) = plan_erase_node(node)?
        else {
            return Ok(());
        };

        self.guard_node_within_lun("<erase>", lun, &start_sector, num_sectors, log)?;

        // Named EFISP erases are zero-programmed (see `erase_partition_at`).
        if efisp {
            return self.erase_partition_at("efisp", lun, &start_sector, num_sectors, log);
        }

        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!(
                "log_edl_erase_lun_cmd",
                lun = lun,
                start = start_sector,
                sectors = num_sectors
            )
        );
        send_firehose_erase(&mut self.dev, num_sectors, lun, &start_sector)
            .map_err(|e| EdlError::Session(format!("Erase failed: {e}")))?;
        Ok(())
    }

    fn apply_patch_plans(&mut self, plans: &[PatchNodePlan], log: &mut Vec<String>) -> Result<()> {
        for PatchNodePlan {
            byte_off,
            slot,
            lun,
            size,
            start_sector,
            value,
        } in plans
        {
            ltbox_core::live_debug!(
                log,
                "[EDL] {}",
                ltbox_core::tr_args!(
                    "log_edl_patch_lun_cmd",
                    lun = lun,
                    start = start_sector,
                    offset = byte_off,
                    bytes = size,
                    value = value
                )
            );
            qdl::firehose_patch(
                &mut self.dev,
                *byte_off,
                *slot,
                *lun,
                *size,
                start_sector,
                value,
            )
            .map_err(|e| EdlError::Session(format!("Patch failed: {e}")))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edl::test_support::*;

    #[test]
    fn protected_sector_overlap_is_checked_and_half_open() {
        assert!(ranges_overlap(10, 10, 15, 2).unwrap());
        assert!(ranges_overlap(10, 10, 0, 30).unwrap());
        assert!(!ranges_overlap(10, 10, 20, 5).unwrap());
        assert!(!ranges_overlap(10, 0, 5, 10).unwrap());
        assert!(ranges_overlap(u64::MAX, 1, 0, 1).is_err());
        assert!(ranges_overlap(0, 1, u64::MAX, 1).is_err());
    }

    #[test]
    fn unlabeled_erase_respects_keep_data_across_xmls() {
        let fw = crate::edl::test_support::TempFirmwareDir::new();
        fw.write("rawprogram0.xml", r#"<data><program label="userdata" physical_partition_number="0" start_sector="100" num_partition_sectors="20"/></data>"#);
        let ranges = collect_protected_ranges(&[fw.path().join("rawprogram0.xml")]).unwrap();
        let doc = ltbox_core::xml::parse(r#"<data><erase physical_partition_number="0" start_sector="90" num_partition_sectors="40"/><erase physical_partition_number="1" start_sector="90" num_partition_sectors="40"/><erase physical_partition_number="0" start_sector="NUM_DISK_SECTORS-1." num_partition_sectors="40"/></data>"#).unwrap();
        let nodes: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("erase"))
            .collect();
        assert_eq!(
            keep_data_erase_label(&nodes[0], false, &ranges).unwrap(),
            Some("userdata".into())
        );
        assert_eq!(
            keep_data_erase_label(&nodes[0], true, &ranges).unwrap(),
            None
        );
        assert_eq!(
            keep_data_erase_label(&nodes[1], false, &ranges).unwrap(),
            None
        );
        assert_eq!(
            keep_data_erase_label(&nodes[2], false, &ranges).unwrap(),
            None
        );
    }

    #[test]
    fn require_destructive_coords_demands_lun_and_start_sector() {
        // Both present (start_sector may be a formula string) → accepted.
        let ok = ltbox_core::xml::parse(
            r#"<data><program physical_partition_number="0" start_sector="NUM_DISK_SECTORS-33."/></data>"#,
        )
        .unwrap();
        assert!(require_destructive_coords(&first_node(&ok, "program"), "<program>").is_ok());

        // Missing start_sector → rejected (would default to sector 0 = GPT).
        let no_start =
            ltbox_core::xml::parse(r#"<data><program physical_partition_number="2"/></data>"#)
                .unwrap();
        assert!(
            require_destructive_coords(&first_node(&no_start, "program"), "<program>").is_err()
        );

        // Missing physical_partition_number → rejected (would default to LUN 0).
        let no_lun = ltbox_core::xml::parse(r#"<data><erase start_sector="34"/></data>"#).unwrap();
        assert!(require_destructive_coords(&first_node(&no_lun, "erase"), "<erase>").is_err());
    }

    #[test]
    fn program_plan_resolves_writes_placeholders_and_missing_images() {
        let fw = TempFirmwareDir::new();
        fw.write_bytes("boot.img", &[0u8; 4096]);
        let doc = ltbox_core::xml::parse(
            r#"<data>
                <program label="boot_a" filename="boot.img" num_partition_sectors="8"
                         physical_partition_number="4" start_sector="100" slot="1"
                         file_sector_offset="2"/>
                <program label="PrimaryGPT" filename="" num_partition_sectors="6"
                         physical_partition_number="0" start_sector="0"/>
                <program label="dtbo_a" filename="dtbo.img" num_partition_sectors="8"
                         physical_partition_number="4" start_sector="200"/>
            </data>"#,
        )
        .unwrap();
        let nodes: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("program"))
            .collect();

        assert_eq!(
            plan_program_node(&nodes[0], fw.path(), 512).unwrap(),
            ProgramNodePlan::Write(ProgramWrite {
                label: "boot_a".into(),
                image_path: fw.path().join("boot.img"),
                lun: 4,
                slot: 1,
                start_sector: "100".into(),
                num_sectors: 8,
                byte_offset: 1024,
            })
        );
        assert_eq!(
            plan_program_node(&nodes[1], fw.path(), 512).unwrap(),
            ProgramNodePlan::Skip
        );
        assert_eq!(
            plan_program_node(&nodes[2], fw.path(), 512).unwrap(),
            ProgramNodePlan::MissingImage {
                label: "dtbo_a".into(),
                image_path: fw.path().join("dtbo.img"),
            }
        );
    }

    #[test]
    fn program_plan_rejects_bad_geometry_before_any_transfer() {
        let fw = TempFirmwareDir::new();
        fw.write_bytes("boot.img", &[0u8; 1024]);
        for node_xml in [
            // Missing start_sector would steer the write at the primary GPT.
            r#"<program label="boot_a" filename="boot.img" num_partition_sectors="2"
                        physical_partition_number="4"/>"#,
            // Offset at or past the end of the image.
            r#"<program label="boot_a" filename="boot.img" num_partition_sectors="2"
                        physical_partition_number="4" start_sector="0"
                        file_sector_offset="2"/>"#,
            // Offset multiply overflows u64.
            r#"<program label="boot_a" filename="boot.img" num_partition_sectors="2"
                        physical_partition_number="4" start_sector="0"
                        file_sector_offset="18446744073709551615"/>"#,
        ] {
            let xml = format!("<data>{node_xml}</data>");
            let doc = ltbox_core::xml::parse(&xml).unwrap();
            assert!(
                plan_program_node(&first_node(&doc, "program"), fw.path(), 512).is_err(),
                "{node_xml}"
            );
        }
    }

    #[test]
    fn erase_plan_keeps_coordinates_and_flags_efisp() {
        let doc = ltbox_core::xml::parse(
            r#"<data>
                <erase label="efisp" num_partition_sectors="16"
                       physical_partition_number="4" start_sector="NUM_DISK_SECTORS-16."/>
                <erase num_partition_sectors="0" physical_partition_number="0" start_sector="0"/>
                <erase num_partition_sectors="8" start_sector="0"/>
            </data>"#,
        )
        .unwrap();
        let nodes: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("erase"))
            .collect();
        assert_eq!(
            plan_erase_node(&nodes[0]).unwrap(),
            Some(EraseNodePlan {
                lun: 4,
                start_sector: "NUM_DISK_SECTORS-16.".into(),
                num_sectors: 16,
                efisp: true,
            })
        );
        assert_eq!(plan_erase_node(&nodes[1]).unwrap(), None);
        assert!(plan_erase_node(&nodes[2]).is_err());
    }

    #[test]
    fn node_within_lun_accepts_a_write_that_fits() {
        assert!(ensure_node_within_lun("<program>", 0, "34", 100, 1024).is_ok());
        // Exactly filling the disk is legal.
        assert!(ensure_node_within_lun("<program>", 0, "0", 1024, 1024).is_ok());
    }

    #[test]
    fn node_within_lun_rejects_a_count_larger_than_the_disk() {
        let err = ensure_node_within_lun("<program>", 4, "0", 2048, 1024).unwrap_err();
        assert!(
            err.to_string().contains("LUN 4 holds only 1024"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn node_within_lun_rejects_a_write_running_past_the_end() {
        let err = ensure_node_within_lun("<program>", 0, "1000", 100, 1024).unwrap_err();
        assert!(
            err.to_string().contains("run past the end"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn node_within_lun_rejects_start_plus_count_overflow() {
        let err =
            ensure_node_within_lun("<program>", 0, &u64::MAX.to_string(), 8, u64::MAX).unwrap_err();
        assert!(
            err.to_string().contains("overflows u64"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn node_within_lun_still_bounds_the_count_for_a_device_side_expression() {
        // `NUM_DISK_SECTORS-33.` is resolved by the programmer, not here, so
        // only the sector count can be checked — but it still must be.
        assert!(ensure_node_within_lun("<erase>", 0, "NUM_DISK_SECTORS-33.", 33, 1024).is_ok());
        assert!(ensure_node_within_lun("<erase>", 0, "NUM_DISK_SECTORS-33.", 4096, 1024).is_err());
    }

    #[test]
    fn wipe_erase_plan_keeps_xml_order_and_log_geometry() {
        let rawprogram = TempXml::new(
            r#"
            <data>
              <program label="metadata" physical_partition_number="0" start_sector="8192" num_partition_sectors="2048" />
              <program label="super" physical_partition_number="0" start_sector="16384" num_partition_sectors="4096" />
              <program label="frp" physical_partition_number="1" start_sector="24576" num_partition_sectors="128" />
              <program label="userdata_a" physical_partition_number="2" start_sector="32768" num_partition_sectors="0" />
              <program label="userdata_b" physical_partition_number="3" start_sector="65536" num_partition_sectors="8192" />
            </data>
            "#,
        );

        let plan = EdlSession::collect_wipe_erase_plan(&[rawprogram.path()])
            .expect("collect wipe erase plan");

        assert_eq!(
            plan.iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            ["metadata", "userdata_b"]
        );
        assert!(
            !plan.iter().any(|entry| entry.label == "frp"),
            "frp must not be pre-erased"
        );
        // Render without mutating the process-global GUI translator.
        let template = "Erase {label}: LUN {lun}, sector {start}, {sectors} sectors";
        assert_eq!(
            plan[0].log_line_with_template(template),
            "[EDL] Erase metadata: LUN 0, sector 8192, 2048 sectors"
        );
        assert_eq!(
            plan[1].log_line_with_template(template),
            "[EDL] Erase userdata_b: LUN 3, sector 65536, 8192 sectors"
        );
    }

    #[test]
    fn wipe_erase_plan_rejects_missing_destructive_coords() {
        // Missing start_sector would otherwise default to LBA 0 (primary GPT).
        let missing_start = TempXml::new(
            r#"
            <data>
              <program label="metadata" physical_partition_number="0" num_partition_sectors="2048" />
            </data>
            "#,
        );
        let err = EdlSession::collect_wipe_erase_plan(&[missing_start.path()])
            .expect_err("missing start_sector must fail");
        assert!(
            err.to_string().contains("start_sector"),
            "unexpected error: {err}"
        );

        // Missing physical_partition_number would otherwise default to LUN 0.
        let missing_lun = TempXml::new(
            r#"
            <data>
              <program label="userdata" start_sector="24576" num_partition_sectors="128" />
            </data>
            "#,
        );
        let err = EdlSession::collect_wipe_erase_plan(&[missing_lun.path()])
            .expect_err("missing physical_partition_number must fail");
        assert!(
            err.to_string().contains("physical_partition_number"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rawprogram_super_image_preflight_rejects_missing_image() {
        let fw = TempFirmwareDir::new();
        fw.write(
            "rawprogram0.xml",
            r#"<data><program label="super" filename="images/SUPER_1.IMG" num_partition_sectors="1" /></data>"#,
        );

        let rawprogram = fw.path().join("rawprogram0.xml");
        let err = EdlSession::preflight_rawprogram_super_images(&[rawprogram])
            .expect_err("missing split-super image must fail preflight");

        assert!(matches!(err, EdlError::Session(_)));
        let message = err.to_string();
        assert!(
            message.contains("SUPER_1.IMG"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("rawprogram0.xml"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn rawprogram_super_image_preflight_allows_present_and_optional_missing_images() {
        let fw = TempFirmwareDir::new();
        let image_dir = fw.path().join("images");
        std::fs::create_dir_all(&image_dir).expect("create split-super image directory");
        std::fs::write(image_dir.join("SuPeR_2.ImG"), b"super").expect("write split-super image");
        fw.write(
            "rawprogram0.xml",
            r#"
            <data>
              <program label="super" filename="images/SuPeR_2.ImG" num_partition_sectors="1" />
              <program label="boot" filename="boot.img" num_partition_sectors="1" />
              <program label="super" filename="super.img" num_partition_sectors="1" />
              <program label="super" filename="super_ignored.img" num_partition_sectors="0" />
              <program label="placeholder" filename="" num_partition_sectors="1" />
            </data>
            "#,
        );

        let rawprogram = fw.path().join("rawprogram0.xml");
        EdlSession::preflight_rawprogram_super_images(&[rawprogram])
            .expect("present split-super and optional missing images should pass preflight");
    }
}
