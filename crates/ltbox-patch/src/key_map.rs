//! AVB signing-key lookup by public-key SHA-1.
//!
//! Maps Lenovo stock pubkey SHA-1s to `avbtool-rs` embedded key specs
//! (`testkey_rsa2048` / `testkey_rsa4096`). PEMs ship inside avbtool-rs.

/// Stock pubkey SHA-1 → `avbtool-rs` key spec name.
/// Keep in sync whenever Lenovo rolls the signing key.
const KEY_MAP: &[(&str, &str)] = &[
    (
        "2597c218aae470a130f61162feaae70afd97f011",
        "testkey_rsa4096",
    ),
    (
        "cdbb77177f731920bbe0a0f94f84d9038ae0617d",
        "testkey_rsa2048",
    ),
];

/// Pubkey SHA-1s for builds signed with the **Lenovo production key**
/// (a non-testkey root of trust). Matched for classification only — these keys
/// are never used to re-sign (a testkey device's bootloader trusts the testkey,
/// so re-signs always target `testkey_rsa4096`).
const LENOVO_KEY_MAP: &[&str] = &["8fcb864f11f53ed11284615fb67685522085d3a2"];

/// Which root of trust an image's vbmeta pubkey belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyClass {
    /// Pubkey is in [`KEY_MAP`] — an AOSP testkey (testkey-vulnerability build).
    Testkey,
    /// Pubkey is in [`LENOVO_KEY_MAP`] — a build signed with the Lenovo key,
    /// where the AOSP testkey hole is closed.
    Lenovo,
    /// Pubkey is empty/absent or matches neither map.
    Unknown,
}

/// Classify a vbmeta `public_key_sha1` into its root of trust. An empty/absent
/// pubkey is treated as `Unknown` (callers gate writes on a positive class).
pub fn classify_pubkey(pubkey_sha1: Option<&str>) -> KeyClass {
    let Some(sha1) = pubkey_sha1.map(str::trim).filter(|s| !s.is_empty()) else {
        return KeyClass::Unknown;
    };
    if has_key_for(sha1) {
        KeyClass::Testkey
    } else if LENOVO_KEY_MAP.iter().any(|k| k.eq_ignore_ascii_case(sha1)) {
        KeyClass::Lenovo
    } else {
        KeyClass::Unknown
    }
}

/// Resolve a `public_key_sha1` to an `avbtool-rs` key spec name.
/// Returns `None` for an empty / unknown pubkey.
pub fn key_spec_for_pubkey(pubkey_sha1: Option<&str>) -> Option<&'static str> {
    let sha1 = pubkey_sha1?.trim();
    if sha1.is_empty() {
        return None;
    }
    KEY_MAP
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(sha1))
        .map(|(_, spec)| *spec)
}

/// Resolve a signed pubkey to a bundled key.
///
/// Missing / empty pubkey means unsigned and is allowed. A present but unknown
/// pubkey is a mismatch and must abort before writes.
pub fn key_spec_for_signed_pubkey(
    pubkey_sha1: Option<&str>,
) -> Result<Option<&'static str>, String> {
    let Some(sha1) = pubkey_sha1.map(str::trim).filter(|sha| !sha.is_empty()) else {
        return Ok(None);
    };
    key_spec_for_pubkey(Some(sha1))
        .map(Some)
        .ok_or_else(|| sha1.to_string())
}

/// The user-facing message for a signing key that cannot be resolved.
///
/// A [`KeyClass::Lenovo`] key means firmware that closed the AOSP test-key
/// hole, which retrying cannot fix; any other key is reported as unknown, with
/// the diagnostic. Both messages leave `{work}` for the GUI to fill with the
/// running operation, which is not known at this depth.
pub fn unresolved_signing_key_error(image: &str, pubkey_sha1: &str) -> String {
    if classify_pubkey(Some(pubkey_sha1)) == KeyClass::Lenovo {
        ltbox_core::i18n::tr("err_device_key_lenovo")
    } else {
        ltbox_core::tr_args!(
            "err_avb_signing_key_unknown",
            image = image,
            key = pubkey_sha1
        )
    }
}

/// True iff the bundled map knows a key for this pubkey SHA-1.
pub fn has_key_for(pubkey_sha1: &str) -> bool {
    KEY_MAP
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case(pubkey_sha1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_4096_pubkey_resolves() {
        let spec = key_spec_for_pubkey(Some("2597c218aae470a130f61162feaae70afd97f011"));
        assert_eq!(spec, Some("testkey_rsa4096"));
    }

    #[test]
    fn known_2048_pubkey_resolves() {
        let spec = key_spec_for_pubkey(Some("cdbb77177f731920bbe0a0f94f84d9038ae0617d"));
        assert_eq!(spec, Some("testkey_rsa2048"));
    }

    #[test]
    fn pubkey_lookup_is_case_insensitive() {
        let spec = key_spec_for_pubkey(Some("2597C218AAE470A130F61162FEAAE70AFD97F011"));
        assert_eq!(spec, Some("testkey_rsa4096"));
        assert!(has_key_for("2597C218AAE470A130F61162FEAAE70AFD97F011"));
    }

    #[test]
    fn unknown_pubkey_returns_none() {
        assert!(key_spec_for_pubkey(Some("deadbeef")).is_none());
        assert!(!has_key_for("deadbeef"));
    }

    #[test]
    fn empty_or_missing_pubkey_returns_none() {
        assert!(key_spec_for_pubkey(Some("")).is_none());
        assert!(key_spec_for_pubkey(None).is_none());
    }

    #[test]
    fn signed_pubkey_guard_accepts_key_map_and_allows_unsigned() {
        assert_eq!(
            key_spec_for_signed_pubkey(Some("2597c218aae470a130f61162feaae70afd97f011")),
            Ok(Some("testkey_rsa4096"))
        );
        assert_eq!(
            key_spec_for_signed_pubkey(Some("cdbb77177f731920bbe0a0f94f84d9038ae0617d")),
            Ok(Some("testkey_rsa2048"))
        );
        assert_eq!(key_spec_for_signed_pubkey(None), Ok(None));
        assert_eq!(key_spec_for_signed_pubkey(Some("")), Ok(None));
        assert_eq!(
            key_spec_for_signed_pubkey(Some("deadbeef")),
            Err("deadbeef".to_string())
        );
    }

    #[test]
    fn resolved_spec_loads_from_avbtool_rs() {
        // Sanity: avbtool-rs must accept the returned spec so resign/rebuild doesn't fail.
        let spec = key_spec_for_pubkey(Some("2597c218aae470a130f61162feaae70afd97f011")).unwrap();
        let key = avbtool_rs::crypto::load_key_from_spec(spec)
            .expect("avbtool-rs must accept the bundled spec");
        assert!(
            key.algorithm()
                .map(|a| a.starts_with("SHA256_RSA"))
                .unwrap_or(false),
            "embedded 4096 key should report a SHA256_RSA* algorithm",
        );
    }

    #[test]
    fn a_lenovo_device_key_is_not_reported_as_an_unknown_key() {
        // The two have different causes: one is a device that can never run the
        // operation, the other is an image signed with something unexpected.
        // Collapsing them back into one message is the regression to catch.
        let lenovo = unresolved_signing_key_error("vbmeta.img", LENOVO_KEY_MAP[0]);
        let unknown = unresolved_signing_key_error("vbmeta.img", "1234567890abcdef1234");
        assert_ne!(lenovo, unknown);
        // A testkey device resolves and never reaches either message.
        assert!(key_spec_for_signed_pubkey(Some(KEY_MAP[0].0)).is_ok());
    }
}
