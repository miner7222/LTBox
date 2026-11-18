//! Source-free kallsyms decoder.
//!
//! Reconstructs the kernel's symbol table from the in-image kallsyms blob alone
//! (no `System.map`, `/proc/kallsyms` or kernel source). Unified port of
//! upstream's seven near-duplicate `kallsyms_lookup_name_*` classes; their
//! layout differences are captured in [`Profile`] and selected by kernel version.
//!
//! Only the `CONFIG_KALLSYMS_BASE_RELATIVE` format is implemented (all
//! AArch64 Android kernels in scope use it), not the absolute
//! `kallsyms_addresses` format of the oldest kernels.
//!
//! The table-location heuristic ([`Decoder::scan_offsets_list`], as in upstream)
//! assumes `kallsyms_offsets` is a monotonically increasing unsigned table, as
//! on the validated kernels. [`Decoder::sym_address`] does implement the signed
//! `ABSOLUTE_PERCPU` rule (negative entries relative to `relative_base - 1`),
//! but a kernel with many negative entries could truncate the scanned run. The
//! whole-image `kallsyms_num_syms` fallback usually recovers the count; that
//! layout is not covered by the integration tests.
//!
//! Layout walked (relative format), in file order:
//! `kallsyms_offsets` → `kallsyms_relative_base` (6.1.42+) →
//! `kallsyms_seqs_of_names` (6.12+, before num) → `kallsyms_num_syms` →
//! `kallsyms_names` → `kallsyms_markers` → `kallsyms_seqs_of_names`
//! (6.1.42..6.12) → `kallsyms_token_table` → `kallsyms_token_index`.
#![allow(dead_code)]

use std::collections::HashMap;

use super::version::KernelVersion;

/// Decoded kernel symbol table: every symbol mapped to its **file offset**
/// inside the kernel image (not its virtual address).
#[derive(Debug, Clone, Default)]
pub struct Symbols {
    by_name: HashMap<String, u64>,
    /// Symbols in kallsyms order: `(name, file_offset)`.
    order: Vec<(String, u64)>,
    /// Sorted unique file offsets, for next-symbol size queries.
    sorted: Vec<u64>,
}

impl Symbols {
    /// File offset of `name`, or `None` if absent.
    pub fn lookup(&self, name: &str) -> Option<u64> {
        self.by_name.get(name).copied()
    }

    /// Distance to the next symbol after `addr` — an upper bound on the symbol
    /// size (upstream `kallsyms_symbol_size`). `None` if `addr` is the last.
    pub fn size_at(&self, addr: u64) -> Option<u64> {
        if addr == 0 {
            return None;
        }
        // Smallest recorded offset strictly greater than `addr`.
        let idx = self.sorted.partition_point(|&o| o <= addr);
        self.sorted.get(idx).map(|&next| next - addr)
    }

    /// Every symbol whose name contains `needle` (upstream
    /// `kallsyms_lookup_names_like`), excluding CFI jump-table aliases.
    pub fn names_like(&self, needle: &str) -> Vec<(&str, u64)> {
        self.order
            .iter()
            .filter(|(n, _)| n.contains(needle) && !n.ends_with(".cfi_jt"))
            .map(|(n, a)| (n.as_str(), *a))
            .collect()
    }

    /// All symbols in kallsyms order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, u64)> {
        self.order.iter().map(|(n, a)| (n.as_str(), *a))
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// Why kallsyms decoding failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KallsymsError {
    NoKernelVersion,
    NoOffsetsList,
    NoRelativeBase,
    NoSymbolCount,
    NoNamesList,
    NoMarkersList,
    NoTokenTable,
    NoTokenIndex,
    NoSymbolBase,
}

/// Where `kallsyms_num_syms` is scanned for, relative to known structures.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NumFrom {
    OffsetsEnd,
    RelBaseEnd,
    Zero,
}

/// Where `kallsyms_seqs_of_names` sits in the layout (6.1.42+ only).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seqs {
    None,
    /// Between `kallsyms_markers` and `kallsyms_token_table` (6.1.42..6.12).
    AfterMarkers,
    /// Between `kallsyms_relative_base` and `kallsyms_num_syms` (6.12+).
    AfterRelBase,
}

/// Per-kernel-version layout knobs; the whole delta between upstream's seven
/// decoder classes.
struct Profile {
    /// Reject a compressed name length `>=` this (`KSYM_NAME_LEN`).
    ksym_name_len: u32,
    /// `kallsyms_names` lengths use a 2-byte varint when the MSB is set.
    names_varint: bool,
    /// Upper bound on the average bytes/symbol of `kallsyms_names`.
    entropy_max_avg: f64,
    /// `kallsyms_relative_base` (a `u64`) follows the offsets list.
    has_relative_base: bool,
    /// Negative offsets resolve via `relative_base - 1 - off` (else dropped).
    negative_uses_base: bool,
    seqs: Seqs,
    num_from: NumFrom,
}

impl Profile {
    fn for_version(v: (u32, u32, u32)) -> Profile {
        // Mirrors KernelSymbolParser::init_kallsyms_lookup_name dispatch.
        if v < (6, 1, 0) {
            // 4_6_0 decoder (also covers the 4.4..4.6 range in practice).
            Profile {
                ksym_name_len: 128,
                names_varint: false,
                entropy_max_avg: 40.0,
                has_relative_base: false,
                negative_uses_base: false,
                seqs: Seqs::None,
                num_from: NumFrom::OffsetsEnd,
            }
        } else if v < (6, 1, 42) {
            // 6_1_0 decoder.
            Profile {
                ksym_name_len: 512,
                names_varint: true,
                entropy_max_avg: 80.0,
                has_relative_base: false,
                negative_uses_base: false,
                seqs: Seqs::None,
                num_from: NumFrom::OffsetsEnd,
            }
        } else if v < (6, 12, 0) {
            // 6_1_42 / 6_1_60 / 6_4_0 decoders (byte-identical upstream).
            Profile {
                ksym_name_len: 512,
                names_varint: true,
                entropy_max_avg: 80.0,
                has_relative_base: true,
                negative_uses_base: true,
                seqs: Seqs::AfterMarkers,
                num_from: NumFrom::RelBaseEnd,
            }
        } else {
            // 6_12_0 decoder.
            Profile {
                ksym_name_len: 512,
                names_varint: true,
                entropy_max_avg: 80.0,
                has_relative_base: true,
                negative_uses_base: true,
                seqs: Seqs::AfterRelBase,
                num_from: NumFrom::Zero,
            }
        }
    }
}

/// Decode the kallsyms table of a kernel image.
pub fn analyze(kernel: &[u8]) -> Result<Symbols, KallsymsError> {
    let ver = KernelVersion::from_kernel(kernel).ok_or(KallsymsError::NoKernelVersion)?;
    analyze_with_version(kernel, ver.triple())
}

/// Decode using an explicit version triple (lets callers reuse an already
/// parsed [`KernelVersion`]).
pub fn analyze_with_version(
    kernel: &[u8],
    version: (u32, u32, u32),
) -> Result<Symbols, KallsymsError> {
    Decoder {
        buf: kernel,
        profile: Profile::for_version(version),
    }
    .run()
}

fn rd16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn rd32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
fn rd64(b: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(a)
}
fn align_up8(v: usize) -> usize {
    (v + 7) & !7
}

/// AArch64 kernel-VA recognizer across the common `VA_BITS` values.
fn looks_kernel_va(v: u64) -> bool {
    const STARTS: [u64; 4] = [
        0xFFFF_FFC0_0000_0000, // VA_BITS=39
        0xFFFF_FE00_0000_0000, // VA_BITS=42
        0xFFFF_8000_0000_0000, // VA_BITS=48
        0xFFF8_0000_0000_0000, // VA_BITS=52
    ];
    STARTS.into_iter().any(|s| (v & s) == s)
}

const A64_NOP: u32 = 0xD503_201F;
const MAX_FIND_RANGE: usize = 0x1000;

/// `aarch64_insn_is_b` — used by [`find_static_code_start`].
fn insn_is_b(w: u32) -> bool {
    (w & 0xFC00_0000) == 0x1400_0000
}

/// Find where real text begins (after the image header / exception vectors):
/// the first run of genuine, non-NOP, non-all-`b` instructions.
fn find_static_code_start(buf: &[u8]) -> usize {
    if buf.len() < 0x200 {
        return 0;
    }
    let mut x = 0x200;
    while x + 4 <= buf.len() {
        let w = rd32(buf, x);
        if w == 0 || w == A64_NOP {
            x += 4;
            continue;
        }
        if is_really_work(buf, x) {
            return x;
        }
        x += 4;
    }
    0
}

fn is_really_empty(buf: &[u8], start: usize) -> bool {
    let must = 12;
    let mut i = start;
    while i + 4 <= buf.len() && i < start + must * 4 {
        let w = rd32(buf, i);
        if w == 0 {
            return true;
        }
        if w != A64_NOP {
            return false;
        }
        i += 4;
    }
    true
}

fn is_really_all_b(buf: &[u8], start: usize) -> bool {
    let must = 80;
    let mut i = start;
    while i < start + must * 4 {
        if i + 4 > buf.len() {
            return false;
        }
        let w = rd32(buf, i);
        if w == 0 || w == A64_NOP || !insn_is_b(w) {
            return false;
        }
        i += 4;
    }
    true
}

fn is_really_work(buf: &[u8], start: usize) -> bool {
    let must = 80;
    if is_really_all_b(buf, start) {
        return false;
    }
    let mut i = start;
    while i + 4 <= buf.len() && i < start + must * 4 {
        let w = rd32(buf, i);
        if w == 0 {
            return false;
        }
        if w == A64_NOP && is_really_empty(buf, i) {
            return false;
        }
        i += 4;
    }
    true
}

struct Decoder<'a> {
    buf: &'a [u8],
    profile: Profile,
}

impl Decoder<'_> {
    fn run(&self) -> Result<Symbols, KallsymsError> {
        let buf = self.buf;
        let code_static_start = find_static_code_start(buf);

        // kallsyms_offsets list (monotonic u32 deltas, first entry 0).
        let (mut off_start, off_end) = self
            .find_offsets_list()
            .ok_or(KallsymsError::NoOffsetsList)?;

        // kallsyms_relative_base (6.1.42+).
        let mut relative_base = 0u64;
        let mut rel_base_end = 0usize;
        if self.profile.has_relative_base {
            let rb = self
                .find_relative_base_offset(off_end)
                .ok_or(KallsymsError::NoRelativeBase)?;
            relative_base = rd64(buf, rb);
            rel_base_end = rb + 8;
        }

        // kallsyms_num_syms.
        let min_cnt = ((off_end - off_start) / 4) as i64;
        let num_search_start = match self.profile.num_from {
            NumFrom::OffsetsEnd => off_end,
            NumFrom::RelBaseEnd => rel_base_end,
            NumFrom::Zero => 0,
        };
        // `kallsyms_num_syms` precedes `kallsyms_names`, and its position relative
        // to the offsets list varies (Android backports put num/names before the
        // offsets). The version-directed start is a fast path; on a miss scan the
        // whole image. The names entropy check makes a false match very unlikely.
        let (num, num_offset) = self
            .find_num(num_search_start, min_cnt - 10, min_cnt + 20)
            .or_else(|| {
                if num_search_start != 0 {
                    self.find_num(0, min_cnt - 10, min_cnt + 20)
                } else {
                    None
                }
            })
            .ok_or(KallsymsError::NoSymbolCount)?;
        let _ = rel_base_end;

        // Re-align the offsets list to `num` entries ending at off_end, backing up
        // while the first entry is non-zero (mirrors upstream).
        off_start = off_end - (num as usize) * 4;
        while off_start >= 4 && rd32(buf, off_start) != 0 {
            off_start -= 4;
        }
        let offsets_offset = off_start;

        // kallsyms_names.
        let (names_start, names_end) = self
            .find_names(num, num_offset + 4)
            .ok_or(KallsymsError::NoNamesList)?;

        // kallsyms_markers.
        let (_markers_start, markers_end) = self
            .find_markers(num, names_end)
            .ok_or(KallsymsError::NoMarkersList)?;

        // kallsyms_token_table + kallsyms_token_index. `kallsyms_seqs_of_names`
        // (6.1.42+) may sit between markers and the token table or trail the
        // block, depending on the build, so scan forward for the first candidate
        // whose token index decodes the leading symbols into clean names.
        let tables = self
            .find_token_tables(names_start, markers_end, num)
            .ok_or(KallsymsError::NoTokenTable)?;

        // Decode names + raw symbol addresses in kallsyms order.
        let raw = self.decode_symbols(num, offsets_offset, relative_base, &tables);

        // Resolve the VA→file-offset base from _stext (upstream
        // resolve_kallsyms_offset_symbol_base), then materialize offsets.
        let stext_raw = raw
            .iter()
            .find(|(n, _)| n == "_stext")
            .map(|(_, a)| *a)
            .ok_or(KallsymsError::NoSymbolBase)?;
        // base maps _stext's VA onto its file position (code_static_start); other
        // symbols follow by VA delta. i64 here; upstream's i32 `base_off` differs
        // only above a 4 GiB image span.
        let base = (code_static_start as i64).wrapping_sub(stext_raw as i64);

        let mut by_name = HashMap::with_capacity(raw.len());
        let mut order = Vec::with_capacity(raw.len());
        let mut sorted = Vec::with_capacity(raw.len());
        for (name, raw_addr) in raw {
            let file_off = raw_addr.wrapping_add(base as u64);
            by_name.entry(name.clone()).or_insert(file_off);
            order.push((name, file_off));
            sorted.push(file_off);
        }
        sorted.sort_unstable();
        sorted.dedup();

        Ok(Symbols {
            by_name,
            order,
            sorted,
        })
    }

    /// Scan for the `kallsyms_offsets` array: a long run of `u32` starting at 0
    /// and non-decreasing, with per-step deltas under 16 MiB.
    fn find_offsets_list(&self) -> Option<(usize, usize)> {
        let mut max_cnt = 60000usize;
        while max_cnt > 5000 {
            if let Some(r) = self.scan_offsets_list(max_cnt) {
                return Some(r);
            }
            max_cnt -= 5000;
        }
        None
    }

    fn scan_offsets_list(&self, max_cnt: usize) -> Option<(usize, usize)> {
        let buf = self.buf;
        let n = buf.len();
        let mut x = 0usize;
        while x + 4 < n {
            let val1 = rd32(buf, x);
            let val2 = rd32(buf, x + 4);
            if val1 != 0 || val1 >= val2 {
                x += 4;
                continue;
            }
            let mut cnt = 0usize;
            let mut j = x + 4;
            while j + 4 < n {
                let a = rd32(buf, j);
                let b = rd32(buf, j + 4);
                if a > b || b == 0 || b.wrapping_sub(a) > 0x0100_0000 {
                    j += 4;
                    break;
                }
                cnt += 1;
                j += 4;
            }
            if cnt >= max_cnt {
                return Some((x, j));
            }
            x += 4;
        }
        None
    }

    /// `kallsyms_relative_base`: the first `u64` within five 4-byte steps after
    /// the offsets list that looks like a kernel VA.
    fn find_relative_base_offset(&self, offsets_end: usize) -> Option<usize> {
        for i in 0..5 {
            let off = offsets_end + 4 * i;
            if off + 8 > self.buf.len() {
                break;
            }
            if looks_kernel_va(rd64(self.buf, off)) {
                return Some(off);
            }
        }
        None
    }

    /// `kallsyms_num_syms`: an `int` in `[min,max]` immediately preceding a
    /// valid names list.
    fn find_num(&self, start: usize, min: i64, max: i64) -> Option<(u32, usize)> {
        let buf = self.buf;
        if buf.len() < 8 {
            return None;
        }
        let mut x = start;
        while x < buf.len() - 8 {
            let test = rd32(buf, x) as i32 as i64;
            if test >= min && test <= max && self.find_names(test as u32, x + 4).is_some() {
                return Some((test as u32, x));
            }
            x += 4;
        }
        None
    }

    /// `kallsyms_names`: `num` length-prefixed compressed entries. Returns the
    /// `[start, end)` of the blob, gated by an entropy sanity check.
    fn find_names(&self, num: u32, num_end_offset: usize) -> Option<(usize, usize)> {
        let buf = self.buf;
        let limit = buf.len().min(num_end_offset + MAX_FIND_RANGE);
        // Skip the zero padding after num to the first non-zero byte.
        let mut x = num_end_offset;
        let mut start = 0usize;
        while x + 1 < limit {
            if buf[x] != 0 {
                start = x;
                break;
            }
            x += 1;
        }
        if start == 0 {
            return None;
        }
        let mut off = start;
        let mut parsed = 0u32;
        for _ in 0..num {
            if off >= buf.len() {
                return None;
            }
            let ch = buf[off] as u32;
            off += 1;
            let sym_len = if self.profile.names_varint {
                if ch <= 0x7F {
                    ch
                } else {
                    if off >= buf.len() {
                        return None;
                    }
                    let ch2 = buf[off] as u32;
                    off += 1;
                    (ch & 0x7F) | (ch2 << 7)
                }
            } else {
                if ch >= self.profile.ksym_name_len {
                    return None;
                }
                ch
            };
            if self.profile.names_varint && sym_len == 0 {
                break;
            }
            if sym_len >= self.profile.ksym_name_len {
                return None;
            }
            off += sym_len as usize;
            parsed += 1;
        }
        if parsed == 0 {
            return None;
        }
        if !self.names_entropy_ok(start, off, parsed) {
            return None;
        }
        Some((start, off))
    }

    /// Reject false-positive name blobs: a real compressed stream has a wide
    /// byte alphabet, few zeros, and a plausible average symbol length.
    fn names_entropy_ok(&self, start: usize, end: usize, num: u32) -> bool {
        let total = (end - start) as f64;
        let num = num as f64;
        if total < num * 1.5 || total > num * self.profile.entropy_max_avg {
            return false;
        }
        let mut seen = [false; 256];
        let mut unique = 0u32;
        let mut zeros = 0u32;
        for &b in &self.buf[start..end] {
            if b == 0 {
                zeros += 1;
            }
            if !seen[b as usize] {
                seen[b as usize] = true;
                unique += 1;
            }
        }
        if unique < 128 {
            return false;
        }
        (zeros as f64 / total) <= 0.15
    }

    /// `kallsyms_markers`: a `u32` table, one entry per 256 symbols. Detects the
    /// 8-byte-aligned variant where each marker is followed by a zero pad word.
    fn find_markers(&self, num: u32, names_end: usize) -> Option<(usize, usize)> {
        let buf = self.buf;
        let n = buf.len();
        let start = align_up8(names_end);
        let mut markers_start = 0usize;
        let mut x = start;
        while x + 4 < n {
            let val1 = rd32(buf, x);
            let val2 = rd32(buf, x + 4);
            if val1 == 0 && val2 > 0 {
                markers_start = x;
                break;
            } else if val1 == 0 && val2 == 0 {
                x += 4;
                continue;
            }
            return None;
        }

        // Detect 8-byte alignment: the high word of each 8-byte slot repeats.
        let mut is_align8 = false;
        let mut cnt = 5;
        let mut last_hi = 0u32;
        let mut y = markers_start + 4;
        while y + 4 < n {
            let hi = rd32(buf, y + 4);
            if hi != last_hi {
                break;
            }
            last_hi = hi;
            cnt -= 1;
            if cnt == 0 {
                is_align8 = true;
                break;
            }
            y += 8;
        }

        let entries = ((num as usize + 255) >> 8) * 4;
        let markers_end = if is_align8 {
            let back = align_up8(markers_start) - markers_start;
            markers_start -= if back == 0 { 8 } else { back };
            markers_start + entries * 2
        } else {
            markers_start + entries
        };
        Some((markers_start, markers_end))
    }

    /// Locate `kallsyms_token_table` + `kallsyms_token_index` by scanning forward
    /// from the markers list: a real token table is 256 short NUL-terminated
    /// strings whose token index expands the first symbols into clean, printable
    /// names. This skips any interposed `kallsyms_seqs_of_names`.
    fn find_token_tables(&self, names_offset: usize, from: usize, num: u32) -> Option<Tables> {
        let buf = self.buf;
        let n = buf.len();
        let mut x = align_up8(from);
        while x + 4 < n {
            if rd32(buf, x) == 0 {
                x += 4;
                continue;
            }
            // The real token table is compact (256 tokens of a few bytes).
            if let Some(tt_end) = self.parse_token_strings(x).filter(|&end| end - x <= 0x1000)
                && let Some(ti) = self.find_token_index(tt_end)
            {
                let tables = Tables {
                    names_offset,
                    token_table_offset: x,
                    token_index_offset: ti,
                };
                if self.token_tables_decode_cleanly(&tables, num) {
                    return Some(tables);
                }
            }
            x += 4;
        }
        None
    }

    /// Walk 256 consecutive NUL-terminated strings from `x`; returns the end
    /// offset, or `None` if the buffer runs out first.
    fn parse_token_strings(&self, x: usize) -> Option<usize> {
        let buf = self.buf;
        let mut off = x;
        for _ in 0..256 {
            let rel = buf[off..].iter().position(|&c| c == 0)?;
            off += rel + 1;
        }
        Some(off)
    }

    /// `kallsyms_token_index`: a `u16` table; starts at the first `(0, >0)` pair.
    fn find_token_index(&self, token_table_end: usize) -> Option<usize> {
        let buf = self.buf;
        let n = buf.len();
        let start = align_up8(token_table_end);
        let mut x = start;
        while x + 2 < n {
            let val1 = rd16(buf, x);
            let val2 = rd16(buf, x + 2);
            if val1 == 0 && val2 > 0 {
                return Some(x);
            } else if val1 == 0 && val2 == 0 {
                x += 2;
                continue;
            }
            return None;
        }
        None
    }

    /// Accept a token-table/index candidate only if it expands the first symbols
    /// into non-empty, printable names, which a misaligned (e.g. seqs-data)
    /// candidate never does.
    fn token_tables_decode_cleanly(&self, tables: &Tables, num: u32) -> bool {
        let probe = num.min(16);
        if probe == 0 {
            return false;
        }
        let mut off = 0usize;
        for _ in 0..probe {
            if tables.names_offset + off >= self.buf.len() {
                return false;
            }
            let (name, next) = self.expand_symbol(tables, off);
            off = next;
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
                return false;
            }
        }
        true
    }

    /// Walk all `num` symbols, expanding each name and computing its raw VA.
    fn decode_symbols(
        &self,
        num: u32,
        offsets_offset: usize,
        relative_base: u64,
        tables: &Tables,
    ) -> Vec<(String, u64)> {
        let mut out = Vec::with_capacity(num as usize);
        let mut off = 0usize;
        for i in 0..num as usize {
            let (name, next) = self.expand_symbol(tables, off);
            off = next;
            let addr = self.sym_address(offsets_offset, relative_base, i);
            out.push((name, addr));
        }
        out
    }

    /// Raw VA of symbol `idx` from `kallsyms_offsets` (ABSOLUTE_PERCPU rules).
    fn sym_address(&self, offsets_offset: usize, relative_base: u64, idx: usize) -> u64 {
        let off = rd32(self.buf, offsets_offset + idx * 4) as i32;
        if off >= 0 {
            // Per-cpu / absolute entries: positive offsets are absolute values.
            off as u64
        } else if self.profile.negative_uses_base {
            // Negative offsets are relative to `relative_base - 1`.
            relative_base
                .wrapping_sub(1)
                .wrapping_sub(off as i64 as u64)
        } else {
            0
        }
    }

    /// Expand one compressed symbol (upstream `kallsyms_expand_symbol`).
    /// Returns the name and the stream offset of the next symbol. The varint
    /// length form is backward compatible with the single-byte form.
    fn expand_symbol(&self, t: &Tables, mut off: usize) -> (String, usize) {
        let buf = self.buf;
        let names = t.names_offset;
        let mut len = match buf.get(names + off) {
            Some(&b) => b as usize,
            None => return (String::new(), off + 1),
        };
        off += 1;
        if (len & 0x80) != 0 {
            len = (len & 0x7F) | ((buf.get(names + off).copied().unwrap_or(0) as usize) << 7);
            off += 1;
        }
        off += len;

        let data_start = names + (off - len);
        let mut out = String::new();
        let mut skipped_first = false;
        for i in 0..len {
            let x = match buf.get(data_start + i) {
                Some(&b) => b as usize,
                None => break,
            };
            if t.token_index_offset + x * 2 + 2 > buf.len() {
                break;
            }
            let y = rd16(buf, t.token_index_offset + x * 2) as usize;
            let mut tptr = t.token_table_offset + y;
            while matches!(buf.get(tptr), Some(&b) if b != 0) {
                if skipped_first {
                    out.push(buf[tptr] as char);
                } else {
                    skipped_first = true;
                }
                tptr += 1;
            }
        }
        (out, off)
    }
}

/// Resolved offsets of the three tables needed to expand names.
struct Tables {
    names_offset: usize,
    token_table_offset: usize,
    token_index_offset: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_dispatch_matches_upstream_ranges() {
        assert!(!Profile::for_version((5, 10, 233)).has_relative_base);
        assert!(!Profile::for_version((5, 10, 233)).names_varint);
        assert_eq!(Profile::for_version((5, 10, 233)).ksym_name_len, 128);

        assert!(Profile::for_version((6, 1, 0)).names_varint);
        assert!(!Profile::for_version((6, 1, 0)).has_relative_base);

        let p = Profile::for_version((6, 1, 128));
        assert!(p.has_relative_base && p.negative_uses_base);
        assert!(matches!(p.seqs, Seqs::AfterMarkers));
        assert!(matches!(p.num_from, NumFrom::RelBaseEnd));

        let p = Profile::for_version((6, 12, 30));
        assert!(matches!(p.seqs, Seqs::AfterRelBase));
        assert!(matches!(p.num_from, NumFrom::Zero));
    }

    #[test]
    fn va_recognizer() {
        assert!(looks_kernel_va(0xFFFF_FFC0_0801_0000));
        assert!(looks_kernel_va(0xFFFF_8000_1234_5678));
        assert!(!looks_kernel_va(0x0000_0000_0801_0000));
    }

    #[test]
    fn expand_symbol_decompresses_tokens() {
        // Hand-built kallsyms token machinery: one symbol "main" encoded as the
        // type token "t" (its single char is the skipped type prefix) followed
        // by the token "main". Layout: [names][token_index u16×2][token_table].
        let names: usize = 0;
        let ti: usize = 8;
        let tt: usize = 16;
        let mut buf = vec![0u8; 32];
        // names: len=2, token bytes [0, 1].
        buf[names] = 0x02;
        buf[names + 1] = 0x00;
        buf[names + 2] = 0x01;
        // token_index: token 0 at tt+0, token 1 at tt+2.
        buf[ti..ti + 2].copy_from_slice(&0u16.to_le_bytes());
        buf[ti + 2..ti + 4].copy_from_slice(&2u16.to_le_bytes());
        // token_table: "t\0" then "main\0".
        buf[tt..tt + 2].copy_from_slice(b"t\0");
        buf[tt + 2..tt + 7].copy_from_slice(b"main\0");

        let dec = Decoder {
            buf: &buf,
            profile: Profile::for_version((6, 1, 60)),
        };
        let tables = Tables {
            names_offset: names,
            token_table_offset: tt,
            token_index_offset: ti,
        };
        let (name, next) = dec.expand_symbol(&tables, 0);
        assert_eq!(name, "main");
        assert_eq!(next, 3); // 1 length byte + 2 token bytes
    }
}
