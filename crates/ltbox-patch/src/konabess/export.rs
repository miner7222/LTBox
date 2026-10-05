//! KonaBess URI and machine-generated DTS-like frequency-table parsing.

use std::{collections::BTreeMap, io::Read};

use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use flate2::bufread::GzDecoder;
use ltbox_core::Result;
use serde::Deserialize;

use super::error;

const URI_PREFIX: &str = "konabess://";
const MAX_EXPORT_JSON_SIZE: usize = 4 * 1024 * 1024;

/// One ordered u32-cell property in a GPU group or power level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuProperty {
    pub name: String,
    pub cells: Vec<u32>,
}

/// One `qcom,gpu-pwrlevel@N` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuLevel {
    pub id: u32,
    pub properties: Vec<GpuProperty>,
}

/// One `qcom,gpu-pwrlevels-N` node, including its ordered header properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuGroup {
    pub id: u32,
    pub header_properties: Vec<GpuProperty>,
    pub levels: Vec<GpuLevel>,
}

/// Complete ordered GPU power-level sibling set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTable {
    pub groups: Vec<GpuGroup>,
}

/// Validated data carried by a KonaBess export URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonaBessExport {
    pub chip: String,
    pub description: String,
    pub table: GpuTable,
    /// Non-blocking findings discovered while reading the export envelope.
    pub import_warnings: Vec<GpuTableIssue>,
}

/// One table-validation finding suitable for display next to an editor cell or row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTableIssue {
    /// Stable, human-readable location such as `group 0 / level 2 / qcom,gpu-freq`.
    pub path: String,
    /// Explanation intended for callers to present to the user.
    pub message: String,
}

/// Blocking structural errors and non-blocking tuning advisories for a GPU table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuTableValidation {
    /// Malformed data that cannot be serialized safely.
    pub hard_errors: Vec<GpuTableIssue>,
    /// Suspicious but serializable data that callers may allow after warning.
    pub warnings: Vec<GpuTableIssue>,
}

impl GpuTableValidation {
    /// Whether applying this table must be blocked.
    pub fn has_hard_errors(&self) -> bool {
        !self.hard_errors.is_empty()
    }
}

/// Build a new level with exactly the property names and ordering of an
/// existing level in `group` while letting the caller choose every cell value.
///
/// Using this constructor for editor row insertion prevents properties such as
/// bus votes from being omitted accidentally. The callback receives each
/// template property in source order and returns the cells for the new row.
///
/// # Errors
///
/// Returns an error when `template_level_id` does not identify a level in
/// `group`.
pub fn build_gpu_level_from_template(
    group: &GpuGroup,
    template_level_id: u32,
    new_level_id: u32,
    mut cells_for_property: impl FnMut(&GpuProperty) -> Vec<u32>,
) -> Result<GpuLevel> {
    let template = group
        .levels
        .iter()
        .find(|level| level.id == template_level_id)
        .ok_or_else(|| {
            error(format!(
                "GPU group {} has no template level {template_level_id}",
                group.id
            ))
        })?;
    Ok(GpuLevel {
        id: new_level_id,
        properties: template
            .properties
            .iter()
            .map(|property| GpuProperty {
                name: property.name.clone(),
                cells: cells_for_property(property),
            })
            .collect(),
    })
}

/// Validate an in-memory table without treating tuning policy as a hard error.
///
/// The observed stock envelope is 125 MHz through 1.3 GHz for `qcom,gpu-freq`
/// and 50 through 452 for the encoded `qcom,level` regulator vote. Values
/// outside those envelopes remain writable and are reported only as warnings.
pub fn validate_gpu_table(table: &GpuTable) -> GpuTableValidation {
    const STOCK_MIN_FREQUENCY: u32 = 125_000_000;
    const STOCK_MAX_FREQUENCY: u32 = 1_300_000_000;
    const STOCK_MIN_LEVEL_VOTE: u32 = 50;
    const STOCK_MAX_LEVEL_VOTE: u32 = 452;

    let mut validation = GpuTableValidation::default();
    if table.groups.is_empty() {
        validation
            .hard_errors
            .push(issue("table", "GPU table must contain at least one group"));
        return validation;
    }

    for (group_position, group) in table.groups.iter().enumerate() {
        let group_path = format!("group {}", group.id);
        if table.groups[..group_position]
            .iter()
            .any(|prior| prior.id == group.id)
        {
            validation
                .hard_errors
                .push(issue(&group_path, "duplicate GPU group id"));
        }
        validate_properties(
            &group.header_properties,
            &group_path,
            PropertyLocation::GroupHeader,
            &mut validation,
        );
        if group.levels.is_empty() {
            validation.hard_errors.push(issue(
                &group_path,
                "GPU group must contain at least one level",
            ));
            continue;
        }

        let mut frequencies = Vec::with_capacity(group.levels.len());
        for (level_position, level) in group.levels.iter().enumerate() {
            let level_path = format!("{group_path} / level {}", level.id);
            if group.levels[..level_position]
                .iter()
                .any(|prior| prior.id == level.id)
            {
                validation
                    .hard_errors
                    .push(issue(&level_path, "duplicate GPU level id"));
            }
            validate_properties(
                &level.properties,
                &level_path,
                PropertyLocation::Level,
                &mut validation,
            );

            match scalar_property(&level.properties, "qcom,gpu-freq") {
                Some(frequency) => {
                    frequencies.push((level_path.clone(), frequency));
                    if !(STOCK_MIN_FREQUENCY..=STOCK_MAX_FREQUENCY).contains(&frequency) {
                        validation.warnings.push(issue(
                            format!("{level_path} / qcom,gpu-freq"),
                            format!(
                                "frequency {frequency} Hz is outside the observed stock range {STOCK_MIN_FREQUENCY}..={STOCK_MAX_FREQUENCY} Hz"
                            ),
                        ));
                    }
                }
                None => validation.hard_errors.push(issue(
                    format!("{level_path} / qcom,gpu-freq"),
                    "required property must contain exactly one u32 cell",
                )),
            }

            match scalar_property(&level.properties, "qcom,level") {
                Some(level_vote) => {
                    if !(STOCK_MIN_LEVEL_VOTE..=STOCK_MAX_LEVEL_VOTE).contains(&level_vote) {
                        validation.warnings.push(issue(
                            format!("{level_path} / qcom,level"),
                            format!(
                                "encoded regulator vote {level_vote} is outside the observed stock range {STOCK_MIN_LEVEL_VOTE}..={STOCK_MAX_LEVEL_VOTE}"
                            ),
                        ));
                    }
                }
                None => validation.hard_errors.push(issue(
                    format!("{level_path} / qcom,level"),
                    "required property must contain exactly one u32 cell",
                )),
            }
        }

        for pair in frequencies.windows(2) {
            if pair[0].1 <= pair[1].1 {
                validation.warnings.push(issue(
                    &group_path,
                    format!(
                        "frequencies are not strictly descending at {} Hz then {} Hz",
                        pair[0].1, pair[1].1
                    ),
                ));
            }
        }
    }
    validation
}

/// Parse one editor cell as a decimal or `0x`-prefixed unsigned 32-bit value.
///
/// Empty, signed, overflowing, and otherwise non-u32 input is a hard error.
pub fn parse_gpu_cell(input: &str) -> Result<u32> {
    let cell = input.trim();
    if cell.is_empty() {
        return Err(error("GPU table cell is empty"));
    }
    let parsed = if let Some(hex) = cell.strip_prefix("0x").or_else(|| cell.strip_prefix("0X")) {
        if hex.is_empty() {
            return Err(error("GPU table hexadecimal cell is empty"));
        }
        u32::from_str_radix(hex, 16)
    } else {
        cell.parse()
    };
    parsed.map_err(|_| error(format!("GPU table cell `{cell}` is not a u32")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyLocation {
    GroupHeader,
    Level,
}

fn validate_properties(
    properties: &[GpuProperty],
    parent_path: &str,
    location: PropertyLocation,
    validation: &mut GpuTableValidation,
) {
    for (position, property) in properties.iter().enumerate() {
        let property_path = format!("{parent_path} / {}", property.name);
        if property.name.is_empty() || property.name.contains('\0') {
            validation.hard_errors.push(issue(
                &property_path,
                "property name is empty or contains NUL",
            ));
        }
        if property.cells.is_empty() {
            validation.hard_errors.push(issue(
                &property_path,
                "property must contain at least one u32 cell",
            ));
        }
        if properties[..position]
            .iter()
            .any(|prior| prior.name == property.name)
        {
            validation
                .hard_errors
                .push(issue(&property_path, "duplicate property name"));
        }

        let must_be_scalar = match location {
            PropertyLocation::GroupHeader => matches!(
                property.name.as_str(),
                "qcom,initial-pwrlevel" | "qcom,initial-min-pwrlevel"
            ),
            PropertyLocation::Level => true,
        };
        if must_be_scalar && property.cells.len() != 1 {
            validation.hard_errors.push(issue(
                &property_path,
                "property must contain exactly one u32 cell",
            ));
        }
    }

    if location == PropertyLocation::Level && scalar_property(properties, "reg").is_none() {
        validation.hard_errors.push(issue(
            format!("{parent_path} / reg"),
            "required property must contain exactly one u32 cell",
        ));
    }
}

fn scalar_property(properties: &[GpuProperty], name: &str) -> Option<u32> {
    let cells = &properties
        .iter()
        .find(|property| property.name == name)?
        .cells;
    (cells.len() == 1).then(|| cells[0])
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> GpuTableIssue {
    GpuTableIssue {
        path: path.into(),
        message: message.into(),
    }
}

#[derive(Deserialize)]
struct ExportJson {
    chip: String,
    desc: String,
    freq: String,
    #[serde(default)]
    volt: Option<serde_json::Value>,
    #[serde(flatten)]
    unknown_fields: BTreeMap<String, serde::de::IgnoredAny>,
}

/// Parse a complete `konabess://` export string.
pub fn parse_export(input: &str) -> Result<KonaBessExport> {
    let input = input.trim();
    let payload = input
        .strip_prefix(URI_PREFIX)
        .ok_or_else(|| error("export must start with konabess://"))?;
    if payload.is_empty() {
        return Err(error("export payload is empty"));
    }

    let compressed = decode_base64(payload)?;
    let json_bytes = decode_gzip(&compressed)?;
    let raw: ExportJson = serde_json::from_slice(&json_bytes)
        .map_err(|e| error(format!("invalid export JSON: {e}")))?;
    if raw.volt.is_some() {
        return Err(error("voltage tables are not supported"));
    }
    if raw.chip.trim().is_empty() {
        return Err(error("export chip is empty"));
    }

    let import_warnings = raw
        .unknown_fields
        .into_keys()
        .map(|field| GpuTableIssue {
            path: format!("export / {field}"),
            message: "unknown export field was ignored".to_string(),
        })
        .collect();

    Ok(KonaBessExport {
        chip: raw.chip,
        description: raw.desc,
        table: parse_frequency_table(&raw.freq)?,
        import_warnings,
    })
}

fn decode_base64(input: &str) -> Result<Vec<u8>> {
    // Standard alphabet, canonical padding required, no non-zero trailing bits.
    // Whitespace is not skipped: callers trim the whole URI beforehand.
    const STRICT: GeneralPurpose = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::RequireCanonical)
            .with_decode_allow_trailing_bits(false),
    );
    STRICT
        .decode(input)
        .map_err(|e| error(format!("invalid base64 payload: {e}")))
}

fn decode_gzip(input: &[u8]) -> Result<Vec<u8>> {
    if input.len() < 18 || input[..3] != [0x1f, 0x8b, 8] {
        return Err(error("payload is not a gzip stream"));
    }
    if input[3] & 0xe0 != 0 {
        return Err(error("gzip header uses reserved flags"));
    }

    // `bufread::GzDecoder` parses the header and verifies CRC32 and ISIZE. It
    // decodes a single member and leaves any bytes after it in the slice, which
    // lets the caller reject trailing data instead of ignoring it.
    let mut decoder = GzDecoder::new(input);
    let mut output = Vec::new();
    (&mut decoder)
        .take(MAX_EXPORT_JSON_SIZE as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|e| error(format!("cannot decompress gzip payload: {e}")))?;
    if output.len() > MAX_EXPORT_JSON_SIZE {
        return Err(error(format!(
            "decompressed export exceeds {MAX_EXPORT_JSON_SIZE} bytes"
        )));
    }
    if !decoder.get_ref().is_empty() {
        return Err(error("gzip payload has trailing data"));
    }
    Ok(output)
}

fn parse_frequency_table(input: &str) -> Result<GpuTable> {
    let mut groups = Vec::<GpuGroup>::new();
    let mut current_group: Option<GpuGroup> = None;
    let mut current_level: Option<GpuLevel> = None;

    for (line_index, raw_line) in input.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(id) = parse_node_header(line, "qcom,gpu-pwrlevels-", line_number)? {
            if current_group.is_some() || current_level.is_some() {
                return Err(line_error(line_number, "nested GPU group"));
            }
            current_group = Some(GpuGroup {
                id,
                header_properties: Vec::new(),
                levels: Vec::new(),
            });
            continue;
        }
        if let Some(id) = parse_node_header(line, "qcom,gpu-pwrlevel@", line_number)? {
            if current_group.is_none() || current_level.is_some() {
                return Err(line_error(line_number, "power level outside a GPU group"));
            }
            current_level = Some(GpuLevel {
                id,
                properties: Vec::new(),
            });
            continue;
        }
        if line == "};" {
            if let Some(level) = current_level.take() {
                validate_level(&level, line_number)?;
                current_group
                    .as_mut()
                    .expect("level state requires a group")
                    .levels
                    .push(level);
            } else if let Some(group) = current_group.take() {
                validate_group(&group, line_number)?;
                groups.push(group);
            } else {
                return Err(line_error(line_number, "unmatched node terminator"));
            }
            continue;
        }

        let property = parse_property(line, line_number)?;
        if let Some(level) = current_level.as_mut() {
            if property.cells.len() != 1 {
                return Err(line_error(
                    line_number,
                    "power-level properties must contain exactly one u32 cell",
                ));
            }
            level.properties.push(property);
        } else if let Some(group) = current_group.as_mut() {
            if property.cells.len() != 1 && property.name != "qcom,sku-codes" {
                return Err(line_error(
                    line_number,
                    "only qcom,sku-codes may contain multiple cells",
                ));
            }
            group.header_properties.push(property);
        } else {
            return Err(line_error(line_number, "property outside a GPU group"));
        }
    }

    if current_level.is_some() || current_group.is_some() {
        return Err(error("frequency table ends inside a node"));
    }
    if groups.is_empty() {
        return Err(error("frequency table has no GPU groups"));
    }
    ensure_unique_ids(groups.iter().map(|group| group.id), "GPU group")?;
    Ok(GpuTable { groups })
}

fn parse_node_header(line: &str, prefix: &str, line_number: usize) -> Result<Option<u32>> {
    let Some(rest) = line.strip_prefix(prefix) else {
        return Ok(None);
    };
    let id_text = rest
        .strip_suffix(" {")
        .ok_or_else(|| line_error(line_number, "malformed node header"))?;
    if id_text.is_empty() || !id_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(line_error(line_number, "node id must be decimal"));
    }
    let id = id_text
        .parse()
        .map_err(|_| line_error(line_number, "node id does not fit u32"))?;
    Ok(Some(id))
}

fn parse_property(line: &str, line_number: usize) -> Result<GpuProperty> {
    let (name, value) = line
        .strip_suffix(';')
        .and_then(|line| line.split_once(" = "))
        .ok_or_else(|| line_error(line_number, "expected `name = <cells>;`"))?;
    if name.is_empty()
        || !name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'#' | b',' | b'-' | b'_' | b'.')
        })
    {
        return Err(line_error(line_number, "invalid property name"));
    }
    let cells_text = value
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .ok_or_else(|| line_error(line_number, "property value must be u32 cells"))?;
    let cells = cells_text
        .split_ascii_whitespace()
        .map(|cell| parse_cell(cell, line_number))
        .collect::<Result<Vec<_>>>()?;
    if cells.is_empty() {
        return Err(line_error(line_number, "property has no cells"));
    }
    Ok(GpuProperty {
        name: name.to_string(),
        cells,
    })
}

fn parse_cell(cell: &str, line_number: usize) -> Result<u32> {
    parse_gpu_cell(cell).map_err(|_| line_error(line_number, format!("invalid u32 cell `{cell}`")))
}

fn validate_level(level: &GpuLevel, line_number: usize) -> Result<()> {
    if level.properties.is_empty() {
        return Err(line_error(line_number, "power level has no properties"));
    }
    ensure_unique_names(&level.properties, "power-level property")?;
    let reg = level
        .properties
        .iter()
        .find(|property| property.name == "reg")
        .ok_or_else(|| line_error(line_number, "power level has no reg property"))?;
    if reg.cells[0] != level.id {
        return Err(line_error(
            line_number,
            format!(
                "power-level node id {} does not match reg {}",
                level.id, reg.cells[0]
            ),
        ));
    }
    Ok(())
}

fn validate_group(group: &GpuGroup, line_number: usize) -> Result<()> {
    if group.levels.is_empty() {
        return Err(line_error(line_number, "GPU group has no power levels"));
    }
    ensure_unique_names(&group.header_properties, "group header property")?;
    let selector_count = group
        .header_properties
        .iter()
        .filter(|property| matches!(property.name.as_str(), "qcom,speed-bin" | "qcom,sku-codes"))
        .count();
    if selector_count == 0 {
        return Err(line_error(
            line_number,
            "GPU group must have a speed-bin or sku-codes selector",
        ));
    }
    ensure_unique_ids(group.levels.iter().map(|level| level.id), "power level")
}

fn ensure_unique_names(properties: &[GpuProperty], kind: &str) -> Result<()> {
    for (index, property) in properties.iter().enumerate() {
        if properties[..index]
            .iter()
            .any(|prior| prior.name == property.name)
        {
            return Err(error(format!("duplicate {kind} `{}`", property.name)));
        }
    }
    Ok(())
}

fn ensure_unique_ids(ids: impl IntoIterator<Item = u32>, kind: &str) -> Result<()> {
    let mut seen = Vec::new();
    for id in ids {
        if seen.contains(&id) {
            return Err(error(format!("duplicate {kind} id {id}")));
        }
        seen.push(id);
    }
    Ok(())
}

fn line_error(line: usize, message: impl Into<String>) -> ltbox_core::LtboxError {
    error(format!("frequency table line {line}: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_EXPORT: &str = "konabess://H4sIAAAAAAAACmWOywrCMBBFf6WM2wTiYxUf+CFu2mSsAzGmmUbF0n+XpEUUF7O551zmDmAuFEADJw8CLLIBDclTDwLOETvQ0JnbVbQhyfCIDu/oWKpqOPmSc0C0siFf7audejZ42M6EPPVUu09rEpaZL5heKA06x1OqSlpbG5H5GxT9b8Cx/I/YFulHyZvn6mq9yWgsB+MbZCgNFesAAAA=";
    const UNKNOWN_FIELD_EXPORT: &str = "konabess://H4sIAAAAAAAACmWPywrCMBBFf6WM2wbqYxUf+CGCtMm1BtKY5qFi6b9L0iKKi9nccy4zM5C4KkucfDRUkoQXxCkaFaiki0NPnHpx68rWRmYfTuMO7VlVDCeTc28ByRplin2xq54NDtuZKKOCqvWnNQnLxBdevcAEtPZTWuW0ltLB+2+Q9b8Djnm/Q5ulHyXdPFdX601CY570TgzR4dzdJIgPBFM3GpJ4cBHj+AbhUQm8CgEAAA==";

    #[test]
    fn parses_typed_export() {
        let export = parse_export(VALID_EXPORT).unwrap();
        assert_eq!(export.chip, "sun");
        assert_eq!(export.description, "unit");
        assert!(export.import_warnings.is_empty());
        assert_eq!(export.table.groups.len(), 1);
        assert_eq!(export.table.groups[0].id, 0);
        assert_eq!(export.table.groups[0].levels[0].id, 0);
        assert_eq!(
            export.table.groups[0].levels[0].properties[1].cells,
            [0x1234]
        );
    }

    #[test]
    fn unknown_top_level_field_is_ignored_and_reported() {
        let export = parse_export(UNKNOWN_FIELD_EXPORT).unwrap();

        assert_eq!(export.chip, "sun");
        assert_eq!(
            export.import_warnings,
            [GpuTableIssue {
                path: "export / future_mode".into(),
                message: "unknown export field was ignored".into(),
            }]
        );

        let missing_frequency = serde_json::from_str::<ExportJson>(
            r#"{"chip":"sun","desc":"unit","future_mode":true}"#,
        )
        .err()
        .expect("missing required frequency must be rejected");
        assert!(
            missing_frequency
                .to_string()
                .contains("missing field `freq`")
        );
    }

    #[test]
    fn rejects_malformed_export() {
        let error = parse_export("konabess://not-base64").unwrap_err();
        assert!(error.to_string().contains("base64"));
    }

    #[test]
    fn base64_is_strict() {
        assert_eq!(decode_base64("QUJD").unwrap(), b"ABC");
        assert_eq!(decode_base64("QUI=").unwrap(), b"AB");
        assert_eq!(decode_base64("QQ==").unwrap(), b"A");
        // Missing padding, whitespace, URL-safe alphabet, and padding inside the stream.
        assert!(decode_base64("QUI").is_err());
        assert!(
            decode_base64(
                "QUJD
"
            )
            .is_err()
        );
        assert!(decode_base64("QU JD").is_err());
        assert!(decode_base64("QU-_").is_err());
        assert!(decode_base64("QQ==QUJD").is_err());
        // Non-zero trailing bits before padding.
        assert!(decode_base64("QR==").is_err());
        assert!(decode_base64("QUJ=").is_err());
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        use flate2::{Compression, write::GzEncoder};
        use std::io::Write;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn gzip_round_trips_and_rejects_corruption() {
        let packed = gzip(b"hello konabess");
        assert_eq!(decode_gzip(&packed).unwrap(), b"hello konabess");

        let mut reserved = packed.clone();
        reserved[3] |= 0x20;
        assert!(
            decode_gzip(&reserved)
                .unwrap_err()
                .to_string()
                .contains("reserved flags")
        );

        let mut not_gzip = packed.clone();
        not_gzip[0] = 0;
        assert!(
            decode_gzip(&not_gzip)
                .unwrap_err()
                .to_string()
                .contains("not a gzip stream")
        );

        let mut bad_crc = packed.clone();
        let crc_at = bad_crc.len() - 8;
        bad_crc[crc_at] ^= 0xff;
        assert!(decode_gzip(&bad_crc).is_err());

        let mut bad_size = packed.clone();
        let size_at = bad_size.len() - 4;
        bad_size[size_at] ^= 0x01;
        assert!(decode_gzip(&bad_size).is_err());

        assert!(decode_gzip(&packed[..packed.len() - 3]).is_err());

        let mut trailing = packed.clone();
        trailing.extend_from_slice(b"junk");
        assert!(
            decode_gzip(&trailing)
                .unwrap_err()
                .to_string()
                .contains("trailing data")
        );

        let mut second_member = packed.clone();
        second_member.extend_from_slice(&packed);
        assert!(decode_gzip(&second_member).is_err());
    }

    #[test]
    fn gzip_enforces_the_decompressed_size_cap() {
        let at_cap = gzip(&vec![0u8; MAX_EXPORT_JSON_SIZE]);
        assert_eq!(decode_gzip(&at_cap).unwrap().len(), MAX_EXPORT_JSON_SIZE);

        let over_cap = gzip(&vec![0u8; MAX_EXPORT_JSON_SIZE + 1]);
        assert!(
            decode_gzip(&over_cap)
                .unwrap_err()
                .to_string()
                .contains("exceeds")
        );
    }

    #[test]
    fn rejects_narrow_format_violations() {
        let error = parse_frequency_table(
            "qcom,gpu-pwrlevels-0 {\nqcom,speed-bin = <0>;\nqcom,gpu-pwrlevel@1 {\nreg = <0>;\n};\n};",
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not match reg"));
    }

    #[test]
    fn rejects_non_u32_and_empty_cells() {
        assert!(parse_gpu_cell("").is_err());
        assert!(parse_gpu_cell("4294967296").is_err());
        assert_eq!(parse_gpu_cell("0xffffffff").unwrap(), u32::MAX);
        let non_u32 = parse_frequency_table(
            "qcom,gpu-pwrlevels-0 {\nqcom,speed-bin = <0>;\nqcom,initial-pwrlevel = <0>;\nqcom,gpu-pwrlevel@0 {\nreg = <0>;\nqcom,gpu-freq = <fast>;\nqcom,level = <100>;\n};\n};",
        )
        .unwrap_err();
        assert!(non_u32.to_string().contains("invalid u32 cell"));

        let mut table = GpuTable {
            groups: vec![GpuGroup {
                id: 0,
                header_properties: vec![GpuProperty {
                    name: "qcom,initial-pwrlevel".into(),
                    cells: vec![0],
                }],
                levels: vec![GpuLevel {
                    id: 0,
                    properties: vec![
                        GpuProperty {
                            name: "reg".into(),
                            cells: vec![0],
                        },
                        GpuProperty {
                            name: "qcom,gpu-freq".into(),
                            cells: vec![500_000_000],
                        },
                        GpuProperty {
                            name: "qcom,level".into(),
                            cells: vec![100],
                        },
                    ],
                }],
            }],
        };
        table.groups[0].levels[0].properties[1].cells.clear();
        assert!(validate_gpu_table(&table).has_hard_errors());
    }

    #[test]
    fn advisory_values_do_not_become_hard_errors() {
        let table = GpuTable {
            groups: vec![GpuGroup {
                id: 0,
                header_properties: vec![GpuProperty {
                    name: "qcom,initial-pwrlevel".into(),
                    cells: vec![0],
                }],
                levels: [100_000_000, 200_000_000]
                    .into_iter()
                    .enumerate()
                    .map(|(id, frequency)| GpuLevel {
                        id: id as u32,
                        properties: vec![
                            GpuProperty {
                                name: "reg".into(),
                                cells: vec![id as u32],
                            },
                            GpuProperty {
                                name: "qcom,gpu-freq".into(),
                                cells: vec![frequency],
                            },
                            GpuProperty {
                                name: "qcom,level".into(),
                                cells: vec![500],
                            },
                        ],
                    })
                    .collect(),
            }],
        };

        let validation = validate_gpu_table(&table);
        assert!(!validation.has_hard_errors());
        assert!(!validation.warnings.is_empty());
    }

    #[test]
    fn templated_level_preserves_the_complete_property_schema() {
        let mut table = GpuTable {
            groups: vec![GpuGroup {
                id: 7,
                header_properties: vec![],
                levels: vec![GpuLevel {
                    id: 0,
                    properties: vec![
                        GpuProperty {
                            name: "reg".into(),
                            cells: vec![0],
                        },
                        GpuProperty {
                            name: "qcom,gpu-freq".into(),
                            cells: vec![500_000_000],
                        },
                        GpuProperty {
                            name: "qcom,level".into(),
                            cells: vec![100],
                        },
                        GpuProperty {
                            name: "qcom,bus-freq".into(),
                            cells: vec![5],
                        },
                    ],
                }],
            }],
        };

        let new_level = build_gpu_level_from_template(&table.groups[0], 0, 1, |property| {
            vec![match property.name.as_str() {
                "reg" => 1,
                "qcom,gpu-freq" => 400_000_000,
                "qcom,level" => 80,
                "qcom,bus-freq" => 4,
                _ => property.cells[0],
            }]
        })
        .unwrap();
        assert_eq!(
            new_level
                .properties
                .iter()
                .map(|property| property.name.as_str())
                .collect::<Vec<_>>(),
            ["reg", "qcom,gpu-freq", "qcom,level", "qcom,bus-freq"]
        );
        table.groups[0].levels.push(new_level);
        assert!(!validate_gpu_table(&table).has_hard_errors());
    }

    #[test]
    fn absent_initial_power_level_properties_are_valid_but_malformed_present_ones_are_not() {
        let table = GpuTable {
            groups: vec![GpuGroup {
                id: 0,
                header_properties: vec![],
                levels: vec![GpuLevel {
                    id: 0,
                    properties: vec![
                        GpuProperty {
                            name: "reg".into(),
                            cells: vec![0],
                        },
                        GpuProperty {
                            name: "qcom,gpu-freq".into(),
                            cells: vec![500_000_000],
                        },
                        GpuProperty {
                            name: "qcom,level".into(),
                            cells: vec![100],
                        },
                    ],
                }],
            }],
        };

        assert!(!validate_gpu_table(&table).has_hard_errors());

        for property_name in ["qcom,initial-pwrlevel", "qcom,initial-min-pwrlevel"] {
            let mut malformed = table.clone();
            malformed.groups[0].header_properties.push(GpuProperty {
                name: property_name.into(),
                cells: vec![0, 1],
            });
            let validation = validate_gpu_table(&malformed);
            assert!(validation.has_hard_errors());
            assert!(
                validation
                    .hard_errors
                    .iter()
                    .any(|issue| issue.path.ends_with(property_name))
            );
        }
    }
}
