//! PE/UEFI parsing shared by the ABL AVB-key extractor (`abl_key`) and the
//! efisp loader detector (`efisp_load`).
//!
//! The two callers decompress LZMA with different libraries and break
//! candidate ties differently; both differences are explicit parameters here.

use std::{collections::HashSet, ops::Range};

pub(super) const LZMA_OFFSET: usize = 0x1078;
pub(super) const MAX_DECOMPRESSED_SIZE: usize = 64 * 1024 * 1024;
pub(super) const MAX_INPUT_SIZE: usize = MAX_DECOMPRESSED_SIZE * 4;
/// Real ABLs contain only a handful of PE images; stop scanning past this.
pub(super) const MAX_PE_CANDIDATES: usize = 64;

#[derive(Clone, Debug)]
pub(super) struct Section {
    pub(super) name: String,
    pub(super) virtual_address: u32,
    pub(super) raw_offset: usize,
    pub(super) raw_size: usize,
    pub(super) executable: bool,
}

#[derive(Debug)]
pub(super) struct PeImage {
    pub(super) data: Vec<u8>,
    pub(super) sections: Vec<Section>,
}

impl PeImage {
    pub(super) fn raw_to_rva(&self, raw_offset: usize) -> Option<i64> {
        self.sections.iter().find_map(|section| {
            let end = section.raw_offset.checked_add(section.raw_size)?;
            (section.raw_offset <= raw_offset && raw_offset < end).then(|| {
                i64::from(section.virtual_address) + (raw_offset - section.raw_offset) as i64
            })
        })
    }

    pub(super) fn text_ranges(&self) -> Vec<(usize, usize, i64)> {
        let named: Vec<_> = self
            .sections
            .iter()
            .filter(|section| section.name == ".text")
            .collect();
        let executable: Vec<_> = self
            .sections
            .iter()
            .filter(|section| section.executable)
            .collect();
        let selected = if !named.is_empty() {
            named
        } else if !executable.is_empty() {
            executable
        } else {
            self.sections.iter().collect()
        };

        selected
            .into_iter()
            .filter_map(|section| {
                if section.raw_size == 0 || section.raw_offset >= self.data.len() {
                    return None;
                }
                let end = section
                    .raw_offset
                    .saturating_add(section.raw_size)
                    .min(self.data.len());
                Some((section.raw_offset, end, i64::from(section.virtual_address)))
            })
            .collect()
    }
}

/// A PE found inside a buffer: a byte range into that buffer plus its parsed
/// sections. It owns no image bytes; `PeImage` is materialised only for the
/// candidate that is actually used.
#[derive(Debug)]
pub(super) struct PeCandidate {
    pub(super) range: Range<usize>,
    pub(super) sections: Vec<Section>,
}

impl PeCandidate {
    fn len(&self) -> usize {
        self.range.len()
    }

    fn to_image(&self, data: &[u8]) -> Option<PeImage> {
        Some(PeImage {
            data: data.get(self.range.clone())?.to_vec(),
            sections: self.sections.clone(),
        })
    }
}

/// Decodes one complete LZMA-alone stream (already length-checked by
/// `lzma_decode`); each caller supplies its own decoder and validation.
pub(super) type LzmaStreamDecoder = fn(&[u8]) -> Option<Vec<u8>>;

/// Which of several equally large PE candidates wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TieBreak {
    /// The earliest candidate wins.
    First,
    /// The latest candidate wins.
    Last,
}

pub(super) fn read_u16_le(data: &[u8], offset: usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

pub(super) fn read_u32_le(data: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

pub(super) fn read_u64_le(data: &[u8], offset: usize) -> Option<u64> {
    let bytes: [u8; 8] = data.get(offset..offset.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

pub(super) fn sign_extend(value: u32, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((i64::from(value)) << shift) >> shift
}

/// Try the stream as-is, then with an unknown-size header spliced in, handing
/// each well-formed attempt to `decode`.
pub(super) fn lzma_decode(stream: &[u8], decode: LzmaStreamDecoder) -> Option<Vec<u8>> {
    if stream.is_empty() || stream.len() > MAX_INPUT_SIZE {
        return None;
    }

    let mut spliced = Vec::new();
    let mut attempts = vec![stream];
    if stream.len() >= 5 && stream.first() == Some(&0x5d) {
        spliced.reserve(stream.len().checked_add(8)?);
        spliced.extend_from_slice(stream.get(..5)?);
        spliced.extend_from_slice(&[0xff; 8]);
        spliced.extend_from_slice(stream.get(5..)?);
        attempts.push(&spliced);
    }

    for candidate in attempts {
        if candidate.len() < 13 {
            continue;
        }
        let declared_size = read_u64_le(candidate, 5)?;
        if declared_size != u64::MAX && declared_size > MAX_DECOMPRESSED_SIZE as u64 {
            continue;
        }

        if let Some(decoded) = decode(candidate) {
            return Some(decoded);
        }
    }
    None
}

/// One decompressed LZMA payload that contains at least one PE.
struct Layer {
    data: Vec<u8>,
    candidates: Vec<PeCandidate>,
}

fn decompressed_layers(raw: &[u8], decode: LzmaStreamDecoder) -> Vec<Layer> {
    let mut starts = Vec::new();
    if raw.len() > LZMA_OFFSET {
        starts.push(LZMA_OFFSET);
    }
    for (offset, window) in raw.windows(3).enumerate() {
        if window == [0x5d, 0, 0] {
            starts.push(offset);
        }
    }

    let mut seen = HashSet::new();
    starts
        .into_iter()
        .filter(|start| seen.insert(*start))
        .filter_map(|start| lzma_decode(raw.get(start..)?, decode))
        .filter_map(|data| {
            let candidates = find_pe_candidates(&data);
            (!candidates.is_empty()).then_some(Layer { data, candidates })
        })
        .collect()
}

/// Locate PE images in `data` without copying them, stopping after
/// `MAX_PE_CANDIDATES`.
pub(super) fn find_pe_candidates(data: &[u8]) -> Vec<PeCandidate> {
    let mut images = Vec::new();
    let mut cursor = 0usize;

    while cursor < data.len() && images.len() < MAX_PE_CANDIDATES {
        let Some(relative_mz) = data
            .get(cursor..)
            .and_then(|tail| tail.windows(2).position(|window| window == b"MZ"))
        else {
            break;
        };
        let Some(mz) = cursor.checked_add(relative_mz) else {
            break;
        };
        cursor = match mz.checked_add(2) {
            Some(next) => next,
            None => break,
        };

        let Some(dos_end) = mz.checked_add(0x40) else {
            continue;
        };
        if dos_end > data.len() {
            continue;
        }
        let Some(e_lfanew) = read_u32_le(data, mz + 0x3c).map(|value| value as usize) else {
            continue;
        };
        let Some(pe_header) = mz.checked_add(e_lfanew) else {
            continue;
        };
        let Some(pe_fixed_end) = pe_header.checked_add(24) else {
            continue;
        };
        if pe_header < mz || pe_fixed_end > data.len() {
            continue;
        }
        if data.get(pe_header..pe_header + 4) != Some(b"PE\0\0".as_slice()) {
            continue;
        }

        let Some(number_of_sections) = read_u16_le(data, pe_header + 6).map(usize::from) else {
            continue;
        };
        let Some(optional_size) = read_u16_le(data, pe_header + 20).map(usize::from) else {
            continue;
        };
        if !(1..=96).contains(&number_of_sections) || optional_size < 0x60 {
            continue;
        }
        let optional = pe_header + 24;
        let Some(optional_end) = optional.checked_add(optional_size) else {
            continue;
        };
        if optional_end > data.len() || !matches!(read_u16_le(data, optional), Some(0x10b | 0x20b))
        {
            continue;
        }
        let section_table = optional_end;
        let Some(table_size) = number_of_sections.checked_mul(40) else {
            continue;
        };
        let Some(table_end) = section_table.checked_add(table_size) else {
            continue;
        };
        if table_end > data.len() {
            continue;
        }

        let Some(size_of_headers) = read_u32_le(data, optional + 60).map(|value| value as usize)
        else {
            continue;
        };
        let mut real_size = size_of_headers.max(table_end - mz);
        let mut sections = Vec::with_capacity(number_of_sections);
        let mut valid = true;
        for index in 0..number_of_sections {
            let Some(section) = section_table.checked_add(index * 40) else {
                valid = false;
                break;
            };
            let Some(raw_size) = read_u32_le(data, section + 16).map(|value| value as usize) else {
                valid = false;
                break;
            };
            let Some(raw_offset) = read_u32_le(data, section + 20).map(|value| value as usize)
            else {
                valid = false;
                break;
            };
            let Some(virtual_address) = read_u32_le(data, section + 12) else {
                valid = false;
                break;
            };
            let Some(characteristics) = read_u32_le(data, section + 36) else {
                valid = false;
                break;
            };
            let Some(raw_end) = raw_offset.checked_add(raw_size) else {
                valid = false;
                break;
            };
            if raw_size != 0 && raw_end > data.len() - mz {
                valid = false;
                break;
            }
            real_size = real_size.max(raw_end);
            let Some(name_bytes) = data.get(section..section + 8) else {
                valid = false;
                break;
            };
            let name_end = name_bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(name_bytes.len());
            sections.push(Section {
                name: String::from_utf8_lossy(&name_bytes[..name_end]).into_owned(),
                virtual_address,
                raw_offset,
                raw_size,
                executable: characteristics & 0x2000_0000 != 0,
            });
        }
        let Some(image_end) = mz.checked_add(real_size) else {
            continue;
        };
        if !valid || real_size == 0 || image_end > data.len() {
            continue;
        }
        images.push(PeCandidate {
            range: mz..image_end,
            sections,
        });
    }
    images
}

/// Pick the largest PE from a raw MZ buffer, or from every decompressed layer
/// otherwise, and copy only that one. `tie` decides between equal sizes; the
/// callers differ here for parity with their original detectors.
pub(super) fn load_linuxloader(
    raw: &[u8],
    decode: LzmaStreamDecoder,
    tie: TieBreak,
) -> Option<PeImage> {
    fn consider<'a>(
        best: &mut Option<(&'a [u8], &'a PeCandidate)>,
        data: &'a [u8],
        candidates: &'a [PeCandidate],
        tie: TieBreak,
    ) {
        for candidate in candidates {
            let wins = best.is_none_or(|(_, current)| match tie {
                TieBreak::First => candidate.len() > current.len(),
                TieBreak::Last => candidate.len() >= current.len(),
            });
            if wins {
                *best = Some((data, candidate));
            }
        }
    }

    let mut best = None;
    let direct;
    let layers;
    if raw.starts_with(b"MZ") {
        direct = find_pe_candidates(raw);
        consider(&mut best, raw, &direct, tie);
    } else {
        layers = decompressed_layers(raw, decode);
        for layer in &layers {
            consider(&mut best, &layer.data, &layer.candidates, tie);
        }
    }
    best.and_then(|(data, candidate)| candidate.to_image(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: usize = 0x100;

    fn put32(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// A minimal valid PE header block whose declared headers size is
    /// `size_of_headers`, so each candidate spans far past its own block.
    fn block(size_of_headers: u32) -> [u8; BLOCK] {
        let mut b = [0u8; BLOCK];
        b[..2].copy_from_slice(b"MZ");
        put32(&mut b, 0x3c, 0x40);
        b[0x40..0x44].copy_from_slice(b"PE\0\0");
        b[0x46] = 1; // one section
        b[0x54] = 0x60; // optional header size
        b[0x58..0x5a].copy_from_slice(&0x20bu16.to_le_bytes());
        put32(&mut b, 0x58 + 60, size_of_headers);
        put32(&mut b, 0xb8 + 16, 0x10); // section raw size
        b
    }

    fn no_decoder(_: &[u8]) -> Option<Vec<u8>> {
        None
    }

    #[test]
    fn candidate_cap_and_no_copy_amplification() {
        const BLOCKS: usize = 4096; // 1 MiB of overlapping candidates
        let mut data = Vec::with_capacity(BLOCKS * BLOCK);
        for _ in 0..BLOCKS {
            data.extend_from_slice(&block(0x1_0000));
        }

        let candidates = find_pe_candidates(&data);
        assert_eq!(candidates.len(), MAX_PE_CANDIDATES);
        // Each candidate spans 64 KiB, so copying them would exceed the input.
        let spanned: usize = candidates.iter().map(PeCandidate::len).sum();
        assert!(spanned > data.len());
        // Candidates are ranges plus section metadata; no image bytes held.
        for candidate in &candidates {
            assert!(candidate.range.end <= data.len());
            assert_eq!(candidate.sections.len(), 1);
        }

        let image = load_linuxloader(&data, no_decoder, TieBreak::First).unwrap();
        assert_eq!(image.data.len(), 0x1_0000);
        assert_eq!(&image.data[..2], b"MZ");
    }

    #[test]
    fn tie_break_selects_first_or_last_equal_candidate() {
        let mut data = Vec::new();
        for marker in [1u8, 2, 3] {
            let mut b = block(BLOCK as u32);
            b[0xff] = marker;
            data.extend_from_slice(&b);
        }
        let first = load_linuxloader(&data, no_decoder, TieBreak::First).unwrap();
        let last = load_linuxloader(&data, no_decoder, TieBreak::Last).unwrap();
        assert_eq!(first.data[0xff], 1);
        assert_eq!(last.data[0xff], 3);
    }
}
