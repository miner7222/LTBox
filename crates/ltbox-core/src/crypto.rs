//! AES-CBC decryption for .x firmware files.
//!
//! Key: PBKDF1(SHA-256, "OSD", salt, 1000, 32). Cipher: AES-256-CBC.
//! File: [IV:16][Salt:16][Encrypted body].
//! Plaintext: [original_size:i64LE][signature:8][data][sha256:32].

use aes::Aes256;
use cbc::cipher::{BlockModeDecrypt, KeyIvInit};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::error::{LtboxError, Result};

const PASSWORD: &[u8] = b"OSD";
const SIGNATURE: &[u8] = &[0xcf, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0xfc];

type Aes256CbcDec = cbc::Decryptor<Aes256>;

/// PBKDF1: hash(password+salt) iterated `iterations` times, truncated to `len_out`.
fn pbkdf1_sha256(password: &[u8], salt: &[u8], iterations: u32, len_out: usize) -> Vec<u8> {
    let mut digest = {
        let mut h = Sha256::new();
        h.update(password);
        h.update(salt);
        h.finalize().to_vec()
    };
    for _ in 1..iterations {
        let mut h = Sha256::new();
        h.update(&digest);
        digest = h.finalize().to_vec();
    }
    digest.truncate(len_out);
    digest
}

/// Decrypt a .x file. Returns the original (unencrypted) size.
pub fn decrypt_file(input: &Path, output: &Path) -> Result<u64> {
    let data = std::fs::read(input)
        .map_err(|e| LtboxError::Config(format!("Cannot read {}: {e}", input.display())))?;

    if data.len() < 32 {
        return Err(LtboxError::Config("Encrypted file too small".into()));
    }

    let iv = &data[..16];
    let salt = &data[16..32];
    let encrypted = &data[32..];

    let key = pbkdf1_sha256(PASSWORD, salt, 1000, 32);

    let mut buf = encrypted.to_vec();
    let decryptor = Aes256CbcDec::new_from_slices(&key, iv)
        .map_err(|e| LtboxError::Config(format!("Cipher init error: {e}")))?;
    let plain = decryptor
        .decrypt_padded::<cbc::cipher::block_padding::NoPadding>(&mut buf)
        .map_err(|e| LtboxError::Config(format!("Decryption error: {e}")))?;

    if plain.len() < 16 {
        return Err(LtboxError::Config("Decrypted data too small".into()));
    }
    // Header size is i64 LE. Reject a negative size (hostile salt) before the
    // `as u64` cast turns it into u64::MAX and the slice panics. The 8-byte
    // conversion cannot fail after the `plain.len() < 16` check above.
    let header_bytes: [u8; 8] = plain
        .get(0..8)
        .ok_or_else(|| LtboxError::Config("Decrypted data too small for size header".into()))?
        .try_into()
        .expect("slice of len 8 converts to [u8; 8]");
    let original_size_i64 = i64::from_le_bytes(header_bytes);
    let signature = &plain[8..16];

    if signature != SIGNATURE {
        return Err(LtboxError::Config("Invalid decryption signature".into()));
    }

    if original_size_i64 < 0 {
        return Err(LtboxError::Config(format!(
            "Invalid original_size in header: {original_size_i64}"
        )));
    }
    let original_size = original_size_i64 as u64;
    // Checked arithmetic: 16 prefix + size + 32 SHA must fit in usize and the buffer.
    let body_end: usize = 16usize
        .checked_add(usize::try_from(original_size).map_err(|_| {
            LtboxError::Config(format!("original_size {original_size} exceeds usize"))
        })?)
        .ok_or_else(|| {
            LtboxError::Config(format!("Header arithmetic overflow (size={original_size})"))
        })?;
    let hash_end: usize = body_end
        .checked_add(32)
        .ok_or_else(|| LtboxError::Config("Trailing SHA offset overflow".into()))?;
    if hash_end > plain.len() {
        return Err(LtboxError::Config("Truncated decrypted data".into()));
    }

    let body = &plain[16..body_end];
    let expected_hash = &plain[body_end..hash_end];

    let actual_hash = Sha256::digest(body);
    if actual_hash.as_slice() != expected_hash {
        return Err(LtboxError::Config("SHA-256 hash mismatch".into()));
    }

    std::fs::write(output, body)
        .map_err(|e| LtboxError::Config(format!("Cannot write {}: {e}", output.display())))?;

    Ok(original_size)
}

/// Lowercase hex SHA-256 of a file, streamed in 64 KiB chunks.
pub fn sha256_file_hex(path: &Path) -> std::io::Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Require `path` to match a GitHub `sha256:<hex>` asset digest.
///
/// For downloads that run with privilege: an absent, malformed or
/// non-SHA-256 digest fails exactly like a mismatch.
pub fn verify_github_sha256_digest(path: &Path, reported: Option<&str>) -> Result<()> {
    let reported = reported.unwrap_or_default();
    let expected = reported
        .strip_prefix("sha256:")
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| {
            LtboxError::Download(format!(
                "{}: no usable SHA-256 digest reported (got {reported:?})",
                path.display()
            ))
        })?;
    let actual = sha256_file_hex(path)?;
    if !expected.eq_ignore_ascii_case(&actual) {
        return Err(LtboxError::Download(format!(
            "{}: SHA-256 mismatch (expected {expected}, got {actual})",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn github_digest_accepts_only_a_matching_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(sha256_file_hex(&path).unwrap(), ABC_SHA256);
        let upper = format!("sha256:{}", ABC_SHA256.to_ascii_uppercase());
        verify_github_sha256_digest(&path, Some(&upper)).unwrap();

        let wrong = format!("sha256:{}", "0".repeat(64));
        let sha512 = format!("sha512:{ABC_SHA256}");
        for reported in [
            None,
            Some(""),
            Some(ABC_SHA256),
            Some("sha256:"),
            Some(sha512.as_str()),
            Some(wrong.as_str()),
        ] {
            assert!(
                verify_github_sha256_digest(&path, reported).is_err(),
                "{reported:?}"
            );
        }
    }

    #[test]
    fn pbkdf1_deterministic() {
        let key1 = pbkdf1_sha256(b"OSD", b"0123456789abcdef", 1000, 32);
        let key2 = pbkdf1_sha256(b"OSD", b"0123456789abcdef", 1000, 32);
        assert_eq!(key1, key2);
        assert_eq!(key1.len(), 32);
    }

    #[test]
    fn pbkdf1_different_salt() {
        let key1 = pbkdf1_sha256(b"OSD", b"salt1___________", 1000, 32);
        let key2 = pbkdf1_sha256(b"OSD", b"salt2___________", 1000, 32);
        assert_ne!(key1, key2);
    }

    /// Build a valid [IV][salt][encrypted plaintext] blob for regression tests.
    fn build_x_with_plaintext(plain: &[u8]) -> Vec<u8> {
        use cbc::cipher::{BlockModeEncrypt, KeyIvInit};
        type Aes256CbcEnc = cbc::Encryptor<Aes256>;

        let iv = [0u8; 16];
        let salt = [0u8; 16];
        let key = pbkdf1_sha256(b"OSD", &salt, 1000, 32);
        let cipher = Aes256CbcEnc::new_from_slices(&key, &iv).unwrap();

        // NoPadding requires 16-aligned plaintext.
        assert!(
            plain.len().is_multiple_of(16),
            "test plaintext must be 16-aligned"
        );
        let mut buf = plain.to_vec();
        let encrypted_len = buf.len();
        buf.resize(buf.len() + 16, 0);
        let ct = cipher
            .encrypt_padded::<cbc::cipher::block_padding::NoPadding>(&mut buf, encrypted_len)
            .unwrap();
        let ct_len = ct.len();
        buf.truncate(ct_len);

        let mut out = Vec::with_capacity(32 + buf.len());
        out.extend_from_slice(&iv);
        out.extend_from_slice(&salt);
        out.extend_from_slice(&buf);
        out
    }

    fn write_temp_bytes(bytes: &[u8]) -> std::path::PathBuf {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.x");
        std::fs::write(&path, bytes).unwrap();
        // Leak the tempdir; process exits well before OS cares.
        let leaked = path.clone();
        std::mem::forget(dir);
        leaked
    }

    #[test]
    fn negative_original_size_rejected_not_panic() {
        // original_size = -1 would bit-cast to u64::MAX and panic at the
        // slice instead of returning LtboxError::Config.
        let mut plain = Vec::new();
        plain.extend_from_slice(&(-1i64).to_le_bytes());
        plain.extend_from_slice(SIGNATURE);
        plain.extend_from_slice(&[0u8; 16]);
        let blob = build_x_with_plaintext(&plain);
        let input = write_temp_bytes(&blob);
        let output = input.with_extension("out");

        let err = decrypt_file(&input, &output).expect_err("must reject negative size");
        match err {
            LtboxError::Config(msg) => {
                assert!(
                    msg.contains("Invalid original_size") || msg.contains("overflow"),
                    "unexpected error: {msg}"
                );
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn huge_original_size_rejected_not_panic() {
        // i64::MAX must fail a bounds check (truncated data, or overflow on
        // 32-bit targets) with a Config error, not panic.
        let mut plain = Vec::new();
        plain.extend_from_slice(&i64::MAX.to_le_bytes());
        plain.extend_from_slice(SIGNATURE);
        plain.extend_from_slice(&[0u8; 16]);
        let blob = build_x_with_plaintext(&plain);
        let input = write_temp_bytes(&blob);
        let output = input.with_extension("out");

        let err = decrypt_file(&input, &output).expect_err("must reject huge size");
        assert!(matches!(err, LtboxError::Config(_)));
    }
}
