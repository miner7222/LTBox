//! Static efisp loader detection, ported from check_efisp_load.py.
use liblzma::stream::{Action, Status, Stream};

use crate::abl_pe::{self, MAX_DECOMPRESSED_SIZE, PeImage, TieBreak, read_u32_le, sign_extend};

const LOAD: &[u8] = b"Loading GBL app";
const START: &[u8] = b"Starting GBL app";
const EFISP: &[u8] = b"e\0f\0i\0s\0p\0";
const SECURE: &[u8] = b"S\0e\0c\0u\0r\0e\0B\0o\0o\0t\0";

/// Whether the reference detector finds an ungated efisp execution chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EfispLoad {
    Yes,
    No,
    #[default]
    Undetermined,
}

/// Inspect a raw PE or an ABL containing an LZMA-compressed PE.
/// Incomplete chains, authentication gates and unusable inputs are undetermined.
#[must_use]
pub fn detect(data: &[u8]) -> EfispLoad {
    load_linuxloader(data).map_or(EfispLoad::Undetermined, |pe| check(&pe))
}
#[cfg(test)]
fn lzma_decode(stream: &[u8]) -> Option<Vec<u8>> {
    abl_pe::lzma_decode(stream, decode_complete_stream)
}

/// Use the same decoder family as Python. A declared output length alone is
/// insufficient: malformed range-coder endings must not yield a usable ABL.
fn decode_complete_stream(candidate: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = Stream::new_lzma_decoder(MAX_DECOMPRESSED_SIZE as u64).ok()?;
    let mut decoded = Vec::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        let before = (decoder.total_in(), decoder.total_out());
        let input_offset = usize::try_from(before.0).ok()?;
        let remaining = (MAX_DECOMPRESSED_SIZE + 1).checked_sub(decoded.len())?;
        if remaining == 0 {
            return None;
        }
        let limit = remaining.min(buffer.len());
        let status = decoder
            .process(
                candidate.get(input_offset..)?,
                &mut buffer[..limit],
                Action::Run,
            )
            .ok()?;
        let written = usize::try_from(decoder.total_out() - before.1).ok()?;
        decoded.extend_from_slice(&buffer[..written]);
        if status == Status::StreamEnd {
            return (64..=MAX_DECOMPRESSED_SIZE)
                .contains(&decoded.len())
                .then_some(decoded);
        }
        if before == (decoder.total_in(), decoder.total_out()) {
            return None;
        }
    }
}

fn load_linuxloader(raw: &[u8]) -> Option<PeImage> {
    // The first of several equally large PEs wins, unlike `abl_key`.
    abl_pe::load_linuxloader(raw, decode_complete_stream, TieBreak::First)
}

fn string_rvas(pe: &PeImage, needle: &[u8]) -> Vec<i64> {
    pe.data
        .windows(needle.len())
        .enumerate()
        .filter(|(_, bytes)| *bytes == needle)
        .filter_map(|(raw, _)| pe.raw_to_rva(raw))
        .collect()
}

fn code_xrefs_to(pe: &PeImage, target_rva: i64, margin: i64) -> Option<Vec<i64>> {
    let mut hits = Vec::new();
    for (raw_start, raw_end, section_rva) in pe.text_ranges() {
        let first = raw_start + ((4 - (raw_start & 3)) & 3);
        for instruction in (first..raw_end.saturating_sub(3)).step_by(4) {
            let word = read_u32_le(&pe.data, instruction)?;
            let pc = section_rva + (instruction - raw_start) as i64;
            let immediate = sign_extend(((word >> 5) & 0x7ffff) << 2 | ((word >> 29) & 3), 21);
            let mut instr_rva = pc;
            let target = match word & 0x9f00_0000 {
                0x1000_0000 => Some(pc + immediate),
                0x9000_0000 => {
                    let page = (pc & !0xfff) + (immediate << 12);
                    let mut target = None;
                    for add_offset in
                        ((instruction + 4)..(instruction + 20).min(raw_end)).step_by(4)
                    {
                        let add = read_u32_le(&pe.data, add_offset)?;
                        if add & 0x7f00_0000 != 0x1100_0000
                            || add & 0x8000_0000 == 0
                            || (add >> 5) & 0x1f != word & 0x1f
                        {
                            continue;
                        }
                        let immediate = i64::from((add >> 10) & 0xfff)
                            << if (add >> 22) & 1 != 0 { 12 } else { 0 };
                        target = Some(page + immediate);
                        instr_rva = section_rva + (add_offset - raw_start) as i64;
                        break;
                    }
                    target
                }
                _ => None,
            };
            if target.is_some_and(|target| target_rva <= target && target < target_rva + margin) {
                hits.push(instr_rva);
            }
        }
    }
    hits.sort_unstable();
    hits.dedup();
    Some(hits)
}

fn xrefs(pe: &PeImage, rvas: &[i64], margin: i64) -> Option<Vec<i64>> {
    let mut hits = Vec::new();
    for rva in rvas {
        hits.extend(code_xrefs_to(pe, *rva, margin)?);
    }
    hits.sort_unstable();
    hits.dedup();
    Some(hits)
}

fn check(pe: &PeImage) -> EfispLoad {
    check_inner(pe).unwrap_or_default()
}

fn check_inner(pe: &PeImage) -> Option<EfispLoad> {
    let loads = string_rvas(pe, LOAD);
    let starts = string_rvas(pe, START);
    let efisp_x = xrefs(pe, &string_rvas(pe, EFISP), 64)?;
    let _prefix_x = xrefs(pe, &string_rvas(pe, b"EFISP:"), 64)?;
    if loads.is_empty() && starts.is_empty() && efisp_x.is_empty() {
        return Some(EfispLoad::No);
    }
    let load_x = xrefs(pe, &loads, LOAD.len() as i64)?;
    let start_x = xrefs(pe, &starts, START.len() as i64)?;
    // Preserve the first ordered chain: later ungated chains do not override it.
    let chain = efisp_x.iter().find_map(|e1| {
        load_x.iter().filter(|e2| e1 < *e2).find_map(|e2| {
            start_x
                .iter()
                .find(|e3| e2 < *e3 && **e3 - e1 <= 0x4000)
                .map(|e3| (*e1, *e3))
        })
    });
    let Some((first, last)) = chain else {
        return Some(EfispLoad::Undetermined);
    };
    let secure = string_rvas(pe, SECURE)
        .into_iter()
        .chain(string_rvas(pe, b"SecureBoot"))
        .collect::<Vec<_>>();
    if xrefs(pe, &secure, 64)?
        .iter()
        .any(|pc| first - 0x200 <= *pc && *pc <= last + 0x400)
    {
        Some(EfispLoad::Undetermined)
    } else {
        Some(EfispLoad::Yes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abl_pe::LZMA_OFFSET;
    use lzma_rust2::{LzmaOptions, LzmaWriter};
    use std::io::Write;

    fn put(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture(chain: bool) -> Vec<u8> {
        let mut data = vec![0; 0x6000];
        data[..2].copy_from_slice(b"MZ");
        put(&mut data, 0x3c, 0x80);
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        data[0x86] = 1;
        data[0x94] = 0x60;
        data[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
        put(&mut data, 0xd4, 0x200);
        data[0xf8..0xfd].copy_from_slice(b".text");
        put(&mut data, 0x104, 0x1000);
        put(&mut data, 0x108, 0x5e00);
        put(&mut data, 0x10c, 0x200);
        put(&mut data, 0x11c, 0x2000_0000);
        if chain {
            for (raw, mark) in [(0x5000, EFISP), (0x5100, LOAD), (0x5200, START)] {
                data[raw..raw + mark.len()].copy_from_slice(mark);
            }
            adr(&mut data, 0x400, 0x5000);
            adr(&mut data, 0x420, 0x5100);
            adr(&mut data, 0x440, 0x5200);
        }
        data
    }
    fn adr(data: &mut [u8], at: usize, target: usize) {
        let imm = (target as i64 - at as i64) as u32 & 0x1f_ffff;
        put(data, at, 0x1000_0000 | (imm & 3) << 29 | (imm >> 2) << 5);
    }
    fn compress(data: &[u8], size: Option<u64>) -> Vec<u8> {
        let mut writer =
            LzmaWriter::new_use_header(Vec::new(), &LzmaOptions::default(), size).unwrap();
        writer.write_all(data).unwrap();
        writer.finish().unwrap()
    }
    #[test]
    fn rejects_invalid_known_size_range_coder_end() {
        // A relaxed decoder produces the declared 128 bytes; Python/liblzma
        // rejects the damaged ending instead of treating that length as EOF.
        let stream = [
            0x5d, 0, 0, 0x80, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x6e, 0xd8, 0x44, 0x98, 0x01,
        ];
        assert!(lzma_decode(&stream).is_none());
    }

    #[test]
    fn tri_state_and_malformed_pe() {
        assert_eq!(detect(&fixture(true)), EfispLoad::Yes);
        assert_eq!(detect(&fixture(false)), EfispLoad::No);
        assert_eq!(detect(b"garbage"), EfispLoad::Undetermined);
        let mut data = fixture(true);
        put(&mut data, 0x440, 0);
        assert_eq!(detect(&data), EfispLoad::Undetermined);
        put(&mut data, 0x108, u32::MAX);
        assert_eq!(detect(&data), EfispLoad::Undetermined);
    }
    #[test]
    fn secure_gate_inclusive_boundary() {
        let mut data = fixture(true);
        data[0x5300..0x530a].copy_from_slice(b"SecureBoot");
        adr(&mut data, 0x840, 0x5300);
        assert_eq!(detect(&data), EfispLoad::Undetermined);
        put(&mut data, 0x840, 0);
        adr(&mut data, 0x844, 0x5300);
        assert_eq!(detect(&data), EfispLoad::Yes);
    }
    #[test]
    fn ordered_chain_span_and_marker_margin() {
        let mut data = fixture(true);
        adr(&mut data, 0x420, 0x5100 + LOAD.len());
        assert_eq!(detect(&data), EfispLoad::Undetermined);
        adr(&mut data, 0x420, 0x5100 + LOAD.len() - 1);
        assert_eq!(detect(&data), EfispLoad::Yes);
        put(&mut data, 0x440, 0);
        adr(&mut data, 0x4400, 0x5200);
        assert_eq!(detect(&data), EfispLoad::Yes);
        put(&mut data, 0x4400, 0);
        adr(&mut data, 0x4404, 0x5200);
        assert_eq!(detect(&data), EfispLoad::Undetermined);
    }
    #[test]
    fn adrp_first_matching_add_and_window() {
        let mut data = fixture(false);
        // PC 0x1200, page + 0x4000; target 0x5e00 (raw 0x5000).
        put(&mut data, 0x400, 0x9000_0023);
        put(&mut data, 0x410, 0x9100_0000 | (0xe00 << 10) | (3 << 5));
        let pe = load_linuxloader(&data).unwrap();
        assert_eq!(code_xrefs_to(&pe, 0x5e00, 1).unwrap(), vec![0x1210]);
        put(&mut data, 0x410, 0);
        put(&mut data, 0x414, 0x9100_0000 | (0xe00 << 10) | (3 << 5));
        assert!(
            code_xrefs_to(&load_linuxloader(&data).unwrap(), 0x5e00, 1)
                .unwrap()
                .is_empty()
        );
        put(&mut data, 0x404, 0x9100_0000 | (3 << 5));
        put(&mut data, 0x408, 0x9100_0000 | (0xe00 << 10) | (3 << 5));
        assert!(
            code_xrefs_to(&load_linuxloader(&data).unwrap(), 0x5e00, 1)
                .unwrap()
                .is_empty()
        );
        adr(&mut data, 0x5400, 0x5000);
        assert!(
            code_xrefs_to(&load_linuxloader(&data).unwrap(), 0x5e00, 1)
                .unwrap()
                .contains(&0x6200)
        );
    }
    #[test]
    fn compressed_wrappers_and_truncation() {
        let data = fixture(true);
        for size in [None, Some(data.len() as u64)] {
            let compressed = compress(&data, size);
            let mut wrapper = vec![0; LZMA_OFFSET];
            wrapper.extend_from_slice(&compressed);
            assert_eq!(detect(&wrapper), EfispLoad::Yes);
            wrapper.truncate(LZMA_OFFSET + compressed.len() / 2);
            assert_eq!(detect(&wrapper), EfispLoad::Undetermined);
        }
        let compressed = compress(&data, None);
        let mut repaired = vec![0; 37];
        repaired.extend_from_slice(&compressed[..5]);
        repaired.extend_from_slice(&compressed[13..]);
        assert_eq!(detect(&repaired), EfispLoad::Yes);
        let mut over_memory = compressed;
        over_memory[1..5].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(lzma_decode(&over_memory).is_none());
    }
    #[test]
    fn truncated_adrp_lookahead_is_undetermined() {
        let mut data = fixture(true);
        data.truncate(0x5ffd);
        put(&mut data, 0x108, 0x5dfd);
        put(&mut data, 0x5ff8, 0x9000_0000);
        assert_eq!(detect(&data), EfispLoad::Undetermined);
    }

    #[test]
    fn section_selection_and_unreferenced_partition_name() {
        let mut data = fixture(true);
        // Removing the name exercises executable fallback, then all-section fallback.
        data[0xf8..0x100].fill(0);
        assert_eq!(detect(&data), EfispLoad::Yes);
        put(&mut data, 0x11c, 0);
        assert_eq!(detect(&data), EfispLoad::Yes);
        let mut data = fixture(false);
        data[0x5000..0x5000 + EFISP.len()].copy_from_slice(EFISP);
        data[0x5100..0x5106].copy_from_slice(b"EFISP:");
        assert_eq!(detect(&data), EfispLoad::No);
        adr(&mut data, 0x400, 0x503f);
        assert_eq!(detect(&data), EfispLoad::Undetermined);
        adr(&mut data, 0x400, 0x5040);
        assert_eq!(detect(&data), EfispLoad::No);
    }
    #[test]
    fn largest_pe_and_first_tie_win() {
        let mut raw = fixture(false);
        raw.extend_from_slice(&fixture(true));
        assert_eq!(detect(&raw), EfispLoad::No);
        let mut wrapper = vec![0; 20];
        wrapper.extend(compress(&raw, None));
        assert_eq!(detect(&wrapper), EfispLoad::No);
        // Embedded uncompressed PE is ignored unless the whole input starts MZ.
        let mut prefixed = vec![0; 20];
        prefixed.extend(fixture(true));
        assert_eq!(detect(&prefixed), EfispLoad::Undetermined);
    }
}
