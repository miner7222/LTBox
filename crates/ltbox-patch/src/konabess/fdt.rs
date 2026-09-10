//! Minimal flattened-device-tree reader and splice serializer.

use std::ops::Range;

use ltbox_core::Result;

use super::{
    GpuGroup, GpuLevel, GpuProperty, GpuTable, GpuTableIssue, KonaBessExport, error,
    validate_gpu_table,
};

const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;
const FDT_HEADER_SIZE: usize = 40;

/// GPU-relevant information read from one FDT blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdtGpuInfo {
    /// Root `model` property, when present and string-valued.
    pub model: Option<String>,
    /// Supported Qualcomm chip codename inferred from root `compatible`.
    pub chip: Option<String>,
    /// Complete GPU table, or `None` when the blob has no such sibling set.
    pub table: Option<GpuTable>,
}

#[derive(Debug, Clone, Copy)]
struct FdtLayout {
    total_size: usize,
    structure: RangeParts,
    strings: RangeParts,
}

#[derive(Debug, Clone, Copy)]
struct RangeParts {
    start: usize,
    end: usize,
}

impl RangeParts {
    fn range(self) -> Range<usize> {
        self.start..self.end
    }
}

#[derive(Debug)]
struct ParsedFdt {
    layout: FdtLayout,
    info: FdtGpuInfo,
    table_range: Option<Range<usize>>,
}

#[derive(Debug)]
struct OpenGroup {
    group: GpuGroup,
    start: usize,
    depth: usize,
    parent_path: Vec<String>,
    level: Option<GpuLevel>,
}

/// Parse model/chip identity and the GPU table from one FDT blob.
pub fn parse_fdt_gpu_info(fdt: &[u8]) -> Result<FdtGpuInfo> {
    Ok(parse_fdt(fdt)?.info)
}

/// Replace the whole GPU group sibling set with an export's table.
///
/// The chip gate is evaluated before serialization. Structure bytes before and
/// after the group region are copied verbatim; the existing strings block is
/// also copied verbatim and extended only for property names absent from it.
pub fn replace_fdt_gpu_table(fdt: &[u8], export: &KonaBessExport) -> Result<Vec<u8>> {
    replace_fdt_gpu_table_with_chip(fdt, export, None)
}

/// Replace a GPU table directly from an in-memory editor model.
///
/// `chip` is checked against the target FDT exactly as it is for a file-backed
/// [`KonaBessExport`].
pub fn replace_fdt_gpu_table_from_table(
    fdt: &[u8],
    chip: &str,
    table: &GpuTable,
) -> Result<Vec<u8>> {
    replace_fdt_gpu_table(
        fdt,
        &KonaBessExport {
            chip: chip.to_string(),
            description: String::new(),
            table: table.clone(),
            import_warnings: vec![],
        },
    )
}

pub(super) fn replace_fdt_gpu_table_with_chip(
    fdt: &[u8],
    export: &KonaBessExport,
    section_chip: Option<&str>,
) -> Result<Vec<u8>> {
    let parsed = parse_fdt(fdt)?;
    let chip = parsed
        .info
        .chip
        .as_deref()
        .or(section_chip)
        .ok_or_else(|| error("target FDT chip cannot be identified"))?;
    if chip != export.chip {
        return Err(error(format!(
            "chip mismatch: export is `{}`, target FDT is `{chip}`",
            export.chip
        )));
    }
    let table_range = parsed
        .table_range
        .clone()
        .ok_or_else(|| error("target FDT has no GPU power-level table"))?;
    let source_table = parsed
        .info
        .table
        .as_ref()
        .ok_or_else(|| error("target FDT has no GPU power-level table"))?;
    let normalization = normalize_edited_gpu_table(source_table, &export.table)?;
    rebuild_fdt(fdt, parsed.layout, table_range, &normalization.table)
}

/// A normalized editor table plus non-blocking findings suitable for GUI display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTableNormalization {
    /// Table with sequential row ids/`reg` cells and retargeted initial indices.
    pub table: GpuTable,
    /// Non-blocking validation and retargeting findings.
    pub advisories: Vec<GpuTableIssue>,
}

/// Normalize editor-owned row order while preserving the source table's
/// initial-frequency targets.
///
/// For each initial-power-level index, the original target frequency is looked
/// up in the edited row order. If that frequency no longer exists, the old
/// index is clamped to the edited group's last row. This makes deletion
/// deterministic and guarantees an in-range index. Per-row properties are not
/// reordered or synthesized; only the row id and its scalar `reg` cell are
/// rewritten to the row's new zero-based position.
///
/// # Errors
///
/// Returns an error when `edited` has a blocking structural validation issue or
/// contains more levels than can be represented by a `u32` id.
pub fn normalize_edited_gpu_table(
    source: &GpuTable,
    edited: &GpuTable,
) -> Result<GpuTableNormalization> {
    let validation = validate_gpu_table(edited);
    if let Some(first) = validation.hard_errors.first() {
        return Err(error(format!(
            "replacement GPU table has {} hard validation error(s); {}: {}",
            validation.hard_errors.len(),
            first.path,
            first.message
        )));
    }

    let mut advisories = validation.warnings;
    let mut normalized = edited.clone();
    for group in &mut normalized.groups {
        let source_group = source
            .groups
            .iter()
            .find(|candidate| candidate.id == group.id);
        for property_name in ["qcom,initial-pwrlevel", "qcom,initial-min-pwrlevel"] {
            let Some(property_position) = group
                .header_properties
                .iter()
                .position(|property| property.name == property_name)
            else {
                continue;
            };
            let old_index = source_group
                .and_then(|source_group| {
                    scalar_property(&source_group.header_properties, property_name)
                })
                .unwrap_or(group.header_properties[property_position].cells[0]);
            let old_frequency = source_group
                .and_then(|source_group| source_group.levels.get(old_index as usize))
                .and_then(level_frequency);
            let matching_indices = old_frequency
                .map(|frequency| {
                    group
                        .levels
                        .iter()
                        .enumerate()
                        .filter_map(|(index, level)| {
                            (level_frequency(level) == Some(frequency)).then_some(index)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let retargeted = matching_indices.first().copied().unwrap_or_else(|| {
                usize::try_from(old_index)
                    .unwrap_or(usize::MAX)
                    .min(group.levels.len() - 1)
            });
            let new_frequency = level_frequency(&group.levels[retargeted])
                .expect("hard validation requires scalar qcom,gpu-freq");
            if let Some(old_frequency) = old_frequency {
                let path = format!("group {} / {property_name}", group.id);
                if matching_indices.is_empty() {
                    advisories.push(GpuTableIssue {
                        path,
                        message: format!(
                            "group {} target frequency {old_frequency} Hz was deleted; retargeted to {new_frequency} Hz at level {retargeted}",
                            group.id
                        ),
                    });
                } else if matching_indices.len() > 1 {
                    advisories.push(GpuTableIssue {
                        path,
                        message: format!(
                            "group {} target frequency {old_frequency} Hz matches {} levels; first match wins and retargets to {new_frequency} Hz at level {retargeted}",
                            group.id,
                            matching_indices.len()
                        ),
                    });
                }
            }
            group.header_properties[property_position].cells[0] = retargeted as u32;
        }

        for (position, level) in group.levels.iter_mut().enumerate() {
            let id = u32::try_from(position)
                .map_err(|_| error("replacement GPU group has more than u32::MAX levels"))?;
            level.id = id;
            let reg = level
                .properties
                .iter_mut()
                .find(|property| property.name == "reg")
                .expect("hard validation requires scalar reg");
            reg.cells[0] = id;
        }
    }
    Ok(GpuTableNormalization {
        table: normalized,
        advisories,
    })
}

fn scalar_property(properties: &[GpuProperty], name: &str) -> Option<u32> {
    let cells = &properties
        .iter()
        .find(|property| property.name == name)?
        .cells;
    (cells.len() == 1).then(|| cells[0])
}

fn level_frequency(level: &GpuLevel) -> Option<u32> {
    scalar_property(&level.properties, "qcom,gpu-freq")
}

fn parse_fdt(fdt: &[u8]) -> Result<ParsedFdt> {
    let layout = parse_layout(fdt)?;
    let structure = &fdt[layout.structure.range()];
    let strings = &fdt[layout.strings.range()];
    let mut position = 0usize;
    let mut path = Vec::<String>::new();
    let mut model = None;
    let mut compatible = Vec::<String>::new();
    let mut groups = Vec::<GpuGroup>::new();
    let mut ranges = Vec::<Range<usize>>::new();
    let mut parent_path: Option<Vec<String>> = None;
    let mut open_group: Option<OpenGroup> = None;
    let mut saw_end = false;

    while position < structure.len() {
        let token_start = position;
        let token = read_be_u32(structure, position, "FDT structure token")?;
        position += 4;
        match token {
            FDT_BEGIN_NODE => {
                let (name, next) = read_node_name(structure, position)?;
                position = next;
                path.push(name.clone());
                let depth = path.len() - 1;
                if let Some(id) = decimal_suffix(&name, "qcom,gpu-pwrlevels-")? {
                    if open_group.is_some() {
                        return Err(error("nested GPU group nodes in FDT"));
                    }
                    let this_parent = path[..path.len() - 1].to_vec();
                    if let Some(expected) = &parent_path {
                        if expected != &this_parent {
                            return Err(error("GPU groups do not share one parent node"));
                        }
                    } else {
                        parent_path = Some(this_parent.clone());
                    }
                    open_group = Some(OpenGroup {
                        group: GpuGroup {
                            id,
                            header_properties: Vec::new(),
                            levels: Vec::new(),
                        },
                        start: token_start,
                        depth,
                        parent_path: this_parent,
                        level: None,
                    });
                } else if let Some(id) = decimal_suffix(&name, "qcom,gpu-pwrlevel@")? {
                    if let Some(group) = open_group.as_mut() {
                        if depth != group.depth + 1 || group.level.is_some() {
                            return Err(error("unexpected GPU power-level nesting"));
                        }
                        group.level = Some(GpuLevel {
                            id,
                            properties: Vec::new(),
                        });
                    }
                } else if let Some(group) = &open_group
                    && depth > group.depth
                {
                    return Err(error(format!(
                        "unexpected node `{name}` inside GPU group {}",
                        group.group.id
                    )));
                }
            }
            FDT_END_NODE => {
                let depth = path
                    .len()
                    .checked_sub(1)
                    .ok_or_else(|| error("unmatched FDT_END_NODE"))?;
                if let Some(group) = open_group.as_mut()
                    && depth == group.depth + 1
                {
                    let level = group
                        .level
                        .take()
                        .ok_or_else(|| error("unexpected child ending inside GPU group"))?;
                    group.group.levels.push(level);
                } else if open_group
                    .as_ref()
                    .is_some_and(|group| depth == group.depth)
                {
                    let group = open_group.take().expect("group checked above");
                    if group.level.is_some() {
                        return Err(error("unterminated GPU power-level node"));
                    }
                    if group.group.levels.is_empty() {
                        return Err(error(format!(
                            "GPU group {} has no power levels",
                            group.group.id
                        )));
                    }
                    if group.parent_path != path[..path.len() - 1] {
                        return Err(error("GPU group parent changed while parsing"));
                    }
                    groups.push(group.group);
                    ranges.push(group.start..position);
                }
                path.pop();
            }
            FDT_PROP => {
                let length = usize::try_from(read_be_u32(structure, position, "property length")?)
                    .map_err(|_| error("FDT property length does not fit usize"))?;
                let name_offset = usize::try_from(read_be_u32(
                    structure,
                    position + 4,
                    "property name offset",
                )?)
                .map_err(|_| error("FDT property name offset does not fit usize"))?;
                let value_start = position + 8;
                let value_end = value_start
                    .checked_add(length)
                    .ok_or_else(|| error("FDT property length overflow"))?;
                let value = structure
                    .get(value_start..value_end)
                    .ok_or_else(|| error("truncated FDT property value"))?;
                position = align_up(value_end, 4)?;
                if position > structure.len() {
                    return Err(error("truncated FDT property padding"));
                }
                let name = read_string(strings, name_offset, "property name")?;
                let depth = path
                    .len()
                    .checked_sub(1)
                    .ok_or_else(|| error("FDT property appears outside the root node"))?;
                if depth == 0 {
                    if name == "model" {
                        model = first_string(value);
                    } else if name == "compatible" {
                        compatible = string_list(value);
                    }
                }
                if let Some(group) = open_group.as_mut() {
                    let property = parse_cell_property(name, value)?;
                    if depth == group.depth {
                        group.group.header_properties.push(property);
                    } else if depth == group.depth + 1 && group.level.is_some() {
                        group
                            .level
                            .as_mut()
                            .expect("level checked above")
                            .properties
                            .push(property);
                    } else {
                        return Err(error("property has unexpected depth inside GPU group"));
                    }
                }
            }
            FDT_NOP => {}
            FDT_END => {
                if !path.is_empty() || open_group.is_some() {
                    return Err(error("FDT_END appears before all nodes close"));
                }
                saw_end = true;
                if structure[position..]
                    .chunks_exact(4)
                    .any(|word| word != [0, 0, 0, 0])
                {
                    return Err(error("nonzero data follows FDT_END"));
                }
                break;
            }
            other => return Err(error(format!("unknown FDT token 0x{other:08x}"))),
        }
    }
    if !saw_end {
        return Err(error("FDT structure has no FDT_END token"));
    }

    let table_range = if ranges.is_empty() {
        None
    } else {
        for pair in ranges.windows(2) {
            let gap = &structure[pair[0].end..pair[1].start];
            if !gap
                .chunks_exact(4)
                .all(|word| word == FDT_NOP.to_be_bytes())
                || !gap.len().is_multiple_of(4)
            {
                return Err(error("GPU group sibling set is not contiguous"));
            }
        }
        Some(ranges[0].start..ranges.last().expect("ranges nonempty").end)
    };
    let chip = infer_chip(&compatible);
    Ok(ParsedFdt {
        layout,
        info: FdtGpuInfo {
            model,
            chip,
            table: (!groups.is_empty()).then_some(GpuTable { groups }),
        },
        table_range,
    })
}

fn parse_layout(fdt: &[u8]) -> Result<FdtLayout> {
    if fdt.len() < FDT_HEADER_SIZE || read_be_u32(fdt, 0, "FDT magic")? != FDT_MAGIC {
        return Err(error("invalid FDT magic"));
    }
    let total_size = usize_field(fdt, 4, "FDT total size")?;
    let structure_start = usize_field(fdt, 8, "FDT structure offset")?;
    let strings_start = usize_field(fdt, 12, "FDT strings offset")?;
    let strings_size = usize_field(fdt, 32, "FDT strings size")?;
    let structure_size = usize_field(fdt, 36, "FDT structure size")?;
    if total_size > fdt.len() || total_size < FDT_HEADER_SIZE {
        return Err(error("FDT total size is outside the input blob"));
    }
    let structure_end = structure_start
        .checked_add(structure_size)
        .ok_or_else(|| error("FDT structure range overflow"))?;
    let strings_end = strings_start
        .checked_add(strings_size)
        .ok_or_else(|| error("FDT strings range overflow"))?;
    if structure_start < FDT_HEADER_SIZE
        || structure_end > total_size
        || strings_start < structure_end
        || strings_end > total_size
    {
        return Err(error(
            "unsupported or overlapping FDT structure/strings layout",
        ));
    }
    Ok(FdtLayout {
        total_size,
        structure: RangeParts {
            start: structure_start,
            end: structure_end,
        },
        strings: RangeParts {
            start: strings_start,
            end: strings_end,
        },
    })
}

fn rebuild_fdt(
    fdt: &[u8],
    layout: FdtLayout,
    table_range: Range<usize>,
    table: &GpuTable,
) -> Result<Vec<u8>> {
    let old_structure = &fdt[layout.structure.range()];
    let old_strings = &fdt[layout.strings.range()];
    let mut strings = StringTable::new(old_strings);
    let replacement = encode_table(table, &mut strings)?;
    let mut structure =
        Vec::with_capacity(old_structure.len() - table_range.len() + replacement.len());
    structure.extend_from_slice(&old_structure[..table_range.start]);
    structure.extend_from_slice(&replacement);
    structure.extend_from_slice(&old_structure[table_range.end..]);

    let structure_delta = signed_delta(structure.len(), old_structure.len())?;
    let strings_delta = signed_delta(strings.bytes.len(), old_strings.len())?;
    let new_strings_start = add_signed(layout.strings.start, structure_delta)?;
    let new_total_size = add_signed(
        add_signed(layout.total_size, structure_delta)?,
        strings_delta,
    )?;

    let mut output = Vec::with_capacity(new_total_size);
    output.extend_from_slice(&fdt[..layout.structure.start]);
    output.extend_from_slice(&structure);
    output.extend_from_slice(&fdt[layout.structure.end..layout.strings.start]);
    output.extend_from_slice(&strings.bytes);
    output.extend_from_slice(&fdt[layout.strings.end..layout.total_size]);
    if output.len() != new_total_size {
        return Err(error("internal FDT rebuild size mismatch"));
    }
    write_be_u32(&mut output, 4, u32_field(new_total_size, "FDT total size")?)?;
    write_be_u32(
        &mut output,
        12,
        u32_field(new_strings_start, "FDT strings offset")?,
    )?;
    write_be_u32(
        &mut output,
        32,
        u32_field(strings.bytes.len(), "FDT strings size")?,
    )?;
    write_be_u32(
        &mut output,
        36,
        u32_field(structure.len(), "FDT structure size")?,
    )?;
    Ok(output)
}

struct StringTable {
    bytes: Vec<u8>,
}

impl StringTable {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
        }
    }

    fn offset(&mut self, name: &str) -> Result<u32> {
        let mut position = 0usize;
        while position < self.bytes.len() {
            let end = self.bytes[position..]
                .iter()
                .position(|&byte| byte == 0)
                .map(|relative| position + relative)
                .ok_or_else(|| error("unterminated existing FDT string"))?;
            if &self.bytes[position..end] == name.as_bytes() {
                return u32_field(position, "property name offset");
            }
            position = end + 1;
        }
        let offset = u32_field(self.bytes.len(), "property name offset")?;
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes.push(0);
        Ok(offset)
    }
}

fn encode_table(table: &GpuTable, strings: &mut StringTable) -> Result<Vec<u8>> {
    if table.groups.is_empty() {
        return Err(error("replacement GPU table has no groups"));
    }
    let mut output = Vec::new();
    for group in &table.groups {
        encode_begin_node(&mut output, &format!("qcom,gpu-pwrlevels-{}", group.id));
        for property in &group.header_properties {
            encode_property(&mut output, property, strings)?;
        }
        for level in &group.levels {
            encode_begin_node(&mut output, &format!("qcom,gpu-pwrlevel@{}", level.id));
            for property in &level.properties {
                encode_property(&mut output, property, strings)?;
            }
            output.extend_from_slice(&FDT_END_NODE.to_be_bytes());
        }
        output.extend_from_slice(&FDT_END_NODE.to_be_bytes());
    }
    Ok(output)
}

fn encode_begin_node(output: &mut Vec<u8>, name: &str) {
    output.extend_from_slice(&FDT_BEGIN_NODE.to_be_bytes());
    output.extend_from_slice(name.as_bytes());
    output.push(0);
    while !output.len().is_multiple_of(4) {
        output.push(0);
    }
}

fn encode_property(
    output: &mut Vec<u8>,
    property: &GpuProperty,
    strings: &mut StringTable,
) -> Result<()> {
    if property.cells.is_empty() {
        return Err(error(format!(
            "replacement property `{}` has no cells",
            property.name
        )));
    }
    let value_size = property
        .cells
        .len()
        .checked_mul(4)
        .ok_or_else(|| error("replacement property size overflow"))?;
    output.extend_from_slice(&FDT_PROP.to_be_bytes());
    output.extend_from_slice(&u32_field(value_size, "property size")?.to_be_bytes());
    output.extend_from_slice(&strings.offset(&property.name)?.to_be_bytes());
    for cell in &property.cells {
        output.extend_from_slice(&cell.to_be_bytes());
    }
    Ok(())
}

fn parse_cell_property(name: &str, value: &[u8]) -> Result<GpuProperty> {
    if value.is_empty() || !value.len().is_multiple_of(4) {
        return Err(error(format!(
            "GPU property `{name}` is not a nonempty u32-cell list"
        )));
    }
    let cells = value
        .chunks_exact(4)
        .map(|bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .collect();
    Ok(GpuProperty {
        name: name.to_string(),
        cells,
    })
}

const CHIP_ALIAS_SETS: [(&str, &[&str]); 4] = [
    (
        "pineapple",
        &["pineapple", "pineapplep", "volcano", "volcanop"],
    ),
    ("sun", &["sun", "sunp", "tuna", "tunap"]),
    ("diwali", &["diwali", "diwali-lte", "diwalip"]),
    ("canoe", &["canoe", "canoep"]),
];

/// Whether two supported chip names belong to the same KonaBess alias set.
pub fn chip_names_match(left: &str, right: &str) -> bool {
    match (chip_family(left), chip_family(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn chip_family(name: &str) -> Option<&'static str> {
    let name = name.strip_prefix("qcom,").unwrap_or(name);
    CHIP_ALIAS_SETS
        .iter()
        .find_map(|(family, aliases)| aliases.contains(&name).then_some(*family))
}

fn infer_chip(compatible: &[String]) -> Option<String> {
    for (chip, aliases) in CHIP_ALIAS_SETS {
        if compatible
            .iter()
            .filter_map(|compatible| compatible.strip_prefix("qcom,"))
            .any(|compatible| aliases.contains(&compatible))
        {
            return Some(chip.to_string());
        }
    }
    None
}

fn decimal_suffix(name: &str, prefix: &str) -> Result<Option<u32>> {
    let Some(suffix) = name.strip_prefix(prefix) else {
        return Ok(None);
    };
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(format!("invalid numeric FDT node name `{name}`")));
    }
    suffix
        .parse()
        .map(Some)
        .map_err(|_| error(format!("FDT node id in `{name}` does not fit u32")))
}

fn read_node_name(bytes: &[u8], start: usize) -> Result<(String, usize)> {
    let relative_end = bytes
        .get(start..)
        .and_then(|tail| tail.iter().position(|&byte| byte == 0))
        .ok_or_else(|| error("unterminated FDT node name"))?;
    let end = start + relative_end;
    let name = std::str::from_utf8(&bytes[start..end])
        .map_err(|e| error(format!("FDT node name is not UTF-8: {e}")))?
        .to_string();
    let next = align_up(end + 1, 4)?;
    if next > bytes.len() {
        return Err(error("truncated FDT node-name padding"));
    }
    Ok((name, next))
}

fn read_string<'a>(strings: &'a [u8], offset: usize, kind: &str) -> Result<&'a str> {
    let tail = strings
        .get(offset..)
        .ok_or_else(|| error(format!("{kind} offset is outside FDT strings")))?;
    let end = tail
        .iter()
        .position(|&byte| byte == 0)
        .ok_or_else(|| error(format!("unterminated FDT {kind}")))?;
    std::str::from_utf8(&tail[..end]).map_err(|e| error(format!("FDT {kind} is not UTF-8: {e}")))
}

fn first_string(value: &[u8]) -> Option<String> {
    string_list(value).into_iter().next()
}

fn string_list(value: &[u8]) -> Vec<String> {
    value
        .split(|&byte| byte == 0)
        .filter(|item| !item.is_empty())
        .filter_map(|item| std::str::from_utf8(item).ok().map(str::to_string))
        .collect()
}

fn usize_field(bytes: &[u8], offset: usize, name: &str) -> Result<usize> {
    usize::try_from(read_be_u32(bytes, offset, name)?)
        .map_err(|_| error(format!("{name} does not fit usize")))
}

fn read_be_u32(bytes: &[u8], offset: usize, name: &str) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| error(format!("truncated {name}")))?;
    Ok(u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
}

fn write_be_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<()> {
    bytes
        .get_mut(offset..offset + 4)
        .ok_or_else(|| error("internal FDT header write is out of bounds"))?
        .copy_from_slice(&value.to_be_bytes());
    Ok(())
}

fn u32_field(value: usize, name: &str) -> Result<u32> {
    u32::try_from(value).map_err(|_| error(format!("{name} does not fit u32")))
}

fn align_up(value: usize, alignment: usize) -> Result<usize> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(error("invalid alignment"));
    }
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| error("alignment overflow"))
}

fn signed_delta(new: usize, old: usize) -> Result<isize> {
    if new >= old {
        isize::try_from(new - old).map_err(|_| error("FDT size delta overflow"))
    } else {
        isize::try_from(old - new)
            .map(|value| -value)
            .map_err(|_| error("FDT size delta overflow"))
    }
}

fn add_signed(value: usize, delta: isize) -> Result<usize> {
    value
        .checked_add_signed(delta)
        .ok_or_else(|| error("FDT layout offset overflow"))
}

#[cfg(test)]
pub(super) mod test_support {
    use super::*;

    pub fn table(group_levels: &[(u32, usize)], base_frequency: u32) -> GpuTable {
        GpuTable {
            groups: group_levels
                .iter()
                .map(|&(group_id, count)| GpuGroup {
                    id: group_id,
                    header_properties: vec![
                        GpuProperty {
                            name: "qcom,speed-bin".into(),
                            cells: vec![group_id],
                        },
                        GpuProperty {
                            name: "qcom,initial-pwrlevel".into(),
                            cells: vec![0],
                        },
                    ],
                    levels: (0..count)
                        .map(|id| GpuLevel {
                            id: id as u32,
                            properties: vec![
                                GpuProperty {
                                    name: "reg".into(),
                                    cells: vec![id as u32],
                                },
                                GpuProperty {
                                    name: "qcom,gpu-freq".into(),
                                    cells: vec![base_frequency.saturating_sub(id as u32)],
                                },
                                GpuProperty {
                                    name: "qcom,level".into(),
                                    cells: vec![100],
                                },
                            ],
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    pub fn synthetic_fdt(chip: &str, model: &str, table: &GpuTable) -> Vec<u8> {
        let mut strings = StringTable::new(&[]);
        let mut structure = Vec::new();
        encode_begin_node(&mut structure, "");
        encode_bytes_property(
            &mut structure,
            "compatible",
            &format!("qcom,{chip}\0").into_bytes(),
            &mut strings,
        );
        encode_bytes_property(
            &mut structure,
            "model",
            &format!("{model}\0").into_bytes(),
            &mut strings,
        );
        encode_begin_node(&mut structure, "untouched-before");
        encode_bytes_property(
            &mut structure,
            "marker-before",
            &[0xaa, 0xbb, 0xcc, 0xdd],
            &mut strings,
        );
        structure.extend_from_slice(&FDT_END_NODE.to_be_bytes());
        structure.extend_from_slice(&encode_table(table, &mut strings).unwrap());
        encode_begin_node(&mut structure, "untouched-after");
        encode_bytes_property(
            &mut structure,
            "marker-after",
            &[1, 2, 3, 4, 5],
            &mut strings,
        );
        structure.extend_from_slice(&FDT_END_NODE.to_be_bytes());
        structure.extend_from_slice(&FDT_END_NODE.to_be_bytes());
        structure.extend_from_slice(&FDT_END.to_be_bytes());

        let reserve_offset = FDT_HEADER_SIZE;
        let structure_offset = reserve_offset + 16;
        let strings_offset = structure_offset + structure.len();
        let total_size = strings_offset + strings.bytes.len();
        let mut fdt = vec![0u8; FDT_HEADER_SIZE];
        fdt.extend_from_slice(&[0u8; 16]);
        fdt.extend_from_slice(&structure);
        fdt.extend_from_slice(&strings.bytes);
        for (offset, value) in [
            (0, FDT_MAGIC),
            (4, total_size as u32),
            (8, structure_offset as u32),
            (12, strings_offset as u32),
            (16, reserve_offset as u32),
            (20, 17),
            (24, 16),
            (28, 0),
            (32, strings.bytes.len() as u32),
            (36, structure.len() as u32),
        ] {
            fdt[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        fdt
    }

    fn encode_bytes_property(
        output: &mut Vec<u8>,
        name: &str,
        value: &[u8],
        strings: &mut StringTable,
    ) {
        output.extend_from_slice(&FDT_PROP.to_be_bytes());
        output.extend_from_slice(&(value.len() as u32).to_be_bytes());
        output.extend_from_slice(&strings.offset(name).unwrap().to_be_bytes());
        output.extend_from_slice(value);
        while !output.len().is_multiple_of(4) {
            output.push(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{synthetic_fdt, table};
    use super::*;

    #[test]
    fn chip_name_matching_reuses_the_fdt_alias_families() {
        for alias in ["sun", "sunp", "tuna", "tunap", "qcom,tuna"] {
            assert!(chip_names_match("sun", alias));
        }
        assert!(chip_names_match("pineapple", "volcano"));
        assert!(chip_names_match("diwali", "diwali-lte"));
        assert!(chip_names_match("canoe", "canoep"));
        assert!(!chip_names_match("sun", "pineapple"));
        assert!(!chip_names_match("unknown", "unknown"));
    }

    #[test]
    fn self_table_round_trip_is_functionally_identical_and_preserves_splices() {
        let original_table = table(&[(0, 2), (3, 1)], 100);
        let fdt = synthetic_fdt("sun", "Synthetic Sun", &original_table);
        let before = parse_fdt(&fdt).unwrap();
        let old_range = before.table_range.clone().unwrap();
        let export = KonaBessExport {
            chip: "sun".into(),
            description: "self".into(),
            table: original_table.clone(),
            import_warnings: vec![],
        };

        let rebuilt = replace_fdt_gpu_table(&fdt, &export).unwrap();
        let after = parse_fdt(&rebuilt).unwrap();
        let new_range = after.table_range.clone().unwrap();

        assert_eq!(rebuilt, fdt);
        assert_eq!(after.info.table, Some(original_table));
        assert_eq!(after.info.model.as_deref(), Some("Synthetic Sun"));
        let old_structure = &fdt[before.layout.structure.range()];
        let new_structure = &rebuilt[after.layout.structure.range()];
        assert_eq!(
            &old_structure[..old_range.start],
            &new_structure[..new_range.start]
        );
        assert_eq!(
            &old_structure[old_range.end..],
            &new_structure[new_range.end..]
        );
        assert_eq!(
            &fdt[before.layout.strings.range()],
            &rebuilt[after.layout.strings.range()]
        );
    }

    #[test]
    fn wholesale_replacement_allows_different_shape_and_appends_names() {
        let fdt = synthetic_fdt("sun", "Synthetic Sun", &table(&[(0, 1)], 100));
        let before = parse_fdt(&fdt).unwrap();
        let old_range = before.table_range.clone().unwrap();
        let mut replacement = table(&[(2, 3), (4, 2)], 900);
        replacement.groups[0].header_properties.push(GpuProperty {
            name: "qcom,new-header".into(),
            cells: vec![7],
        });
        let export = KonaBessExport {
            chip: "sun".into(),
            description: String::new(),
            table: replacement.clone(),
            import_warnings: vec![],
        };
        let rebuilt = replace_fdt_gpu_table(&fdt, &export).unwrap();
        let after = parse_fdt(&rebuilt).unwrap();
        let new_range = after.table_range.clone().unwrap();
        assert_eq!(after.info.table, Some(replacement));
        let old_structure = &fdt[before.layout.structure.range()];
        let new_structure = &rebuilt[after.layout.structure.range()];
        assert_eq!(
            &old_structure[..old_range.start],
            &new_structure[..new_range.start]
        );
        assert_eq!(
            &old_structure[old_range.end..],
            &new_structure[new_range.end..]
        );
        assert!(
            rebuilt[after.layout.strings.range()].starts_with(&fdt[before.layout.strings.range()])
        );
    }

    #[test]
    fn chip_mismatch_is_rejected() {
        let fdt = synthetic_fdt("sun", "Synthetic Sun", &table(&[(0, 1)], 100));
        let export = KonaBessExport {
            chip: "pineapple".into(),
            description: String::new(),
            table: table(&[(0, 1)], 100),
            import_warnings: vec![],
        };
        let error = replace_fdt_gpu_table(&fdt, &export).unwrap_err();
        assert!(error.to_string().contains("chip mismatch"));
    }

    #[test]
    fn value_edit_preserves_unknown_properties() {
        let mut source = table(&[(0, 3)], 900_000_000);
        source.groups[0].header_properties.push(GpuProperty {
            name: "vendor,unknown-header".into(),
            cells: vec![7, 8, 9],
        });
        source.groups[0].levels[1].properties.push(GpuProperty {
            name: "vendor,unknown-level".into(),
            cells: vec![0xfeed_beef],
        });
        let fdt = synthetic_fdt("sun", "Synthetic Sun", &source);
        let mut edited = source.clone();
        scalar_property_mut(&mut edited.groups[0].levels[0].properties, "qcom,gpu-freq").cells[0] =
            950_000_000;

        let no_op = replace_fdt_gpu_table_from_table(&fdt, "sun", &source).unwrap();
        assert_eq!(no_op, fdt);
        let rebuilt = replace_fdt_gpu_table_from_table(&fdt, "sun", &edited).unwrap();
        let changed_bytes = fdt
            .iter()
            .zip(&rebuilt)
            .filter(|(before, after)| before != after)
            .count();
        assert!((1..=4).contains(&changed_bytes));
        let applied = parse_fdt_gpu_info(&rebuilt).unwrap().table.unwrap();

        assert_eq!(
            scalar_property(&applied.groups[0].levels[0].properties, "qcom,gpu-freq"),
            Some(950_000_000)
        );
        assert_eq!(
            applied.groups[0]
                .header_properties
                .iter()
                .find(|property| property.name == "vendor,unknown-header"),
            source.groups[0]
                .header_properties
                .iter()
                .find(|property| property.name == "vendor,unknown-header")
        );
        assert_eq!(
            applied.groups[0].levels[1]
                .properties
                .iter()
                .find(|property| property.name == "vendor,unknown-level"),
            source.groups[0].levels[1]
                .properties
                .iter()
                .find(|property| property.name == "vendor,unknown-level")
        );
    }

    #[test]
    fn add_and_remove_renumber_rows_retarget_initial_indices_and_keep_bus_votes() {
        let mut source = table(&[(0, 3)], 900_000_000);
        source.groups[0]
            .header_properties
            .iter_mut()
            .find(|property| property.name == "qcom,initial-pwrlevel")
            .unwrap()
            .cells[0] = 2;
        source.groups[0].header_properties.push(GpuProperty {
            name: "qcom,initial-min-pwrlevel".into(),
            cells: vec![2],
        });
        for (level, bus_vote) in source.groups[0].levels.iter_mut().zip([9, 6, 3]) {
            level.properties.push(GpuProperty {
                name: "qcom,bus-freq-ddr7".into(),
                cells: vec![bus_vote],
            });
        }
        let fdt = synthetic_fdt("sun", "Synthetic Sun", &source);

        let mut removed = source.clone();
        removed.groups[0].levels.remove(0);
        let removed_fdt = replace_fdt_gpu_table_from_table(&fdt, "sun", &removed).unwrap();
        let removed_table = parse_fdt_gpu_info(&removed_fdt).unwrap().table.unwrap();
        assert_sequential_levels(&removed_table.groups[0]);
        assert_eq!(
            header_scalar(&removed_table.groups[0], "qcom,initial-pwrlevel"),
            1
        );
        assert_eq!(
            header_scalar(&removed_table.groups[0], "qcom,initial-min-pwrlevel"),
            1
        );
        assert_eq!(
            level_scalar(&removed_table.groups[0].levels[1], "qcom,gpu-freq"),
            level_scalar(&source.groups[0].levels[2], "qcom,gpu-freq")
        );
        assert_eq!(
            level_scalar(&removed_table.groups[0].levels[0], "qcom,bus-freq-ddr7"),
            6
        );
        assert_eq!(
            level_scalar(&removed_table.groups[0].levels[1], "qcom,bus-freq-ddr7"),
            3
        );

        let mut added = source.clone();
        let mut new_level = added.groups[0].levels[0].clone();
        new_level.id = 99;
        scalar_property_mut(&mut new_level.properties, "reg").cells[0] = 99;
        scalar_property_mut(&mut new_level.properties, "qcom,gpu-freq").cells[0] = 1_000_000_000;
        scalar_property_mut(&mut new_level.properties, "qcom,bus-freq-ddr7").cells[0] = 10;
        added.groups[0].levels.insert(0, new_level);
        let added_fdt = replace_fdt_gpu_table_from_table(&fdt, "sun", &added).unwrap();
        let added_table = parse_fdt_gpu_info(&added_fdt).unwrap().table.unwrap();
        assert_sequential_levels(&added_table.groups[0]);
        assert_eq!(
            header_scalar(&added_table.groups[0], "qcom,initial-pwrlevel"),
            3
        );
        assert_eq!(
            header_scalar(&added_table.groups[0], "qcom,initial-min-pwrlevel"),
            3
        );
        assert_eq!(
            level_scalar(&added_table.groups[0].levels[3], "qcom,gpu-freq"),
            level_scalar(&source.groups[0].levels[2], "qcom,gpu-freq")
        );
        assert_eq!(
            level_scalar(&added_table.groups[0].levels[0], "qcom,bus-freq-ddr7"),
            10
        );
        assert_eq!(
            level_scalar(&added_table.groups[0].levels[3], "qcom,bus-freq-ddr7"),
            3
        );
    }

    #[test]
    fn normalization_advises_when_an_initial_target_is_deleted_or_ambiguous() {
        let mut source = table(&[(0, 3)], 900_000_000);
        scalar_property_mut(
            &mut source.groups[0].header_properties,
            "qcom,initial-pwrlevel",
        )
        .cells[0] = 1;
        let old_frequency = level_scalar(&source.groups[0].levels[1], "qcom,gpu-freq");

        let mut deleted = source.clone();
        deleted.groups[0].levels.remove(1);
        let deleted_normalization = normalize_edited_gpu_table(&source, &deleted).unwrap();
        let new_frequency = level_scalar(
            &deleted_normalization.table.groups[0].levels[1],
            "qcom,gpu-freq",
        );
        let deleted_advisory = deleted_normalization
            .advisories
            .iter()
            .find(|advisory| advisory.message.contains("was deleted"))
            .expect("deleted-target advisory");
        assert_eq!(deleted_advisory.path, "group 0 / qcom,initial-pwrlevel");
        assert!(
            deleted_advisory
                .message
                .contains(&old_frequency.to_string())
        );
        assert!(
            deleted_advisory
                .message
                .contains(&new_frequency.to_string())
        );

        let mut duplicated = source.clone();
        scalar_property_mut(
            &mut duplicated.groups[0].levels[0].properties,
            "qcom,gpu-freq",
        )
        .cells[0] = old_frequency;
        let duplicated_normalization = normalize_edited_gpu_table(&source, &duplicated).unwrap();
        assert_eq!(
            header_scalar(
                &duplicated_normalization.table.groups[0],
                "qcom,initial-pwrlevel"
            ),
            0
        );
        let duplicate_advisory = duplicated_normalization
            .advisories
            .iter()
            .find(|advisory| advisory.message.contains("first match wins"))
            .expect("duplicate-frequency advisory");
        assert_eq!(duplicate_advisory.path, "group 0 / qcom,initial-pwrlevel");
        assert!(
            duplicate_advisory
                .message
                .contains(&old_frequency.to_string())
        );
        assert!(duplicate_advisory.message.contains("matches 2 levels"));
    }

    #[test]
    fn missing_initial_power_levels_validate_and_round_trip_unchanged() {
        let mut source = table(&[(0, 2)], 900_000_000);
        source.groups[0].header_properties.retain(|property| {
            !matches!(
                property.name.as_str(),
                "qcom,initial-pwrlevel" | "qcom,initial-min-pwrlevel"
            )
        });
        assert!(!validate_gpu_table(&source).has_hard_errors());

        let fdt = synthetic_fdt("sun", "Synthetic Sun", &source);
        let rebuilt = replace_fdt_gpu_table_from_table(&fdt, "sun", &source).unwrap();
        assert_eq!(rebuilt, fdt);
        assert_eq!(parse_fdt_gpu_info(&rebuilt).unwrap().table, Some(source));
    }

    fn scalar_property_mut<'a>(
        properties: &'a mut [GpuProperty],
        name: &str,
    ) -> &'a mut GpuProperty {
        properties
            .iter_mut()
            .find(|property| property.name == name)
            .unwrap()
    }

    fn header_scalar(group: &GpuGroup, name: &str) -> u32 {
        scalar_property(&group.header_properties, name).unwrap()
    }

    fn level_scalar(level: &GpuLevel, name: &str) -> u32 {
        scalar_property(&level.properties, name).unwrap()
    }

    fn assert_sequential_levels(group: &GpuGroup) {
        for (position, level) in group.levels.iter().enumerate() {
            assert_eq!(level.id, position as u32);
            assert_eq!(level_scalar(level, "reg"), position as u32);
        }
    }
}
