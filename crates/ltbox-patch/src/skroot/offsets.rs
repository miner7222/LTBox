//! Struct-offset finders — port of upstream `3rdparty/find_mrs_register.h`,
//! `find_imm_register_offset.h`, and `find_adrp_target.h`.
//!
//! These recover the `task_struct`/`cred` field offsets the patches need by
//! scanning a function's body. They are shallow register scans, not a full
//! data-flow analysis, so they decode straight off the [`insn`] predicates
//! instead of a disassembler:
//!
//! * [`find_cred_offset`] / [`find_seccomp_offset`]: locate `current` (via
//!   `mrs xN, sp_el0` or the `and xN, xM, #~(THREAD_SIZE-1)` form), then take
//!   the first field loaded off it past `0x400` — the `cred` / `seccomp`
//!   pointer inside `task_struct`.
//! * [`find_cred_uid_offset`]: the first small displacement that follows the
//!   `cred` load in `sys_getuid` — the `uid` slot, i.e. the credential's atomic
//!   usage-counter size (4 or 8).
//! * [`find_huawei_kti_addr`]: resolve the `adrp`+load target in
//!   `kti_randomize_init` (Huawei/Honor KTI only).
#![allow(dead_code)]

use super::insn;

fn rd32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// A value loaded/added off a tracked base register: destination reg + byte
/// offset (upstream `track_reg_info`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrackedLoad {
    dest: u32,
    offset: i64,
}

/// Decode a 64-bit AArch64 logical (bitmask) immediate from its instruction
/// encoding, or `None` if the fields are not a valid bitmask. Standard
/// `DecodeBitMasks`.
fn decode_logical_imm64(insn: u32) -> Option<u64> {
    let n = (insn >> 22) & 1;
    let immr = (insn >> 16) & 0x3F;
    let imms = (insn >> 10) & 0x3F;
    let nn = (n << 6) | ((!imms) & 0x3F);
    if nn == 0 {
        return None;
    }
    let len = 31 - nn.leading_zeros();
    let size = 1u32 << len;
    let ones = (imms & (size - 1)) + 1;
    if ones >= size {
        return None;
    }
    let pattern = (1u64 << ones) - 1;
    let elem = rotate_right_in_size(pattern, immr & (size - 1), size);
    let mut out = 0u64;
    let mut shift = 0u32;
    while shift < 64 {
        out |= elem << shift;
        shift += size;
    }
    Some(out)
}

fn rotate_right_in_size(v: u64, r: u32, size: u32) -> u64 {
    if r == 0 {
        return v;
    }
    let mask = if size == 64 {
        u64::MAX
    } else {
        (1u64 << size) - 1
    };
    ((v >> r) | (v << (size - r))) & mask
}

/// Mask of an `and xN, xM, #~(THREAD_SIZE-1)` used to derive `current` from
/// `sp` (THREAD_SIZE = 16 KiB).
const THREAD_SIZE_MASK: u64 = 0xFFFF_FFFF_FFFF_C000;

/// Find the register holding `current` (`task_struct *`): either `mrs xN,
/// sp_el0` or `and xN, xM, #THREAD_SIZE_MASK`. Returns the index of the next
/// instruction and the register number.
fn find_current_reg(buf: &[u8], start: usize, end: usize) -> Option<(usize, u32)> {
    let mut i = start;
    while i + 4 <= end {
        let w = rd32(buf, i);
        if insn::is_mrs_sp_el0(w) {
            return Some(((i - start) / 4 + 1, w & 0x1F));
        }
        i += 4;
    }
    // Fall back to the SP-masking form.
    let mut i = start;
    while i + 4 <= end {
        let w = rd32(buf, i);
        if insn::is_and_imm(w) && decode_logical_imm64(w) == Some(THREAD_SIZE_MASK) {
            return Some(((i - start) / 4 + 1, w & 0x1F));
        }
        i += 4;
    }
    None
}

/// Collect every value loaded/added off `base` from `first_idx` onward.
/// `with_str` also tracks `str` (used by the adrp-target finder).
fn track_loads(
    buf: &[u8],
    start: usize,
    end: usize,
    first_idx: usize,
    base: u32,
    with_str: bool,
) -> Vec<TrackedLoad> {
    let mut out = Vec::new();
    let mut i = start + first_idx * 4;
    while i + 4 <= end {
        let w = rd32(buf, i);
        if let Some(load) = decode_base_offset(w, base, with_str) {
            out.push(load);
        }
        i += 4;
    }
    out
}

/// Decode an `ldr`/`ldrsw`/`add` (optionally `str`) into `Xd` off base `Xn`,
/// returning the destination register and byte offset when `Xn == base`.
///
/// Only X-destination forms are tracked, as upstream does (its text match keys
/// on `"x%d"`). Deliberate: consumers take the first field past `0x400`, and
/// `task_struct` has many 32-bit `int` fields there that `ldr wN` tracking
/// would surface instead. The `cred` access is a 64-bit `ldr`, and
/// `current->seccomp.mode` (an `int`) is read with `ldrsw`, verified across the
/// 5.10-6.12 targets.
fn decode_base_offset(w: u32, base: u32, with_str: bool) -> Option<TrackedLoad> {
    let rn = (w >> 5) & 0x1F;
    if rn != base {
        return None;
    }
    let dest = w & 0x1F;
    let imm12 = ((w >> 10) & 0xFFF) as i64;
    // 64-bit LDR (unsigned offset, scaled by 8).
    if (w & 0xFFC0_0000) == 0xF940_0000 {
        return Some(TrackedLoad {
            dest,
            offset: imm12 * 8,
        });
    }
    // LDRSW (unsigned offset, scaled by 4).
    if (w & 0xFFC0_0000) == 0xB980_0000 {
        return Some(TrackedLoad {
            dest,
            offset: imm12 * 4,
        });
    }
    // ADD Xd, Xn, #imm12 (no shift).
    if (w & 0xFFC0_0000) == 0x9100_0000 {
        return Some(TrackedLoad {
            dest,
            offset: imm12,
        });
    }
    // 64-bit STR (unsigned offset, scaled by 8) — only for adrp-target tracking.
    if with_str && (w & 0xFFC0_0000) == 0xF900_0000 {
        return Some(TrackedLoad {
            dest,
            offset: imm12 * 8,
        });
    }
    None
}

/// All field offsets loaded off `current` within `[start, end)`.
fn current_task_loads(buf: &[u8], start: usize, end: usize) -> Vec<TrackedLoad> {
    match find_current_reg(buf, start, end) {
        Some((idx, reg)) => track_loads(buf, start, end, idx, reg, false),
        None => Vec::new(),
    }
}

/// `(dest_reg, byte_offset)` pairs loaded off `current` in `[start, end)`.
/// Used by the `THREAD_INFO_IN_TASK` heuristic in the patch layer.
pub(crate) fn current_task_field_loads(buf: &[u8], start: usize, end: usize) -> Vec<(u32, i64)> {
    current_task_loads(buf, start, end)
        .into_iter()
        .map(|t| (t.dest, t.offset))
        .collect()
}

/// Resolve the `cred` pointer offset inside `task_struct` from `sys_getuid`:
/// the first field loaded off `current` past `0x400`.
pub fn find_cred_offset(buf: &[u8], start: u64, size: u64) -> Option<u64> {
    first_large_load(buf, start, size)
}

/// Resolve the `seccomp` offset inside `task_struct` from `prctl_get_seccomp`.
pub fn find_seccomp_offset(buf: &[u8], start: u64, size: u64) -> Option<u64> {
    first_large_load(buf, start, size)
}

fn first_large_load(buf: &[u8], start: u64, size: u64) -> Option<u64> {
    let (s, e) = region_bounds(buf, start, size)?;
    current_task_loads(buf, s, e)
        .into_iter()
        .find(|t| t.offset > 0x400)
        .map(|t| t.offset as u64)
}

/// Ordered immediate / memory-displacement candidates in `[start, end)`,
/// skipping `b` (upstream `find_imm_register_offset`). For each instruction the
/// memory displacement comes first, then up to two immediate operands, each
/// only when positive.
fn imm_register_offsets(buf: &[u8], start: usize, end: usize) -> Vec<i64> {
    let mut out = Vec::new();
    let mut i = start;
    while i + 4 <= end {
        let w = rd32(buf, i);
        i += 4;
        if insn::is_b(w) {
            continue;
        }
        if let Some(disp) = mem_disp(w)
            && disp > 0
        {
            out.push(disp);
        }
        for imm in imm_operands(w) {
            if imm > 0 {
                out.push(imm);
            }
        }
    }
    out
}

/// Byte displacement of a load/store with an unsigned scaled immediate offset.
fn mem_disp(w: u32) -> Option<i64> {
    // LDR/STR/LDRB/… unsigned-offset family: size in bits[31:30] scales imm12.
    if (w & 0x3B00_0000) == 0x3900_0000 {
        let size = w >> 30;
        let imm12 = ((w >> 10) & 0xFFF) as i64;
        return Some(imm12 << size);
    }
    None
}

/// Positive immediate operands of common ALU/move instructions, in operand
/// order (`add`/`sub` #imm12, `movz`/`movk`/`movn` #imm16).
fn imm_operands(w: u32) -> Vec<i64> {
    // add/adds/sub/subs (immediate): bits[28:24] = 10001.
    if (w & 0x1F00_0000) == 0x1100_0000 {
        let imm12 = ((w >> 10) & 0xFFF) as i64;
        let shift = if (w >> 22) & 1 == 1 { 12 } else { 0 };
        return vec![imm12 << shift];
    }
    // movz/movn/movk: bits[28:23] = 100101.
    if (w & 0x1F80_0000) == 0x1280_0000 {
        let imm16 = ((w >> 5) & 0xFFFF) as i64;
        let hw = (w >> 21) & 0x3;
        return vec![imm16 << (hw * 16)];
    }
    Vec::new()
}

/// Resolve the `uid` slot offset (the credential atomic usage size, 4 or 8)
/// from `sys_getuid`: the first small displacement after the `cred` load.
/// `min_off` is 4 below kernel 6.6.8, otherwise 8.
pub fn find_cred_uid_offset(
    buf: &[u8],
    start: u64,
    size: u64,
    cred_offset: u64,
    min_off: i64,
) -> Option<u64> {
    let (s, e) = region_bounds(buf, start, size)?;
    let cands = imm_register_offsets(buf, s, e);
    let mut it = cands.iter();
    it.position(|&v| v == cred_offset as i64)?;
    it.find(|&&v| v >= min_off && v <= 0x20).map(|&v| v as u64)
}

/// `min_off` for [`find_cred_uid_offset`] given the kernel version triple.
pub fn cred_uid_min_off(version: (u32, u32, u32)) -> i64 {
    if version < (6, 6, 8) { 4 } else { 8 }
}

/// Resolve the Huawei KTI structure address from `kti_randomize_init`:
/// `adrp xN, page` then the first field loaded/stored off `xN`.
pub fn find_huawei_kti_addr(buf: &[u8], start: u64, size: u64) -> Option<u64> {
    let (s, e) = region_bounds(buf, start, size)?;
    let mut i = s;
    while i + 4 <= e {
        let w = rd32(buf, i);
        if insn::is_adrp(w) {
            let reg = w & 0x1F;
            // adrp page byte-delta from this instruction's page.
            let immlo = ((w >> 29) & 0x3) as i64;
            let immhi = ((w >> 5) & 0x7_FFFF) as i64;
            let pages = (immhi << 2) | immlo;
            let pages = sign_extend(pages, 21);
            let insn_off = i as i64;
            let target_page = (insn_off & !0xFFF) + (pages << 12);
            let next_idx = (i - s) / 4 + 1;
            // Skip an ADRP whose result is not consumed by a tracked load/add
            // (an unrelated PC-relative reference) and keep scanning, rather
            // than aborting the whole search on it.
            if let Some(first) = track_loads(buf, s, e, next_idx, reg, true).first() {
                return Some((target_page + first.offset) as u64);
            }
        }
        i += 4;
    }
    None
}

fn sign_extend(value: i64, bits: u32) -> i64 {
    let shift = 64 - bits;
    (value << shift) >> shift
}

/// Validate and convert a `(offset, size)` region to in-bounds byte indices.
fn region_bounds(buf: &[u8], start: u64, size: u64) -> Option<(usize, usize)> {
    if start == 0 || size == 0 {
        return None;
    }
    let s = start as usize;
    let e = (start + size) as usize;
    if e > buf.len() || s >= e {
        return None;
    }
    Some((s, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asm(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    const MRS_X0_SP_EL0: u32 = 0xD538_4100;
    const RET: u32 = 0xD65F_03C0;
    const NOP: u32 = 0xD503_201F;

    fn ldr_x(rt: u32, rn: u32, byte_off: u32) -> u32 {
        0xF940_0000 | ((byte_off / 8) << 10) | (rn << 5) | rt
    }

    #[test]
    fn decodes_thread_size_mask() {
        // and x0, x1, #0xFFFFFFFFFFFFC000  (N=1, immr=50, imms=49)
        let and = 0x9272_C420;
        assert!(insn::is_and_imm(and));
        assert_eq!(decode_logical_imm64(and), Some(THREAD_SIZE_MASK));
    }

    #[test]
    fn finds_cred_offset_via_mrs() {
        // [pad] mrs x0, sp_el0 ; ldr x0, [x0, #0x5a8] ; ret — function at off 4.
        let buf = asm(&[NOP, MRS_X0_SP_EL0, ldr_x(0, 0, 0x5a8), RET]);
        assert_eq!(find_cred_offset(&buf, 4, 12), Some(0x5a8));
    }

    #[test]
    fn cred_uid_offset_follows_cred_load() {
        // [pad] mrs x0, sp_el0 ; ldr x0,[x0,#0x5a8] ; ldr x1,[x0,#8] ; ret
        // candidate list = [0x5a8, 8]; after 0x5a8 the first value in [4,0x20] is 8.
        let buf = asm(&[NOP, MRS_X0_SP_EL0, ldr_x(0, 0, 0x5a8), ldr_x(1, 0, 8), RET]);
        let cred = find_cred_offset(&buf, 4, 16).unwrap();
        assert_eq!(cred, 0x5a8);
        assert_eq!(find_cred_uid_offset(&buf, 4, 16, cred, 4), Some(8));
    }

    #[test]
    fn min_off_by_version() {
        assert_eq!(cred_uid_min_off((6, 1, 128)), 4);
        assert_eq!(cred_uid_min_off((6, 6, 7)), 4);
        assert_eq!(cred_uid_min_off((6, 6, 8)), 8);
        assert_eq!(cred_uid_min_off((6, 12, 30)), 8);
    }

    #[test]
    fn invalid_region_is_none() {
        let buf = asm(&[MRS_X0_SP_EL0, RET]);
        // offset 0 is treated as "not found"; an oversized size is out of bounds.
        assert_eq!(find_cred_offset(&buf, 0, 8), None);
        assert_eq!(find_cred_offset(&buf, 4, 9999), None);
    }
}
