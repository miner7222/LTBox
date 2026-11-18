//! GUI language selection and translation tables.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    En,
    Ko,
    Zh,
    Ru,
    Ja,
    Fr,
}
impl Language {
    /// Name in its own script — locale-neutral.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Ko => "한국어",
            Self::Zh => "中文",
            Self::Ru => "Русский",
            Self::Ja => "日本語",
            Self::Fr => "Français",
        }
    }
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Ko => "ko",
            Self::Zh => "zh",
            Self::Ru => "ru",
            Self::Ja => "ja",
            Self::Fr => "fr",
        }
    }
    pub(crate) fn from_code(c: &str) -> Option<Self> {
        match c {
            "en" => Some(Self::En),
            "ko" => Some(Self::Ko),
            "zh" => Some(Self::Zh),
            "ru" => Some(Self::Ru),
            "ja" => Some(Self::Ja),
            "fr" => Some(Self::Fr),
            _ => None,
        }
    }
}
pub(crate) const LANGUAGES: &[Language] = &[
    Language::En,
    Language::Ko,
    Language::Zh,
    Language::Ru,
    Language::Ja,
    Language::Fr,
];

const EN_JSON: &str = include_str!("../lang/en.json");
const KO_JSON: &str = include_str!("../lang/ko.json");
const ZH_JSON: &str = include_str!("../lang/zh.json");
const RU_JSON: &str = include_str!("../lang/ru.json");
const JA_JSON: &str = include_str!("../lang/ja.json");
const FR_JSON: &str = include_str!("../lang/fr.json");

// Parsed once on first access, so switching language never reparses.
static EN_TABLE: std::sync::LazyLock<HashMap<String, String>> =
    std::sync::LazyLock::new(|| serde_json::from_str(EN_JSON).expect("en.json must parse"));
static KO_TABLE: std::sync::LazyLock<HashMap<String, String>> =
    std::sync::LazyLock::new(|| serde_json::from_str(KO_JSON).expect("ko.json must parse"));
static ZH_TABLE: std::sync::LazyLock<HashMap<String, String>> =
    std::sync::LazyLock::new(|| serde_json::from_str(ZH_JSON).expect("zh.json must parse"));
static RU_TABLE: std::sync::LazyLock<HashMap<String, String>> =
    std::sync::LazyLock::new(|| serde_json::from_str(RU_JSON).expect("ru.json must parse"));
static JA_TABLE: std::sync::LazyLock<HashMap<String, String>> =
    std::sync::LazyLock::new(|| serde_json::from_str(JA_JSON).expect("ja.json must parse"));
static FR_TABLE: std::sync::LazyLock<HashMap<String, String>> =
    std::sync::LazyLock::new(|| serde_json::from_str(FR_JSON).expect("fr.json must parse"));

/// Active translation table plus the English fallback, as `&'static` refs into
/// the `LazyLock` tables.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Translations {
    pub(crate) primary: &'static HashMap<String, String>,
    pub(crate) fallback: &'static HashMap<String, String>,
}

impl Translations {
    pub(crate) fn load(lang: Language) -> Self {
        let fallback: &'static HashMap<String, String> = &EN_TABLE;
        let primary: &'static HashMap<String, String> = match lang {
            Language::En => &EN_TABLE,
            Language::Ko => &KO_TABLE,
            Language::Zh => &ZH_TABLE,
            Language::Ru => &RU_TABLE,
            Language::Ja => &JA_TABLE,
            Language::Fr => &FR_TABLE,
        };
        Self { primary, fallback }
    }

    pub(crate) fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.primary
            .get(key)
            .or_else(|| self.fallback.get(key))
            .map(String::as_str)
            .unwrap_or(key)
    }
}

impl Default for Translations {
    fn default() -> Self {
        Self::load(Language::En)
    }
}

/// Route `ltbox_core::i18n` lookups through these tables so backend crates log
/// in the active language.
pub(crate) fn install_core_translator(lang: Language) {
    let tr = Translations::load(lang);
    ltbox_core::i18n::set_translator(move |key| tr.t(key).to_string());
}
