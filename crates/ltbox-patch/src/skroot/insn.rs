//! AArch64 instruction predicates — `(insn & mask) == val` bitmask checks.
//!
//! A direct port of the upstream `analyze/aarch64_insn.h` table (itself the
//! Linux kernel `__AARCH64_INSN_FUNCS` style). Decoding the kernel never needs
//! a real disassembler — every check here is a constant mask/value compare.
//!
//! The full table is kept for fidelity with upstream, although only a subset is
//! referenced.
#![allow(dead_code)]

/// Define `pub fn $name(insn: u32) -> bool { (insn & mask) == val }`.
macro_rules! insn_is {
    ($name:ident, $mask:expr, $val:expr) => {
        #[inline]
        pub fn $name(insn: u32) -> bool {
            (insn & $mask) == $val
        }
    };
}

insn_is!(is_class_branch_sys, 0x1c00_0000, 0x1400_0000);

insn_is!(is_adr, 0x9F00_0000, 0x1000_0000);
insn_is!(is_adrp, 0x9F00_0000, 0x9000_0000);
insn_is!(is_prfm, 0x3FC0_0000, 0x3980_0000);
insn_is!(is_prfm_lit, 0xFF00_0000, 0xD800_0000);
insn_is!(is_store_imm, 0x3FC0_0000, 0x3900_0000);
insn_is!(is_load_imm, 0x3FC0_0000, 0x3940_0000);
insn_is!(is_signed_load_imm, 0x3FC0_0000, 0x3980_0000);
insn_is!(is_store_pre, 0x3FE0_0C00, 0x3800_0C00);
insn_is!(is_load_pre, 0x3FE0_0C00, 0x3840_0C00);
insn_is!(is_store_post, 0x3FE0_0C00, 0x3800_0400);
insn_is!(is_load_post, 0x3FE0_0C00, 0x3840_0400);
insn_is!(is_str_reg, 0x3FE0_EC00, 0x3820_6800);
insn_is!(is_str_imm, 0x3FC0_0000, 0x3900_0000);
insn_is!(is_ldadd, 0x3F20_FC00, 0x3820_0000);
insn_is!(is_ldclr, 0x3F20_FC00, 0x3820_1000);
insn_is!(is_ldeor, 0x3F20_FC00, 0x3820_2000);
insn_is!(is_ldset, 0x3F20_FC00, 0x3820_3000);
insn_is!(is_swp, 0x3F20_FC00, 0x3820_8000);
insn_is!(is_cas, 0x3FA0_7C00, 0x08A0_7C00);
insn_is!(is_ldr_reg, 0x3FE0_EC00, 0x3860_6800);
insn_is!(is_signed_ldr_reg, 0x3FE0_FC00, 0x38A0_E800);
insn_is!(is_ldr_imm, 0x3FC0_0000, 0x3940_0000);
insn_is!(is_ldr_lit, 0xBF00_0000, 0x1800_0000);
insn_is!(is_ldrsw_lit, 0xFF00_0000, 0x9800_0000);
insn_is!(is_exclusive, 0x3F80_0000, 0x0800_0000);
insn_is!(is_load_ex, 0x3F40_0000, 0x0840_0000);
insn_is!(is_store_ex, 0x3F40_0000, 0x0800_0000);
insn_is!(is_stp, 0x7FC0_0000, 0x2900_0000);
insn_is!(is_ldp, 0x7FC0_0000, 0x2940_0000);
insn_is!(is_stp_post, 0x7FC0_0000, 0x2880_0000);
insn_is!(is_ldp_post, 0x7FC0_0000, 0x28C0_0000);
insn_is!(is_stp_pre, 0x7FC0_0000, 0x2980_0000);
insn_is!(is_ldp_pre, 0x7FC0_0000, 0x29C0_0000);
insn_is!(is_add_imm, 0x7F00_0000, 0x1100_0000);
insn_is!(is_adds_imm, 0x7F00_0000, 0x3100_0000);
insn_is!(is_sub_imm, 0x7F00_0000, 0x5100_0000);
insn_is!(is_subs_imm, 0x7F00_0000, 0x7100_0000);
insn_is!(is_movn, 0x7F80_0000, 0x1280_0000);
insn_is!(is_sbfm, 0x7F80_0000, 0x1300_0000);
insn_is!(is_bfm, 0x7F80_0000, 0x3300_0000);
insn_is!(is_movz, 0x7F80_0000, 0x5280_0000);
insn_is!(is_ubfm, 0x7F80_0000, 0x5300_0000);
insn_is!(is_movk, 0x7F80_0000, 0x7280_0000);
insn_is!(is_add, 0x7F20_0000, 0x0B00_0000);
insn_is!(is_adds, 0x7F20_0000, 0x2B00_0000);
insn_is!(is_sub, 0x7F20_0000, 0x4B00_0000);
insn_is!(is_subs, 0x7F20_0000, 0x6B00_0000);
insn_is!(is_madd, 0x7FE0_8000, 0x1B00_0000);
insn_is!(is_msub, 0x7FE0_8000, 0x1B00_8000);
insn_is!(is_udiv, 0x7FE0_FC00, 0x1AC0_0800);
insn_is!(is_sdiv, 0x7FE0_FC00, 0x1AC0_0C00);
insn_is!(is_lslv, 0x7FE0_FC00, 0x1AC0_2000);
insn_is!(is_lsrv, 0x7FE0_FC00, 0x1AC0_2400);
insn_is!(is_asrv, 0x7FE0_FC00, 0x1AC0_2800);
insn_is!(is_rorv, 0x7FE0_FC00, 0x1AC0_2C00);
insn_is!(is_rev16, 0x7FFF_FC00, 0x5AC0_0400);
insn_is!(is_rev32, 0x7FFF_FC00, 0x5AC0_0800);
insn_is!(is_rev64, 0x7FFF_FC00, 0x5AC0_0C00);
insn_is!(is_and, 0x7F20_0000, 0x0A00_0000);
insn_is!(is_bic, 0x7F20_0000, 0x0A20_0000);
insn_is!(is_orr, 0x7F20_0000, 0x2A00_0000);
insn_is!(is_mov_reg, 0x7FE0_FFE0, 0x2A00_03E0);
insn_is!(is_orn, 0x7F20_0000, 0x2A20_0000);
insn_is!(is_eor, 0x7F20_0000, 0x4A00_0000);
insn_is!(is_eon, 0x7F20_0000, 0x4A20_0000);
insn_is!(is_ands, 0x7F20_0000, 0x6A00_0000);
insn_is!(is_bics, 0x7F20_0000, 0x6A20_0000);
insn_is!(is_and_imm, 0x7F80_0000, 0x1200_0000);
insn_is!(is_orr_imm, 0x7F80_0000, 0x3200_0000);
insn_is!(is_eor_imm, 0x7F80_0000, 0x5200_0000);
insn_is!(is_ands_imm, 0x7F80_0000, 0x7200_0000);
insn_is!(is_extr, 0x7FA0_0000, 0x1380_0000);
insn_is!(is_b, 0xFC00_0000, 0x1400_0000);
insn_is!(is_bl, 0xFC00_0000, 0x9400_0000);
insn_is!(is_cbz, 0x7F00_0000, 0x3400_0000);
insn_is!(is_cbnz, 0x7F00_0000, 0x3500_0000);
insn_is!(is_tbz, 0x7F00_0000, 0x3600_0000);
insn_is!(is_tbnz, 0x7F00_0000, 0x3700_0000);
insn_is!(is_bcond, 0xFF00_0010, 0x5400_0000);
insn_is!(is_svc, 0xFFE0_001F, 0xD400_0001);
insn_is!(is_hvc, 0xFFE0_001F, 0xD400_0002);
insn_is!(is_smc, 0xFFE0_001F, 0xD400_0003);
insn_is!(is_brk, 0xFFE0_001F, 0xD420_0000);
insn_is!(is_exception, 0xFF00_0000, 0xD400_0000);
insn_is!(is_hint, 0xFFFF_F01F, 0xD503_201F);
insn_is!(is_paciaz, 0xFFFF_FFFF, 0xD503_231F);
insn_is!(is_paciasp, 0xFFFF_FFFF, 0xD503_233F);
insn_is!(is_pacibz, 0xFFFF_FFFF, 0xD503_235F);
insn_is!(is_pacibsp, 0xFFFF_FFFF, 0xD503_237F);
insn_is!(is_autiaz, 0xFFFF_FFFF, 0xD503_239F);
insn_is!(is_autiasp, 0xFFFF_FFFF, 0xD503_23BF);
insn_is!(is_autibz, 0xFFFF_FFFF, 0xD503_23DF);
insn_is!(is_autibsp, 0xFFFF_FFFF, 0xD503_23FF);
insn_is!(is_br, 0xFFFF_FC1F, 0xD61F_0000);
insn_is!(is_br_auth, 0xFEFF_F800, 0xD61F_0800);
insn_is!(is_blr, 0xFFFF_FC1F, 0xD63F_0000);
insn_is!(is_blr_auth, 0xFEFF_F800, 0xD63F_0800);
insn_is!(is_ret, 0xFFFF_FC1F, 0xD65F_0000);
insn_is!(is_retaa, 0xFFFF_FFFF, 0xD65F_0BFF);
insn_is!(is_retab, 0xFFFF_FFFF, 0xD65F_0FFF);
insn_is!(is_ret_auth, 0xFFFF_FBFF, 0xD65F_0BFF);
insn_is!(is_eret, 0xFFFF_FFFF, 0xD69F_03E0);
insn_is!(is_eret_auth, 0xFFFF_FBFF, 0xD69F_0BFF);
insn_is!(is_mrs, 0xFFF0_0000, 0xD530_0000);
insn_is!(is_msr_imm, 0xFFF8_F01F, 0xD500_401F);
insn_is!(is_msr_reg, 0xFFF0_0000, 0xD510_0000);
insn_is!(is_mrs_sp_el0, 0xFFFF_FFE0, 0xD538_4100);
insn_is!(is_dmb, 0xFFFF_F0FF, 0xD503_30BF);
insn_is!(is_dsb_base, 0xFFFF_F0FF, 0xD503_309F);
insn_is!(is_dsb_nxs, 0xFFFF_F3FF, 0xD503_323F);
insn_is!(is_isb, 0xFFFF_F0FF, 0xD503_30DF);
insn_is!(is_sb, 0xFFFF_FFFF, 0xD503_30FF);
insn_is!(is_clrex, 0xFFFF_F0FF, 0xD503_305F);
insn_is!(is_ssbb, 0xFFFF_FFFF, 0xD503_309F);
insn_is!(is_pssbb, 0xFFFF_FFFF, 0xD503_349F);
insn_is!(is_bti, 0xFFFF_FF3F, 0xD503_241F);

// Hint CRm:op2 codes (`HINT #imm`), value already shifted into bits [11:5].
pub const HINT_NOP: u32 = 0x0 << 5;
pub const HINT_XPACLRI: u32 = 0x07 << 5;
pub const HINT_PACIA_1716: u32 = 0x08 << 5;
pub const HINT_PACIB_1716: u32 = 0x0A << 5;
pub const HINT_PACIAZ: u32 = 0x18 << 5;
pub const HINT_PACIASP: u32 = 0x19 << 5;
pub const HINT_PACIBZ: u32 = 0x1A << 5;
pub const HINT_PACIBSP: u32 = 0x1B << 5;
pub const HINT_BTI: u32 = 0x20 << 5;
pub const HINT_BTIC: u32 = 0x22 << 5;
pub const HINT_BTIJ: u32 = 0x24 << 5;
pub const HINT_BTIJC: u32 = 0x26 << 5;

/// A `HINT` that single-stepping can safely treat as a NOP (PAC / BTI / NOP).
pub fn is_steppable_hint(insn: u32) -> bool {
    if !is_hint(insn) {
        return false;
    }
    matches!(
        insn & 0xFE0,
        HINT_XPACLRI
            | HINT_PACIA_1716
            | HINT_PACIB_1716
            | HINT_PACIAZ
            | HINT_PACIASP
            | HINT_PACIBZ
            | HINT_PACIBSP
            | HINT_BTI
            | HINT_BTIC
            | HINT_BTIJ
            | HINT_BTIJC
            | HINT_NOP
    )
}

/// Any branch: `b`, `bl`, `cb*`, `tb*`, `ret*`, `br*`, `blr*`, `b.cond`.
pub fn is_branch(insn: u32) -> bool {
    is_b(insn)
        || is_bl(insn)
        || is_cbz(insn)
        || is_cbnz(insn)
        || is_tbz(insn)
        || is_tbnz(insn)
        || is_ret(insn)
        || is_ret_auth(insn)
        || is_br(insn)
        || is_br_auth(insn)
        || is_blr(insn)
        || is_blr_auth(insn)
        || is_bcond(insn)
}

pub fn is_adr_adrp(insn: u32) -> bool {
    is_adr(insn) || is_adrp(insn)
}

/// PC-relative literal load (`ldr`/`ldrsw` literal, `adr`/`adrp`, `prfm` literal).
pub fn uses_literal(insn: u32) -> bool {
    is_ldr_lit(insn) || is_ldrsw_lit(insn) || is_adr_adrp(insn) || is_prfm_lit(insn)
}

/// Op/CR field of an `msr`/`mrs` (`insn[20:5]`).
pub fn extract_system_reg(insn: u32) -> u32 {
    (insn & 0x1F_FFE0) >> 5
}

/// `imm16` of a `brk` (`insn[20:5]`).
pub fn extract_brk_imm(insn: u32) -> u32 {
    (insn >> 5) & 0xFFFF
}

pub fn is_brk_imm(insn: u32, imm: u16) -> bool {
    is_brk(insn) && extract_brk_imm(insn) == u32::from(imm)
}

pub fn is_pac_or_bti(insn: u32) -> bool {
    is_paciaz(insn) || is_paciasp(insn) || is_pacibz(insn) || is_pacibsp(insn) || is_bti(insn)
}

/// Sign-extend the low `bits` of `value` to `i32` (upstream `sign_extend32`).
#[inline]
fn sign_extend32(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

/// PC-relative byte displacement of an unconditional `b`/`bl` (`imm26 << 2`).
pub fn branch_b_displacement(insn: u32) -> i32 {
    sign_extend32((insn & 0x03FF_FFFF) << 2, 28)
}

/// PC-relative byte displacement of `b.cond` / `cbz` / `cbnz` and other
/// `imm19`-form branches (`imm19 << 2`).
pub fn branch_cond_displacement(insn: u32) -> i32 {
    sign_extend32(((insn >> 5) & 0x7_FFFF) << 2, 21)
}

/// PC-relative byte displacement of `tbz` / `tbnz` (`imm14 << 2`).
pub fn branch_tbz_displacement(insn: u32) -> i32 {
    sign_extend32(((insn >> 5) & 0x3FFF) << 2, 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_encodings_classify() {
        // `ret x30` and its authenticated forms.
        assert!(is_ret(0xD65F_03C0));
        assert!(is_ret_auth(0xD65F_0BFF)); // retaa
        assert!(is_retaa(0xD65F_0BFF));
        assert!(is_retab(0xD65F_0FFF));
        // `mrs x, sp_el0` (the `current` accessor SKRoot keys off of).
        assert!(is_mrs_sp_el0(0xD538_4100)); // mrs x0, sp_el0
        assert!(is_mrs_sp_el0(0xD538_410A)); // mrs x10, sp_el0
        assert!(is_mrs(0xD538_4100));
        // PAC prologues.
        assert!(is_paciasp(0xD503_233F));
        assert!(is_pac_or_bti(0xD503_233F));
        assert!(is_bti(0xD503_241F));
        // Branches.
        assert!(is_b(0x1400_0000));
        assert!(is_bl(0x9400_0000));
        assert!(is_branch(0x1400_0000));
        assert!(is_branch(0xD65F_03C0)); // ret is a branch
        // Not-a-match sanity.
        assert!(!is_ret(0x1400_0000));
        assert!(!is_mrs_sp_el0(0xD538_0000));
    }

    #[test]
    fn field_extractors() {
        // `brk #0x4`  = 0xD4200080.
        assert!(is_brk_imm(0xD420_0080, 0x4));
        assert_eq!(extract_brk_imm(0xD420_0080), 0x4);
        // steppable hint: NOP (0xD503201F).
        assert!(is_steppable_hint(0xD503_201F));
        assert!(!is_steppable_hint(0xD65F_03C0));
    }
}
