//! Lenovo OTA `querynewfirmware` client.
//!
//! Resolves a device serial + firmware id against the public OTA endpoint and
//! returns the `<firmware>` block as a slim struct for the dashboard's OTA
//! popup.
//!
//! An empty `<firmwareupdate/>` element (no OTA for that firmware) is
//! `Ok(None)`, not an error.

use crate::error::{LtboxError, Result};

// Base64-obfuscated so the host doesn't surface in code search; decoded at
// runtime via `obf::reveal` (see `obf.rs` — not security).
const ENDPOINT_B64: &str =
    "aHR0cHM6Ly9vdGEubGVub3ZvLmNvbS9vdGEtc2VydmVyL2Zpcm13YXJlL3F1ZXJ5L2Zvci10ZXh0LWRlc2M=";

/// Slim `<firmware>` entry; fields the GUI doesn't render are dropped on parse.
#[derive(Debug, Clone, Default)]
pub struct OtaUpdate {
    /// Source firmware id — left side of the `_to_` split in `<name>`.
    pub from: String,
    /// Target firmware id — right side of the `_to_` split.
    pub to: String,
    /// MD5 hex of the upgrade package.
    pub md5: String,
    /// Package size in bytes. `None` when `<size>` is missing or not a `u64`,
    /// so callers can show "unknown" rather than "0.0 MB".
    pub size_bytes: Option<u64>,
    /// English changelog (`<desc_en>`), CDATA stripped.
    pub desc_en: String,
    /// Simplified-Chinese changelog (`<desc_cn>`), CDATA stripped.
    pub desc_cn: String,
    /// Direct download URL (`<downloadurl>`), CDATA stripped.
    pub download_url: String,
}

/// Trim a `ro.build.display.id` value to its first two underscore-separated
/// tokens, which the OTA server expects as `devicemodel` (e.g.
/// `TB322FC_CN_OPEN_USER_…` → `TB322FC_CN`). Returns the input verbatim if it
/// has fewer than two underscores.
pub fn devicemodel_from_firmware(firmware_id: &str) -> String {
    let mut splitter = firmware_id.splitn(3, '_');
    match (splitter.next(), splitter.next()) {
        (Some(a), Some(b)) => format!("{a}_{b}"),
        _ => firmware_id.to_string(),
    }
}

/// Blocking GET against the OTA endpoint; `Ok(None)` when no OTA is offered.
pub fn fetch_ota(serial: &str, firmware_id: &str) -> Result<Option<OtaUpdate>> {
    let serial = serial.trim();
    let firmware_id = firmware_id.trim();
    if serial.is_empty() || firmware_id.is_empty() {
        return Err(LtboxError::Other("empty serial or firmware id".to_string()));
    }
    let device_model = devicemodel_from_firmware(firmware_id);
    let url = format!(
        "{}?locale=en&deviceid={serial}&action=querynewfirmware&devicemodel={device_model}&curfirmwarever={firmware_id}",
        crate::obf::reveal(ENDPOINT_B64)
    );
    let agent = crate::downloader::build_agent();
    let mut resp = agent
        .get(&url)
        .call()
        .map_err(|e| LtboxError::Download(format!("Lenovo OTA GET: {e}")))?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| LtboxError::Download(format!("Lenovo OTA body: {e}")))?;
    parse_ota_xml(&body)
}

/// Parse the OTA XML body (separate from [`fetch_ota`] so tests need no network).
pub fn parse_ota_xml(xml: &str) -> Result<Option<OtaUpdate>> {
    let doc = crate::xml::parse(xml).map_err(|e| LtboxError::Other(format!("OTA XML: {e}")))?;
    let root = doc.root_element();
    let Some(firmware) = root.children().find(|n| n.has_tag_name("firmware")) else {
        // Empty `<firmwareupdate/>` — no OTA staged for this firmware.
        return Ok(None);
    };
    let text_of = |tag: &str| -> String {
        firmware
            .children()
            .find(|n| n.has_tag_name(tag))
            .and_then(|n| n.text())
            .map(|s| strip_cdata(s).trim().to_string())
            .unwrap_or_default()
    };
    let name = text_of("name");
    let (from, to) = match name.split_once("_to_") {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (name.clone(), String::new()),
    };
    let md5 = text_of("md5");
    let size_raw = text_of("size");
    let size_bytes = if size_raw.is_empty() {
        None
    } else {
        size_raw.parse::<u64>().ok()
    };
    let desc_en = text_of("desc_en");
    let desc_cn = text_of("desc_cn");
    let download_url = text_of("downloadurl");
    Ok(Some(OtaUpdate {
        from,
        to,
        md5,
        size_bytes,
        desc_en,
        desc_cn,
        download_url,
    }))
}

/// Strip a leftover `<![CDATA[ ... ]]>` wrapper, else return the trimmed input.
/// `roxmltree` usually unwraps CDATA already, but some payloads keep the markers.
fn strip_cdata(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("<![CDATA[")
        && let Some(inner) = rest.strip_suffix("]]>")
    {
        return inner.to_string();
    }
    trimmed.to_string()
}

/// Render a byte count (decimal units) as `MB` below 1 GB, otherwise `GB`;
/// `None` renders as `?`.
pub fn format_size(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else {
        return "?".to_string();
    };
    const ONE_GB: u64 = 1_000_000_000;
    if bytes >= ONE_GB {
        let gb = bytes as f64 / 1_000_000_000.0;
        format!("{gb:.2} GB")
    } else {
        let mb = bytes as f64 / 1_000_000.0;
        format!("{mb:.1} MB")
    }
}

/// Turn Lenovo's `;`-separated changelog into one item per line.
pub fn format_changelog(desc: &str) -> String {
    desc.split(';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devicemodel_trims_to_two_tokens() {
        assert_eq!(devicemodel_from_firmware("AAA_BB_CC_DD"), "AAA_BB");
        assert_eq!(devicemodel_from_firmware("AAA_BB"), "AAA_BB");
        assert_eq!(devicemodel_from_firmware("AAA"), "AAA");
        assert_eq!(devicemodel_from_firmware(""), "");
    }

    #[test]
    fn empty_root_means_none() {
        let xml = r#"<firmwareupdate xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:noNamespaceSchemaLocation="firmware.xsd"/>"#;
        assert!(matches!(parse_ota_xml(xml), Ok(None)));
    }

    #[test]
    fn parses_full_payload() {
        let xml = r#"<firmwareupdate>
<firmware>
<name>FOO_BAR_OPEN_USER_X_to_FOO_BAR_OPEN_USER_Y</name>
<desc_en><![CDATA[ a; b; c ]]></desc_en>
<desc_cn><![CDATA[ 가;나 ]]></desc_cn>
<md5>deadbeef</md5>
<size>1500000000</size>
<downloadurl><![CDATA[ https://example.invalid/x.zip ]]></downloadurl>
</firmware>
</firmwareupdate>"#;
        let got = parse_ota_xml(xml).unwrap().expect("some");
        assert_eq!(got.from, "FOO_BAR_OPEN_USER_X");
        assert_eq!(got.to, "FOO_BAR_OPEN_USER_Y");
        assert_eq!(got.desc_en, "a; b; c");
        assert_eq!(got.md5, "deadbeef");
        assert_eq!(got.size_bytes, Some(1_500_000_000));
        assert_eq!(got.download_url, "https://example.invalid/x.zip");
    }

    #[test]
    fn size_formatter_picks_unit() {
        assert_eq!(format_size(Some(0)), "0.0 MB");
        assert_eq!(format_size(Some(500_000_000)), "500.0 MB");
        assert_eq!(format_size(Some(1_000_000_000)), "1.00 GB");
        assert_eq!(format_size(Some(2_500_000_000)), "2.50 GB");
        assert_eq!(format_size(None), "?");
    }

    #[test]
    fn changelog_splits_on_semicolons() {
        assert_eq!(format_changelog("a; b ;c"), "a\nb\nc");
        assert_eq!(format_changelog(""), "");
    }
}
