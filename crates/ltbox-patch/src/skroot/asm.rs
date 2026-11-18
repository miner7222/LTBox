//! Minimal AArch64 instruction encoder standing in for upstream's asmjit. Only
//! the forms the SKRoot Lite patches emit are implemented, each a fixed-shape
//! encoding from the ARM ARM.
//!
//! Output is byte-accurate: [`Asm`] appends little-endian words (and arbitrary
//! `embed`ed data) into one buffer, tracks byte offsets, and resolves
//! label-relative branches at [`Asm::to_bytes`]. Wrong encodings here would
//! brick a kernel, so every form is covered by a round-trip / known-value test.
#![allow(dead_code)]

/// Register number 0..=30; `31` is `xzr`/`wzr` (or `sp` for the few forms that
/// read it as the stack pointer, handled per-instruction).
pub const ZR: u32 = 31;
pub const SP: u32 = 31;

/// AArch64 condition codes (the `cond` field of `B.cond`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Cond {
    Eq = 0x0,
    Ne = 0x1,
    Cs = 0x2,
    Cc = 0x3,
    Mi = 0x4,
    Pl = 0x5,
    Vs = 0x6,
    Vc = 0x7,
    Hi = 0x8,
    Ls = 0x9,
    Ge = 0xA,
    Lt = 0xB,
    Gt = 0xC,
    Le = 0xD,
    Al = 0xE,
}

#[derive(Clone, Copy)]
pub struct Label(usize);

#[derive(Clone, Copy)]
enum FixKind {
    B,
    Bl,
    Bcond(Cond),
    Cb { is_x: bool, set: bool, rt: u32 }, // set=true → CBNZ, false → CBZ
}

struct Fixup {
    pos: usize, // byte offset of the branch word
    label: usize,
    kind: FixKind,
}

/// An encoding that cannot be represented — emitting it anyway would brick the
/// kernel, so the builder records it and [`Asm::to_bytes`] fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsmError {
    /// A `b`/`bl` target is beyond ±128 MiB.
    BranchOutOfRange,
    /// A `b.cond`/`cbz`/`cbnz` target is beyond ±1 MiB.
    CondBranchOutOfRange,
    /// A branch byte-offset is not a multiple of 4.
    BranchMisaligned,
    /// An `adr` target is beyond ±1 MiB.
    AdrOutOfRange,
    /// An `adrp` page delta is beyond ±4 GiB.
    AdrpOutOfRange,
}

/// AArch64 code builder.
#[derive(Default)]
pub struct Asm {
    buf: Vec<u8>,
    labels: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
    err: Option<AsmError>,
}

impl Asm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current size in bytes (asmjit's `offset()`).
    pub fn offset(&self) -> usize {
        self.buf.len()
    }

    fn word(&mut self, w: u32) {
        self.buf.extend_from_slice(&w.to_le_bytes());
    }

    /// Append raw bytes (data / pre-assembled instructions).
    pub fn embed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn new_label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() - 1)
    }

    pub fn bind(&mut self, l: Label) {
        self.labels[l.0] = Some(self.buf.len());
    }

    /// First recorded encoding error, if any.
    pub fn error(&self) -> Option<&AsmError> {
        self.err.as_ref()
    }

    fn set_err(&mut self, e: AsmError) {
        if self.err.is_none() {
            self.err = Some(e);
        }
    }

    pub fn movz_x(&mut self, rd: u32, imm16: u16, shift: u32) {
        self.word(0xD280_0000 | ((shift / 16) << 21) | (u32::from(imm16) << 5) | rd);
    }
    pub fn movk_x(&mut self, rd: u32, imm16: u16, shift: u32) {
        self.word(0xF280_0000 | ((shift / 16) << 21) | (u32::from(imm16) << 5) | rd);
    }
    pub fn movn_x(&mut self, rd: u32, imm16: u16, shift: u32) {
        self.word(0x9280_0000 | ((shift / 16) << 21) | (u32::from(imm16) << 5) | rd);
    }
    pub fn movz_w(&mut self, rd: u32, imm16: u16, shift: u32) {
        self.word(0x5280_0000 | ((shift / 16) << 21) | (u32::from(imm16) << 5) | rd);
    }
    pub fn movk_w(&mut self, rd: u32, imm16: u16, shift: u32) {
        self.word(0x7280_0000 | ((shift / 16) << 21) | (u32::from(imm16) << 5) | rd);
    }
    pub fn movn_w(&mut self, rd: u32, imm16: u16, shift: u32) {
        self.word(0x1280_0000 | ((shift / 16) << 21) | (u32::from(imm16) << 5) | rd);
    }

    /// `mov xd, #value` via movz/movk (mirrors upstream `aarch64_asm_mov_x`).
    pub fn mov_imm_x(&mut self, rd: u32, value: u64) {
        let mut inited = false;
        for idx in 0..4u32 {
            let imm16 = ((value >> (idx * 16)) & 0xFFFF) as u16;
            if imm16 == 0 {
                continue;
            }
            if !inited {
                self.movz_x(rd, imm16, idx * 16);
                inited = true;
            } else {
                self.movk_x(rd, imm16, idx * 16);
            }
        }
        if !inited {
            self.movz_x(rd, 0, 0);
        }
    }

    /// `mov xd, #value` as asmjit lowers the generic `mov(GpX, Imm)` alias:
    /// prefer one MOVN/MOVZ instruction, then a logical-immediate ORR alias,
    /// then the full MOVN/MOVZ+MOVK sequence.
    pub fn mov_imm_x_asmjit(&mut self, rd: u32, value: u64) {
        let seq = encode_mov_sequence64(value, rd, true);
        if seq.len() == 1 && rd != SP {
            self.word(seq[0]);
            return;
        }
        if rd != ZR && self.orr_imm_x(rd, ZR, value) {
            return;
        }
        for word in seq {
            self.word(word);
        }
    }

    /// `mov wd, #value` (32-bit) via movz/movk.
    pub fn mov_imm_w(&mut self, rd: u32, value: u32) {
        let mut inited = false;
        for idx in 0..2u32 {
            let imm16 = ((value >> (idx * 16)) & 0xFFFF) as u16;
            if imm16 == 0 {
                continue;
            }
            if !inited {
                self.movz_w(rd, imm16, idx * 16);
                inited = true;
            } else {
                self.movk_w(rd, imm16, idx * 16);
            }
        }
        if !inited {
            self.movz_w(rd, 0, 0);
        }
    }

    /// `mov xd, xm` (`orr xd, xzr, xm`).
    pub fn mov_reg_x(&mut self, rd: u32, rm: u32) {
        self.word(0xAA00_03E0 | (rm << 16) | rd);
    }
    /// `mov wd, wm`.
    pub fn mov_reg_w(&mut self, rd: u32, rm: u32) {
        self.word(0x2A00_03E0 | (rm << 16) | rd);
    }
    /// `mov xd, sp` (`add xd, sp, #0`).
    pub fn mov_x_sp(&mut self, rd: u32) {
        self.word(0x9100_0000 | (SP << 5) | rd);
    }

    /// `mrs xt, sp_el0` — the only system register SKRoot Lite reads.
    pub fn mrs_sp_el0(&mut self, rt: u32) {
        self.word(0xD538_4100 | rt);
    }

    /// `add xd, xn, #imm12` (no shift, `imm12 <= 0xFFF`).
    pub fn add_imm_x(&mut self, rd: u32, rn: u32, imm12: u32) {
        self.word(0x9100_0000 | ((imm12 & 0xFFF) << 10) | (rn << 5) | rd);
    }
    /// `sub xd, xn, #imm12` (no shift, `imm12 <= 0xFFF`).
    pub fn sub_imm_x(&mut self, rd: u32, rn: u32, imm12: u32) {
        self.word(0xD100_0000 | ((imm12 & 0xFFF) << 10) | (rn << 5) | rd);
    }
    /// `subs xd, xn, #imm12` (no shift, `imm12 <= 0xFFF`).
    pub fn subs_imm_x(&mut self, rd: u32, rn: u32, imm12: u32) {
        self.word(0xF100_0000 | ((imm12 & 0xFFF) << 10) | (rn << 5) | rd);
    }
    /// `add xd, xn, xm`.
    pub fn add_reg_x(&mut self, rd: u32, rn: u32, rm: u32) {
        self.word(0x8B00_0000 | (rm << 16) | (rn << 5) | rd);
    }
    /// `bic xd, xn, xm` (shifted register, shift 0).
    pub fn bic_reg_x(&mut self, rd: u32, rn: u32, rm: u32) {
        self.word(0x8A20_0000 | (rm << 16) | (rn << 5) | rd);
    }
    /// `cmp xn, xm` (`subs xzr, xn, xm`).
    pub fn cmp_reg_x(&mut self, rn: u32, rm: u32) {
        self.word(0xEB00_0000 | (rm << 16) | (rn << 5) | ZR);
    }
    pub fn cmp_reg_w(&mut self, rn: u32, rm: u32) {
        self.word(0x6B00_0000 | (rm << 16) | (rn << 5) | ZR);
    }
    /// `cmp xn, #imm12` (`subs xzr, xn, #imm`).
    pub fn cmp_imm_x(&mut self, rn: u32, imm12: u32) {
        self.word(0xF100_0000 | ((imm12 & 0xFFF) << 10) | (rn << 5) | ZR);
    }
    pub fn cmp_imm_w(&mut self, rn: u32, imm12: u32) {
        self.word(0x7100_0000 | ((imm12 & 0xFFF) << 10) | (rn << 5) | ZR);
    }

    /// `and xd, xn, #imm` (64-bit logical immediate). Returns `false` (and emits
    /// nothing) when `imm` is not a representable bitmask immediate.
    pub fn and_imm_x(&mut self, rd: u32, rn: u32, imm: u64) -> bool {
        match encode_logical_imm64(imm) {
            Some((n, immr, imms)) => {
                self.word(0x9200_0000 | (n << 22) | (immr << 16) | (imms << 10) | (rn << 5) | rd);
                true
            }
            None => false,
        }
    }
    /// `orr xd, xn, #imm` (64-bit logical immediate). The `mov #imm` alias uses
    /// this with `xn = xzr` when the immediate is encodable as a bitmask.
    pub fn orr_imm_x(&mut self, rd: u32, rn: u32, imm: u64) -> bool {
        match encode_logical_imm64(imm) {
            Some((n, immr, imms)) => {
                self.word(0xB200_0000 | (n << 22) | (immr << 16) | (imms << 10) | (rn << 5) | rd);
                true
            }
            None => false,
        }
    }

    /// `ldr xt, [xn, #imm]` (unsigned offset, `imm` a multiple of 8).
    pub fn ldr_x_uoff(&mut self, rt: u32, rn: u32, imm: u32) {
        self.word(0xF940_0000 | (((imm / 8) & 0xFFF) << 10) | (rn << 5) | rt);
    }
    /// `ldr wt, [xn, #imm]` (unsigned offset, `imm` a multiple of 4).
    pub fn ldr_w_uoff(&mut self, rt: u32, rn: u32, imm: u32) {
        self.word(0xB940_0000 | (((imm / 4) & 0xFFF) << 10) | (rn << 5) | rt);
    }
    /// `str xt, [xn, #imm]` (unsigned offset).
    pub fn str_x_uoff(&mut self, rt: u32, rn: u32, imm: u32) {
        self.word(0xF900_0000 | (((imm / 8) & 0xFFF) << 10) | (rn << 5) | rt);
    }
    /// `str wt, [xn, #imm]` (unsigned offset, `imm` a multiple of 4).
    pub fn str_w_uoff(&mut self, rt: u32, rn: u32, imm: u32) {
        self.word(0xB900_0000 | (((imm / 4) & 0xFFF) << 10) | (rn << 5) | rt);
    }
    /// `str xt, [xn], #imm` (post-index, signed 9-bit).
    pub fn str_x_post(&mut self, rt: u32, rn: u32, imm: i32) {
        self.word(0xF800_0400 | ((u32_imm9(imm)) << 12) | (rn << 5) | rt);
    }
    /// `ldr xt, [xn], #imm` (post-index, signed 9-bit).
    pub fn ldr_x_post(&mut self, rt: u32, rn: u32, imm: i32) {
        self.word(0xF840_0400 | ((u32_imm9(imm)) << 12) | (rn << 5) | rt);
    }
    /// `str wt, [xn], #imm` (post-index).
    pub fn str_w_post(&mut self, rt: u32, rn: u32, imm: i32) {
        self.word(0xB800_0400 | ((u32_imm9(imm)) << 12) | (rn << 5) | rt);
    }
    /// `ldr wt, [xn], #imm` (post-index).
    pub fn ldr_w_post(&mut self, rt: u32, rn: u32, imm: i32) {
        self.word(0xB840_0400 | ((u32_imm9(imm)) << 12) | (rn << 5) | rt);
    }
    /// `ldrb wt, [xn], #imm` (post-index).
    pub fn ldrb_w_post(&mut self, rt: u32, rn: u32, imm: i32) {
        self.word(0x3840_0400 | ((u32_imm9(imm)) << 12) | (rn << 5) | rt);
    }
    /// `ldrb wt, [xn, xm]` (register offset, LSL 0).
    pub fn ldrb_w_reg(&mut self, rt: u32, rn: u32, rm: u32) {
        self.word(0x3860_6800 | (rm << 16) | (rn << 5) | rt);
    }
    /// `stp xt, xt2, [xn, #imm]!` (pre-index, `imm` a multiple of 8).
    pub fn stp_x_pre(&mut self, rt: u32, rt2: u32, rn: u32, imm: i32) {
        self.word(0xA980_0000 | (u32_imm7_scaled8(imm) << 15) | (rt2 << 10) | (rn << 5) | rt);
    }
    /// `stp xt, xt2, [xn], #imm` (post-index, `imm` a multiple of 8).
    pub fn stp_x_post(&mut self, rt: u32, rt2: u32, rn: u32, imm: i32) {
        self.word(0xA880_0000 | (u32_imm7_scaled8(imm) << 15) | (rt2 << 10) | (rn << 5) | rt);
    }
    /// `ldp xt, xt2, [xn], #imm` (post-index).
    pub fn ldp_x_post(&mut self, rt: u32, rt2: u32, rn: u32, imm: i32) {
        self.word(0xA8C0_0000 | (u32_imm7_scaled8(imm) << 15) | (rt2 << 10) | (rn << 5) | rt);
    }
    /// `ldaxr xt, [xn]`.
    pub fn ldaxr_x(&mut self, rt: u32, rn: u32) {
        self.word(0xC85F_FC00 | (rn << 5) | rt);
    }
    /// `stlxr ws, xt, [xn]`.
    pub fn stlxr_w_x(&mut self, rs: u32, rt: u32, rn: u32) {
        self.word(0xC800_FC00 | (rs << 16) | (rn << 5) | rt);
    }

    /// `adr xd, .+byte_off` (signed 21-bit byte range, ±1 MiB).
    pub fn adr(&mut self, rd: u32, byte_off: i32) {
        if !(-(1 << 20)..(1 << 20)).contains(&byte_off) {
            self.set_err(AsmError::AdrOutOfRange);
            self.word(0);
            return;
        }
        let imm = (byte_off as u32) & 0x1F_FFFF;
        let immlo = imm & 0x3;
        let immhi = (imm >> 2) & 0x7_FFFF;
        self.word(0x1000_0000 | (immlo << 29) | (immhi << 5) | rd);
    }

    /// `adrp xd, page(target)` from this instruction's absolute address.
    /// `false` (and records [`AsmError::AdrpOutOfRange`]) when the page delta is
    /// out of the ±4 GiB range.
    pub fn adrp(&mut self, rd: u32, cur_abs: u64, target_abs: u64) -> bool {
        let page_delta = (target_abs & !0xFFF) as i64 - (cur_abs & !0xFFF) as i64;
        let imm_pages = page_delta >> 12;
        if !(-(1 << 20)..(1 << 20)).contains(&imm_pages) {
            self.set_err(AsmError::AdrpOutOfRange);
            self.word(0);
            return false;
        }
        let imm21 = (imm_pages as u64 & 0x1F_FFFF) as u32;
        let immlo = imm21 & 0x3;
        let immhi = (imm21 >> 2) & 0x7_FFFF;
        self.word(0x9000_0000 | (immlo << 29) | (immhi << 5) | rd);
        true
    }

    /// `ret` (`ret x30`).
    pub fn ret(&mut self) {
        self.word(0xD65F_03C0);
    }
    pub fn autiaz(&mut self) {
        self.word(0xD503_239F);
    }
    pub fn autiasp(&mut self) {
        self.word(0xD503_23BF);
    }
    pub fn autibz(&mut self) {
        self.word(0xD503_23DF);
    }
    pub fn autibsp(&mut self) {
        self.word(0xD503_23FF);
    }

    /// `b .+byte_off` (raw, no label).
    pub fn b_off(&mut self, byte_off: i32) {
        match encode_branch26(byte_off) {
            Ok(imm) => self.word(0x1400_0000 | imm),
            Err(e) => {
                self.set_err(e);
                self.word(0);
            }
        }
    }
    /// `bl .+byte_off` (raw, no label).
    pub fn bl_off(&mut self, byte_off: i32) {
        match encode_branch26(byte_off) {
            Ok(imm) => self.word(0x9400_0000 | imm),
            Err(e) => {
                self.set_err(e);
                self.word(0);
            }
        }
    }

    /// `b label` — resolved at [`to_bytes`].
    pub fn b(&mut self, l: Label) {
        self.push_fixup(l, FixKind::B);
    }
    /// `bl label`.
    pub fn bl(&mut self, l: Label) {
        self.push_fixup(l, FixKind::Bl);
    }
    /// `b.<cond> label`.
    pub fn b_cond(&mut self, cond: Cond, l: Label) {
        self.push_fixup(l, FixKind::Bcond(cond));
    }
    pub fn cbz_w(&mut self, rt: u32, l: Label) {
        self.push_fixup(
            l,
            FixKind::Cb {
                is_x: false,
                set: false,
                rt,
            },
        );
    }
    pub fn cbnz_w(&mut self, rt: u32, l: Label) {
        self.push_fixup(
            l,
            FixKind::Cb {
                is_x: false,
                set: true,
                rt,
            },
        );
    }
    pub fn cbz_x(&mut self, rt: u32, l: Label) {
        self.push_fixup(
            l,
            FixKind::Cb {
                is_x: true,
                set: false,
                rt,
            },
        );
    }
    pub fn cbnz_x(&mut self, rt: u32, l: Label) {
        self.push_fixup(
            l,
            FixKind::Cb {
                is_x: true,
                set: true,
                rt,
            },
        );
    }

    fn push_fixup(&mut self, l: Label, kind: FixKind) {
        self.fixups.push(Fixup {
            pos: self.buf.len(),
            label: l.0,
            kind,
        });
        self.word(0); // placeholder, patched in `to_bytes`
    }

    /// Finalize: resolve every label branch and return the machine code.
    /// Returns the first [`AsmError`] (range/alignment) recorded during emission
    /// or hit while resolving a fixup — emitting a truncated branch would point
    /// at the wrong kernel address. Panics if a referenced label was never bound
    /// (a build-time bug, not user input).
    pub fn to_bytes(mut self) -> Result<Vec<u8>, AsmError> {
        if let Some(e) = self.err.clone() {
            return Err(e);
        }
        let fixups = std::mem::take(&mut self.fixups);
        for fx in &fixups {
            let target = self.labels[fx.label].expect("unbound label");
            let rel = target as i64 - fx.pos as i64;
            let word = match fx.kind {
                FixKind::B => 0x1400_0000 | encode_branch26_rel(rel)?,
                FixKind::Bl => 0x9400_0000 | encode_branch26_rel(rel)?,
                FixKind::Bcond(c) => 0x5400_0000 | encode_branch19_rel(rel)? | c as u32,
                FixKind::Cb { is_x, set, rt } => {
                    let base = match (is_x, set) {
                        (false, false) => 0x3400_0000,
                        (false, true) => 0x3500_0000,
                        (true, false) => 0xB400_0000,
                        (true, true) => 0xB500_0000,
                    };
                    base | encode_branch19_rel(rel)? | rt
                }
            };
            self.buf[fx.pos..fx.pos + 4].copy_from_slice(&word.to_le_bytes());
        }
        Ok(self.buf)
    }
}

/// Encode a `b`/`bl` byte offset into the 26-bit immediate field, rejecting
/// misaligned or out-of-range (±128 MiB) targets.
fn encode_branch26(byte_off: i32) -> Result<u32, AsmError> {
    encode_branch26_rel(byte_off as i64)
}

fn encode_branch26_rel(rel: i64) -> Result<u32, AsmError> {
    if rel % 4 != 0 {
        return Err(AsmError::BranchMisaligned);
    }
    let imm = rel >> 2;
    if !(-(1 << 25)..(1 << 25)).contains(&imm) {
        return Err(AsmError::BranchOutOfRange);
    }
    Ok((imm as u32) & 0x03FF_FFFF)
}

/// Encode a `b.cond`/`cbz`/`cbnz` byte offset into the 19-bit immediate field
/// (already shifted into place), rejecting misaligned or out-of-range
/// (±1 MiB) targets.
fn encode_branch19_rel(rel: i64) -> Result<u32, AsmError> {
    if rel % 4 != 0 {
        return Err(AsmError::BranchMisaligned);
    }
    let imm = rel >> 2;
    if !(-(1 << 18)..(1 << 18)).contains(&imm) {
        return Err(AsmError::CondBranchOutOfRange);
    }
    Ok(((imm as u32) & 0x7_FFFF) << 5)
}

/// Two's-complement 9-bit immediate for post/pre-indexed loads/stores.
fn u32_imm9(imm: i32) -> u32 {
    (imm as u32) & 0x1FF
}

/// Two's-complement 7-bit immediate (scaled by 8) for `stp`/`ldp`.
fn u32_imm7_scaled8(imm: i32) -> u32 {
    ((imm / 8) as u32) & 0x7F
}

fn count_zero_half_words64(imm: u64) -> u32 {
    u32::from((imm & 0x0000_0000_0000_FFFF) == 0)
        + u32::from((imm & 0x0000_0000_FFFF_0000) == 0)
        + u32::from((imm & 0x0000_FFFF_0000_0000) == 0)
        + u32::from((imm & 0xFFFF_0000_0000_0000) == 0)
}

fn encode_mov_sequence32(imm: u32, rd: u32, x: bool) -> Vec<u32> {
    let k_movz = 0x5280_0000 | (u32::from(x) << 31);
    let k_movn = 0x1280_0000;
    let k_movk = 0x7280_0000;

    if (imm & 0xFFFF_0000) == 0 {
        return vec![k_movz | ((imm & 0xFFFF) << 5) | rd];
    }
    if (imm & 0xFFFF_0000) == 0xFFFF_0000 {
        return vec![k_movn | (((!imm) & 0xFFFF) << 5) | rd];
    }
    if (imm & 0x0000_FFFF) == 0 {
        return vec![k_movz | (1 << 21) | ((imm >> 16) << 5) | rd];
    }
    if (imm & 0x0000_FFFF) == 0x0000_FFFF {
        return vec![k_movn | (1 << 21) | (((!imm >> 16) & 0xFFFF) << 5) | rd];
    }

    vec![
        k_movz | ((imm & 0xFFFF) << 5) | rd,
        k_movk | (1 << 21) | ((imm >> 16) << 5) | rd,
    ]
}

fn encode_mov_sequence64(mut imm: u64, rd: u32, x: bool) -> Vec<u32> {
    let k_movz = 0xD280_0000;
    let k_movn = 0x9280_0000;
    let k_movk = 0xF280_0000;

    if imm <= 0xFFFF_FFFF {
        return encode_mov_sequence32(imm as u32, rd, x);
    }

    let zhw = count_zero_half_words64(imm);
    let ohw = count_zero_half_words64(!imm);
    let mut out = Vec::with_capacity(4);

    if zhw >= ohw {
        let mut op = k_movz;
        for hw_index in 0..4u32 {
            let hw_imm = (imm & 0xFFFF) as u32;
            if hw_imm != 0 {
                out.push(op | (hw_index << 21) | (hw_imm << 5) | rd);
                op = k_movk;
            }
            imm >>= 16;
        }
    } else {
        let mut op = k_movn;
        let mut neg_mask = 0xFFFFu32;
        for hw_index in 0..4u32 {
            let hw_imm = (imm & 0xFFFF) as u32;
            if hw_imm != 0xFFFF {
                out.push(op | (hw_index << 21) | ((hw_imm ^ neg_mask) << 5) | rd);
                op = k_movk;
                neg_mask = 0;
            }
            imm >>= 16;
        }
        if out.is_empty() {
            out.push(k_movn | ((0xFFFF ^ neg_mask) << 5) | rd);
        }
    }

    out
}

/// Encode a 64-bit value as an AArch64 logical (bitmask) immediate
/// `(N, immr, imms)`, or `None` when it is not representable. Standard
/// element-size / rotation algorithm (per the ARM ARM `DecodeBitMasks`).
fn encode_logical_imm64(imm: u64) -> Option<(u32, u32, u32)> {
    if imm == 0 || imm == u64::MAX {
        return None;
    }
    // Largest element size whose value is a repeating pattern.
    let mut size = 64u32;
    while size > 2 {
        let half = size / 2;
        let mask = if half == 64 {
            u64::MAX
        } else {
            (1u64 << half) - 1
        };
        let lo = imm & mask;
        let hi = (imm >> half) & mask;
        if lo != hi {
            break;
        }
        size = half;
    }
    let mask = if size == 64 {
        u64::MAX
    } else {
        (1u64 << size) - 1
    };
    let elem = imm & mask;
    // The element must be a single contiguous run of ones (after rotation).
    let ones = elem.count_ones();
    if ones == 0 || ones == size {
        return None;
    }
    // Rotate the element so the run of ones starts at bit 0; the required right
    // rotation is the rotation that maps the canonical pattern to `elem`.
    let mut rot = 0u32;
    // Find a rotation making the low `ones` bits all set and the rest clear.
    let mut found = false;
    for r in 0..size {
        let rotated = rotate_right_in_size(elem, r, size);
        if rotated == ((1u64 << ones) - 1) {
            rot = r;
            found = true;
            break;
        }
    }
    if !found {
        return None;
    }
    // immr = element-size right-rotation that produces `elem` from the canonical
    // low-aligned run; equals `(size - rot) % size`.
    let immr = (size - rot) % size;
    // imms encodes element size in its top bits and (ones-1) in the low bits.
    // For size s = 2^k, the top (6-k) bits are 1 and the next bit is 0.
    let imms = ((!(size * 2 - 1)) & 0x3F) | (ones - 1);
    let n = if size == 64 { 1 } else { 0 };
    Some((n, immr, imms))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skroot::insn;

    fn one(build: impl FnOnce(&mut Asm)) -> u32 {
        let mut a = Asm::new();
        build(&mut a);
        let b = a.to_bytes().expect("encodes");
        assert_eq!(b.len(), 4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    #[test]
    fn fixed_encodings() {
        assert_eq!(one(|a| a.ret()), 0xD65F_03C0);
        assert_eq!(one(|a| a.autiasp()), 0xD503_23BF);
        assert_eq!(one(|a| a.mrs_sp_el0(0)), 0xD538_4100);
        assert_eq!(one(|a| a.mrs_sp_el0(10)), 0xD538_410A);
        // mov x0, x0 / mov x0, xzr
        assert_eq!(one(|a| a.mov_reg_x(0, 0)), 0xAA00_03E0);
        assert_eq!(one(|a| a.mov_reg_x(0, ZR)), 0xAA1F_03E0);
        // mov x0, sp = add x0, sp, #0
        assert_eq!(one(|a| a.mov_x_sp(0)), 0x9100_03E0);
        // these decode back through the predicate table
        assert!(insn::is_ret(one(|a| a.ret())));
        assert!(insn::is_mrs_sp_el0(one(|a| a.mrs_sp_el0(3))));
        assert!(insn::is_b(one(|a| a.b_off(0))));
        assert!(insn::is_bl(one(|a| a.bl_off(0))));
    }

    #[test]
    fn imm_moves_and_alu() {
        // movz x5, #0x1234
        assert_eq!(one(|a| a.movz_x(5, 0x1234, 0)), 0xD282_4685);
        // movk x5, #0xABCD, lsl #16
        assert_eq!(one(|a| a.movk_x(5, 0xABCD, 16)), 0xF2B5_79A5);
        // asmjit mov x11, #-4095 chooses a single movn.
        assert_eq!(
            one(|a| a.mov_imm_x_asmjit(11, 4095u64.wrapping_neg())),
            0x9281_FFCB
        );
        // asmjit mov x13, #0x1ffffffffff chooses an ORR logical immediate.
        assert_eq!(
            one(|a| a.mov_imm_x_asmjit(13, 0x1FF_FFFF_FFFF)),
            0xB240_A3ED
        );
        // add x14, x14, #8
        assert_eq!(one(|a| a.add_imm_x(14, 14, 8)), 0x9100_21CE);
        // sub sp, sp, #0x50
        assert_eq!(one(|a| a.sub_imm_x(SP, SP, 0x50)), 0xD101_43FF);
        // subs x13, x13, #1
        assert_eq!(one(|a| a.subs_imm_x(13, 13, 1)), 0xF100_05AD);
        // cmp w14, w15  (subs wzr, w14, w15)
        assert_eq!(one(|a| a.cmp_reg_w(14, 15)), 0x6B0F_01DF);
        // cmp x12, #48
        assert_eq!(one(|a| a.cmp_imm_x(12, 48)), 0xF100_C19F);
    }

    #[test]
    fn loads_stores() {
        // str xzr, [x14], #8
        assert_eq!(one(|a| a.str_x_post(ZR, 14, 8)), 0xF800_85DF);
        // ldr x14, [x11], #8
        assert_eq!(one(|a| a.ldr_x_post(14, 11, 8)), 0xF840_856E);
        // ldr w12, [x11, #24]
        assert_eq!(one(|a| a.ldr_w_uoff(12, 11, 24)), 0xB940_196C);
        // ldr w13, [x11], #8
        assert_eq!(one(|a| a.ldr_w_post(13, 11, 8)), 0xB840_856D);
        // ldrb w14, [x11], #1
        assert_eq!(one(|a| a.ldrb_w_post(14, 11, 1)), 0x3840_156E);
        // stp x29, x30, [sp, #-16]!
        assert_eq!(one(|a| a.stp_x_pre(29, 30, SP, -16)), 0xA9BF_7BFD);
        // ldp x29, x30, [sp], #16
        assert_eq!(one(|a| a.ldp_x_post(29, 30, SP, 16)), 0xA8C1_7BFD);
        // ldaxr x14, [x12]
        assert_eq!(one(|a| a.ldaxr_x(14, 12)), 0xC85F_FD8E);
        // stlxr w15, x14, [x12]
        assert_eq!(one(|a| a.stlxr_w_x(15, 14, 12)), 0xC80F_FD8E);
    }

    #[test]
    fn logical_immediate_roundtrip() {
        // and x13, x13, #~(0x4000-1)  (clear low 14 bits)
        let val = !0x3FFFu64;
        let w = one(|a| {
            assert!(a.and_imm_x(13, 13, val));
        });
        assert!(insn::is_and_imm(w));
        assert_eq!(decode_logical_imm64(w), Some(val));
        // a few more representable masks
        for v in [0xFu64, 0xFF00u64, 0xFFFF_FFFF_0000_0000u64, !0xFFFu64] {
            let w = one(|a| {
                assert!(a.and_imm_x(0, 0, v), "{v:#x} should encode");
            });
            assert_eq!(decode_logical_imm64(w), Some(v), "roundtrip {v:#x}");
        }
        // not representable
        assert!(!Asm::new().and_imm_x(0, 0, 0));
        assert!(!Asm::new().and_imm_x(0, 0, u64::MAX));
    }

    #[test]
    fn labels_resolve() {
        // forward conditional + backward unconditional branch.
        let mut a = Asm::new();
        let top = a.new_label();
        let end = a.new_label();
        a.bind(top); // 0
        a.cmp_imm_w(2, 0); // 0x0
        a.b_cond(Cond::Ne, end); // 0x4
        a.b(top); // 0x8 -> back to 0
        a.bind(end); // 0xC
        a.ret();
        let bytes = a.to_bytes().expect("encodes");
        let w = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        // b.ne end: from 0x4 to 0xC = +8 bytes = imm19 2
        assert_eq!(w(4), 0x5400_0000 | (2 << 5) | Cond::Ne as u32);
        // b top: from 0x8 to 0x0 = -8 bytes = imm26 -2
        assert_eq!(w(8), 0x1400_0000 | (((-2i32) as u32) & 0x03FF_FFFF));
    }

    #[test]
    fn out_of_range_branches_fail() {
        // raw b beyond ±128 MiB → recorded error, to_bytes fails.
        let mut a = Asm::new();
        a.b_off(0x0800_0000); // exactly 128 MiB = first unrepresentable offset
        assert_eq!(a.to_bytes(), Err(AsmError::BranchOutOfRange));

        // misaligned raw branch.
        let mut a = Asm::new();
        a.bl_off(2);
        assert_eq!(a.to_bytes(), Err(AsmError::BranchMisaligned));

        // forward cond branch past ±1 MiB via a far-away label.
        let mut a = Asm::new();
        let far = a.new_label();
        a.b_cond(Cond::Eq, far); // at 0x0
        for _ in 0..(1 << 18) {
            a.ret(); // 4-byte filler
        }
        a.bind(far); // past +1 MiB, imm19 ≥ 1<<18 → out of range
        assert_eq!(a.to_bytes(), Err(AsmError::CondBranchOutOfRange));

        // adr past ±1 MiB.
        let mut a = Asm::new();
        a.adr(0, 1 << 20);
        assert_eq!(a.to_bytes(), Err(AsmError::AdrOutOfRange));
    }

    /// Decode a 64-bit logical-immediate instruction back to its value — the
    /// inverse of [`encode_logical_imm64`], following the ARM `DecodeBitMasks`.
    /// A spec-correct inverse here proves the encoder is spec-correct (not just
    /// self-consistent) via the round-trip test.
    fn decode_logical_imm64(insn: u32) -> Option<u64> {
        let n = (insn >> 22) & 1;
        let immr = (insn >> 16) & 0x3F;
        let imms = (insn >> 10) & 0x3F;
        // Element size = 2^len where len is the position of the highest set bit
        // of the 7-bit value (N : NOT(imms)).
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
        // Canonical low-aligned run of `ones`, rotated right by immr, then the
        // `size`-bit element replicated across 64 bits.
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
}
