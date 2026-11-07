//! Extraction of the AVB public key embedded in Qualcomm ABL images.

use std::{collections::HashSet, io::Read, path::Path};

use ltbox_core::LtboxError;
use lzma_rust2::LzmaReader;
use sha1::{Digest, Sha1};

use crate::abl_pe::{
    self, MAX_DECOMPRESSED_SIZE, MAX_INPUT_SIZE, PeImage, TieBreak, read_u32_le, sign_extend,
};

const RSA_KEY_BITS: [u32; 3] = [2048, 4096, 8192];

#[derive(Debug)]
struct KeyBlob {
    raw_offset: usize,
    blob: Vec<u8>,
    references: usize,
}

/// Extract the AVB public-key SHA-1 (lowercase hex) embedded in an ABL ELF.
pub fn extract_abl_avb_pubkey_sha1(path: &Path) -> Result<String, LtboxError> {
    // An ABL image is a few hundred KiB. Refuse anything that could not hold a
    // decodable payload anyway rather than reading it into memory first.
    let size = fs_err::metadata(path)?.len();
    if size > MAX_INPUT_SIZE as u64 {
        return Err(LtboxError::Avb(format!(
            "ABL image is too large to inspect ({size} bytes)"
        )));
    }
    let raw = fs_err::read(path)?;
    extract_from_bytes(&raw)
}

fn extract_from_bytes(raw: &[u8]) -> Result<String, LtboxError> {
    let pe = load_linuxloader(raw)
        .ok_or_else(|| LtboxError::Avb("ABL LinuxLoader EFI not found".to_string()))?;
    let mut keys = find_key_blobs(&pe);
    let highest = keys
        .iter()
        .map(|key| key.references)
        .max()
        .ok_or_else(|| LtboxError::Avb("ABL AVB public-key blob not found".to_string()))?;
    keys.retain(|key| key.references == highest);
    keys.sort_unstable_by_key(|key| key.raw_offset);
    let key = keys
        .first()
        .ok_or_else(|| LtboxError::Avb("ABL AVB public-key blob not found".to_string()))?;
    let digest = Sha1::digest(&key.blob);
    let mut fingerprint = String::with_capacity(digest.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        fingerprint.push(char::from(HEX[usize::from(byte >> 4)]));
        fingerprint.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(fingerprint)
}

fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn decode_complete_stream(candidate: &[u8]) -> Option<Vec<u8>> {
    let mut reader =
        LzmaReader::new_mem_limit(candidate, (MAX_DECOMPRESSED_SIZE / 1024) as u32, None).ok()?;
    let mut decoded = Vec::new();
    reader
        .by_ref()
        .take((MAX_DECOMPRESSED_SIZE + 1) as u64)
        .read_to_end(&mut decoded)
        .ok()?;
    (64..=MAX_DECOMPRESSED_SIZE)
        .contains(&decoded.len())
        .then_some(decoded)
}

#[cfg(test)]
fn lzma_decode(stream: &[u8]) -> Option<Vec<u8>> {
    abl_pe::lzma_decode(stream, decode_complete_stream)
}

fn load_linuxloader(raw: &[u8]) -> Option<PeImage> {
    // The last of several equally large PEs wins (`Iterator::max_by_key`).
    abl_pe::load_linuxloader(raw, decode_complete_stream, TieBreak::Last)
}

fn bit_length_be(value: &[u8]) -> usize {
    let Some((index, first)) = value.iter().enumerate().find(|(_, byte)| **byte != 0) else {
        return 0;
    };
    (value.len() - index - 1) * 8 + (8 - first.leading_zeros() as usize)
}

fn be_bytes_to_limbs(value: &[u8], limb_count: usize) -> Vec<u32> {
    let mut limbs = vec![0u32; limb_count];
    for (index, chunk) in value.rchunks(4).enumerate().take(limb_count) {
        let mut bytes = [0u8; 4];
        bytes[4 - chunk.len()..].copy_from_slice(chunk);
        limbs[index] = u32::from_be_bytes(bytes);
    }
    limbs
}

fn limbs_cmp(left: &[u32], right: &[u32]) -> std::cmp::Ordering {
    left.iter().rev().cmp(right.iter().rev())
}

fn limbs_sub_assign(left: &mut [u32], right: &[u32]) {
    let mut borrow = 0u64;
    for (left_limb, right_limb) in left.iter_mut().zip(right) {
        let subtrahend = u64::from(*right_limb) + borrow;
        let minuend = u64::from(*left_limb);
        *left_limb = minuend.wrapping_sub(subtrahend) as u32;
        borrow = u64::from(minuend < subtrahend);
    }
}

fn montgomery_rr_matches(modulus_be: &[u8], rr_be: &[u8], modulus_bits: usize) -> bool {
    let limb_count = modulus_be.len().div_ceil(4);
    let modulus = be_bytes_to_limbs(modulus_be, limb_count);
    let expected = be_bytes_to_limbs(rr_be, limb_count);
    let mut value = vec![0u32; limb_count];
    value[0] = 1;

    for _ in 0..modulus_bits.saturating_mul(2) {
        let mut carry = 0u32;
        for limb in &mut value {
            let next = *limb >> 31;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
        if carry != 0 || limbs_cmp(&value, &modulus).is_ge() {
            limbs_sub_assign(&mut value, &modulus);
        }
    }
    value == expected
}

fn decode_key_candidate(data: &[u8], offset: usize, bits: u32) -> Option<Vec<u8>> {
    let modulus_size = usize::try_from(bits / 8).ok()?;
    let size = 8usize.checked_add(modulus_size.checked_mul(2)?)?;
    let blob = data.get(offset..offset.checked_add(size)?)?;
    if read_u32_be(blob, 0)? != bits {
        return None;
    }
    let n0inv = read_u32_be(blob, 4)?;
    let modulus_end = 8usize.checked_add(modulus_size)?;
    let modulus = blob.get(8..modulus_end)?;
    let rr = blob.get(modulus_end..size)?;
    let modulus_bits = bit_length_be(modulus);
    if !(bits as usize / 2 < modulus_bits && modulus_bits <= bits as usize)
        || modulus.last().is_none_or(|byte| byte & 1 == 0)
    {
        return None;
    }

    let low_word = read_u32_be(modulus, modulus_size - 4)?;
    let mut inverse = low_word;
    for _ in 0..5 {
        inverse = inverse.wrapping_mul(2u32.wrapping_sub(low_word.wrapping_mul(inverse)));
    }
    if n0inv != inverse.wrapping_neg() || !montgomery_rr_matches(modulus, rr, modulus_bits) {
        return None;
    }
    Some(blob.to_vec())
}

fn instruction_references(pe: &PeImage, key_start: usize, key_size: usize) -> usize {
    let Some(key_rva) = pe.raw_to_rva(key_start) else {
        return 0;
    };
    let Some(key_end) = key_start.checked_add(key_size) else {
        return 0;
    };
    let Some(key_rva_end) = key_rva.checked_add(key_size as i64) else {
        return 0;
    };
    let mut references = 0usize;

    for (raw_start, raw_end, section_rva) in pe.text_ranges() {
        let Some(first) = raw_start.checked_add((4 - (raw_start & 3)) & 3) else {
            continue;
        };
        let mut instruction = first;
        while instruction.checked_add(4).is_some_and(|end| end <= raw_end) {
            if key_start <= instruction && instruction < key_end {
                instruction += 4;
                continue;
            }
            let Some(word) = read_u32_le(&pe.data, instruction) else {
                break;
            };
            let pc = section_rva + (instruction - raw_start) as i64;

            if word & 0x9f00_0000 == 0x1000_0000 {
                let immlo = (word >> 29) & 3;
                let immhi = (word >> 5) & 0x7ffff;
                let target = pc + sign_extend((immhi << 2) | immlo, 21);
                if key_rva <= target && target < key_rva_end {
                    references += 1;
                }
                instruction += 4;
                continue;
            }

            if word & 0x9f00_0000 == 0x9000_0000 {
                let immlo = (word >> 29) & 3;
                let immhi = (word >> 5) & 0x7ffff;
                let page = (pc & !0xfff) + (sign_extend((immhi << 2) | immlo, 21) << 12);
                let register = word & 0x1f;
                let add_limit = instruction.saturating_add(20).min(raw_end);
                let mut add_offset = instruction + 4;
                while add_offset < add_limit {
                    let Some(add) = read_u32_le(&pe.data, add_offset) else {
                        break;
                    };
                    if add & 0x7f00_0000 == 0x1100_0000
                        && add & 0x8000_0000 != 0
                        && (add >> 5) & 0x1f == register
                    {
                        let mut immediate = i64::from((add >> 10) & 0xfff);
                        if (add >> 22) & 1 != 0 {
                            immediate <<= 12;
                        }
                        let target = page + immediate;
                        if key_rva <= target && target < key_rva_end {
                            references += 1;
                        }
                        break;
                    }
                    add_offset += 4;
                }
            }
            instruction += 4;
        }
    }
    references
}

fn find_key_blobs(pe: &PeImage) -> Vec<KeyBlob> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for bits in RSA_KEY_BITS {
        let marker = bits.to_be_bytes();
        for offset in pe
            .data
            .windows(marker.len())
            .enumerate()
            .filter_map(|(offset, window)| (window == marker).then_some(offset))
        {
            let Some(blob) = decode_key_candidate(&pe.data, offset, bits) else {
                continue;
            };
            if pe.raw_to_rva(offset).is_none() || !seen.insert(blob.clone()) {
                continue;
            }
            let references = instruction_references(pe, offset, blob.len());
            found.push(KeyBlob {
                raw_offset: offset,
                blob,
                references,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, io::Write, path::PathBuf};

    use lzma_rust2::{LzmaOptions, LzmaWriter};

    use super::{extract_abl_avb_pubkey_sha1, extract_from_bytes, lzma_decode};

    #[test]
    fn garbage_and_truncated_inputs_return_errors() {
        let garbage: Vec<u8> = (0..8192)
            .map(|index| ((index * 73 + 41) & 0xff) as u8)
            .collect();
        for input in [&[][..], b"MZ", &garbage[..], &garbage[..63]] {
            assert!(extract_from_bytes(input).is_err());
        }
    }

    #[test]
    fn lzma_alone_unknown_size_and_headerless_size_decode() {
        let payload: Vec<u8> = (0..256).map(|value| value as u8).collect();
        let mut writer =
            LzmaWriter::new_use_header(Vec::new(), &LzmaOptions::default(), None).unwrap();
        writer.write_all(&payload).unwrap();
        let encoded = writer.finish().unwrap();
        assert_eq!(&encoded[5..13], &[0xff; 8]);
        assert_eq!(lzma_decode(&encoded).as_deref(), Some(payload.as_slice()));

        let mut without_size = encoded[..5].to_vec();
        without_size.extend_from_slice(&encoded[13..]);
        assert_eq!(
            lzma_decode(&without_size).as_deref(),
            Some(payload.as_slice())
        );
    }

    #[test]
    #[ignore = "needs LTBOX_TEST_ABL_DIR=/path/to/abl/fixtures"]
    fn extracts_local_abl_fixtures() {
        let directory = PathBuf::from(
            std::env::var_os("LTBOX_TEST_ABL_DIR")
                .expect("LTBOX_TEST_ABL_DIR must point to the ABL fixture directory"),
        );
        let expected = HashMap::from([
            ("abl_tb320.elf", "2597c218aae470a130f61162feaae70afd97f011"),
            ("abl_tb321.elf", "2597c218aae470a130f61162feaae70afd97f011"),
            (
                "abl_tb322_new.elf",
                "8fcb864f11f53ed11284615fb67685522085d3a2",
            ),
            (
                "abl_tb322_old.elf",
                "2597c218aae470a130f61162feaae70afd97f011",
            ),
            ("abl_tb520.elf", "2597c218aae470a130f61162feaae70afd97f011"),
            ("abl_tb710.elf", "2597c218aae470a130f61162feaae70afd97f011"),
            (
                "abl_tb710_new.elf",
                "2597c218aae470a130f61162feaae70afd97f011",
            ),
        ]);

        let mut paths: Vec<_> = fs::read_dir(&directory)
            .expect("read LTBOX_TEST_ABL_DIR")
            .map(|entry| entry.expect("read fixture directory entry").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "elf"))
            .collect();
        paths.sort();
        assert!(
            !paths.is_empty(),
            "LTBOX_TEST_ABL_DIR contains no *.elf files"
        );

        let mut fingerprints = std::collections::HashSet::new();
        for path in paths {
            let fingerprint = extract_abl_avb_pubkey_sha1(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            println!(
                "{} {fingerprint}",
                path.file_name().unwrap().to_string_lossy()
            );
            fingerprints.insert(fingerprint.clone());
            if let Some(expected) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| expected.get(name))
            {
                assert_eq!(&fingerprint, expected, "{}", path.display());
            }
        }
        assert_eq!(fingerprints.len(), 2);
        assert!(fingerprints.contains("2597c218aae470a130f61162feaae70afd97f011"));
        assert!(fingerprints.contains("8fcb864f11f53ed11284615fb67685522085d3a2"));
    }
}
