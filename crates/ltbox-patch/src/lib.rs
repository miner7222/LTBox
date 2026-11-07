//! Patching engine — AVB, boot image, region patching.
//!
//! Uses `avbtool-rs` in-process and isolates magiskboot CLI operations in a child.
//! GUI hosts register their executable; standalone consumers install the
//! `ltbox-magiskboot` companion next to their executable.

pub mod abl_key;
mod abl_pe;
pub mod apatch;
pub mod avb;
pub mod boot;
pub mod efisp_load;
pub mod gki;
pub mod key_map;
pub mod konabess;
pub mod ksu;
pub mod magisk;
pub mod region;
pub mod rollback;
pub mod root_pipeline;
pub mod skroot;
pub(crate) mod zip_util;
