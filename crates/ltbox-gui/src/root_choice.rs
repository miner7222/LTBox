//! Root wizard choices: family, provider, mode and build source.

use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    Magisk,
    KernelSU,
    APatch,
    Skroot,
}
impl Family {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Magisk => "family_magisk",
            Self::KernelSU => "family_ksu",
            Self::APatch => "family_apatch",
            Self::Skroot => "family_skroot",
        }
    }
    pub(crate) fn desc_key(&self) -> &'static str {
        match self {
            Self::Magisk => "family_magisk_desc",
            Self::KernelSU => "family_ksu_desc",
            Self::APatch => "family_apatch_desc",
            Self::Skroot => "family_skroot_desc",
        }
    }
    pub(crate) fn has_modes(&self) -> bool {
        matches!(self, Self::KernelSU | Self::Skroot)
    }
    pub(crate) fn providers(&self) -> &'static [Provider] {
        match self {
            Self::Magisk => &[Provider::Magisk, Provider::MagiskForks],
            Self::KernelSU => &[
                Provider::KernelSU,
                Provider::KernelSUNext,
                Provider::SukiSU,
                Provider::BakaSU,
                Provider::KernelSULocal,
            ],
            Self::APatch => &[Provider::APatch, Provider::FolkPatch],
            Self::Skroot => &[],
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provider {
    Magisk,
    MagiskForks,
    KernelSULocal,
    KernelSU,
    KernelSUNext,
    SukiSU,
    BakaSU,
    APatch,
    FolkPatch,
}
impl Provider {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Magisk => "provider_magisk",
            Self::MagiskForks | Self::KernelSULocal => "provider_magisk_forks",
            Self::KernelSU => "provider_ksu",
            Self::KernelSUNext => "provider_ksu_next",
            Self::SukiSU => "provider_sukisu",
            Self::BakaSU => "provider_bakasu",
            Self::APatch => "provider_apatch",
            Self::FolkPatch => "provider_folkpatch",
        }
    }
    pub(crate) fn desc_key(&self) -> Option<&'static str> {
        match self {
            Self::Magisk => Some("provider_magisk_desc"),
            Self::MagiskForks => Some("provider_magisk_forks_desc"),
            Self::KernelSULocal => Some("provider_ksu_local_desc"),
            Self::KernelSU => Some("provider_ksu_desc"),
            Self::KernelSUNext => Some("provider_ksu_next_desc"),
            Self::SukiSU => Some("provider_sukisu_desc"),
            Self::BakaSU => Some("provider_bakasu_desc"),
            Self::APatch => Some("provider_apatch_desc"),
            Self::FolkPatch => Some("provider_folkpatch_desc"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootMode {
    Lkm,
    Gki,
}
impl RootMode {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Lkm => "rootmode_lkm",
            Self::Gki => "rootmode_gki",
        }
    }
    pub(crate) fn desc_key(&self) -> &'static str {
        match self {
            Self::Lkm => "rootmode_lkm_desc",
            Self::Gki => "rootmode_gki_desc",
        }
    }
    pub(crate) fn icon_disabled(self, size: f32) -> Element<'static, Message> {
        let glyph = match self {
            Self::Lkm => icon::root_lkm(),
            Self::Gki => icon::root_gki(),
        };
        lucide_disabled(glyph, size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkrootFlavor {
    Lite,
    Pro,
}
impl SkrootFlavor {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Lite => "skroot_flavor_lite",
            Self::Pro => "skroot_flavor_pro",
        }
    }
    pub(crate) fn desc_key(&self) -> &'static str {
        match self {
            Self::Lite => "skroot_flavor_lite_desc",
            Self::Pro => "skroot_flavor_pro_desc",
        }
    }
    pub(crate) fn icon(self, size: f32) -> Element<'static, Message> {
        let glyph = match self {
            Self::Lite => icon::skroot_lite(),
            Self::Pro => icon::root_lkm(),
        };
        lucide_primary(glyph, size)
    }
    pub(crate) fn icon_disabled(self, size: f32) -> Element<'static, Message> {
        let glyph = match self {
            Self::Lite => icon::skroot_lite(),
            Self::Pro => icon::root_lkm(),
        };
        lucide_disabled(glyph, size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerChoice {
    Stable,
    Nightly,
}
impl VerChoice {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Stable => "verchoice_stable",
            Self::Nightly => "verchoice_nightly",
        }
    }
    pub(crate) fn desc_key(&self) -> &'static str {
        match self {
            Self::Stable => "verchoice_stable_desc",
            Self::Nightly => "verchoice_nightly_desc",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NightlySource {
    AutoDetect,
    ManualInput,
}
impl NightlySource {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::AutoDetect => "nightly_auto",
            Self::ManualInput => "nightly_manual",
        }
    }
    pub(crate) fn desc_key(&self) -> &'static str {
        match self {
            Self::AutoDetect => "nightly_auto_desc",
            Self::ManualInput => "nightly_manual_desc",
        }
    }
}
