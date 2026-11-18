//! Country-code model data for the Flash wizard.

/// Country-code state for the Flash wizard. A sum type so an `Option<String>` +
/// `bool` pair cannot encode invalid combinations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum CountryAction {
    /// Popup hasn't been answered yet.
    #[default]
    Unset,
    /// User picked "Do not change" — devinfo/persist stays put.
    Skip,
    /// User picked a concrete target code; exec runs the patch.
    Set(String),
}

impl CountryAction {
    pub(crate) fn target(&self) -> Option<&str> {
        match self {
            Self::Set(s) => Some(s.as_str()),
            _ => None,
        }
    }
    pub(crate) fn is_skipped(&self) -> bool {
        matches!(self, Self::Skip)
    }
}

pub(crate) struct CountryEntry {
    pub(crate) code: &'static str,
    pub(crate) name: &'static str,
}

pub(crate) const COUNTRY_CODES: &[CountryEntry] = &[
    CountryEntry {
        code: "AE",
        name: "United Arab Emirates",
    },
    CountryEntry {
        code: "AM",
        name: "Armenia",
    },
    CountryEntry {
        code: "AR",
        name: "Argentina",
    },
    CountryEntry {
        code: "AT",
        name: "Austria",
    },
    CountryEntry {
        code: "AU",
        name: "Australia",
    },
    CountryEntry {
        code: "AZ",
        name: "Azerbaijan",
    },
    CountryEntry {
        code: "BE",
        name: "Belgium",
    },
    CountryEntry {
        code: "BG",
        name: "Bulgaria",
    },
    CountryEntry {
        code: "BH",
        name: "Bahrain",
    },
    CountryEntry {
        code: "BR",
        name: "Brazil",
    },
    CountryEntry {
        code: "CA",
        name: "Canada",
    },
    CountryEntry {
        code: "CH",
        name: "Switzerland",
    },
    CountryEntry {
        code: "CL",
        name: "Chile",
    },
    CountryEntry {
        code: "CN",
        name: "China",
    },
    CountryEntry {
        code: "CO",
        name: "Colombia",
    },
    CountryEntry {
        code: "CR",
        name: "Costa Rica",
    },
    CountryEntry {
        code: "CY",
        name: "Cyprus",
    },
    CountryEntry {
        code: "CZ",
        name: "Czech Republic",
    },
    CountryEntry {
        code: "DE",
        name: "Germany",
    },
    CountryEntry {
        code: "DK",
        name: "Denmark",
    },
    CountryEntry {
        code: "EC",
        name: "Ecuador",
    },
    CountryEntry {
        code: "EE",
        name: "Estonia",
    },
    CountryEntry {
        code: "EG",
        name: "Egypt",
    },
    CountryEntry {
        code: "ES",
        name: "Spain",
    },
    CountryEntry {
        code: "FI",
        name: "Finland",
    },
    CountryEntry {
        code: "FR",
        name: "France",
    },
    CountryEntry {
        code: "GB",
        name: "United Kingdom",
    },
    CountryEntry {
        code: "GE",
        name: "Georgia",
    },
    CountryEntry {
        code: "GH",
        name: "Ghana",
    },
    CountryEntry {
        code: "GR",
        name: "Greece",
    },
    CountryEntry {
        code: "GT",
        name: "Guatemala",
    },
    CountryEntry {
        code: "HK",
        name: "Hong Kong",
    },
    CountryEntry {
        code: "HR",
        name: "Croatia",
    },
    CountryEntry {
        code: "HU",
        name: "Hungary",
    },
    CountryEntry {
        code: "ID",
        name: "Indonesia",
    },
    CountryEntry {
        code: "IL",
        name: "Israel",
    },
    CountryEntry {
        code: "IN",
        name: "India",
    },
    CountryEntry {
        code: "IS",
        name: "Iceland",
    },
    CountryEntry {
        code: "IT",
        name: "Italy",
    },
    CountryEntry {
        code: "JO",
        name: "Jordan",
    },
    CountryEntry {
        code: "JP",
        name: "Japan",
    },
    CountryEntry {
        code: "KE",
        name: "Kenya",
    },
    CountryEntry {
        code: "KG",
        name: "Kyrgyzstan",
    },
    CountryEntry {
        code: "KR",
        name: "Korea",
    },
    CountryEntry {
        code: "KW",
        name: "Kuwait",
    },
    CountryEntry {
        code: "KZ",
        name: "Kazakhstan",
    },
    CountryEntry {
        code: "LB",
        name: "Lebanon",
    },
    CountryEntry {
        code: "LT",
        name: "Lithuania",
    },
    CountryEntry {
        code: "LV",
        name: "Latvia",
    },
    CountryEntry {
        code: "MA",
        name: "Morocco",
    },
    CountryEntry {
        code: "MD",
        name: "Moldova",
    },
    CountryEntry {
        code: "MX",
        name: "Mexico",
    },
    CountryEntry {
        code: "MY",
        name: "Malaysia",
    },
    CountryEntry {
        code: "MZ",
        name: "Mozambique",
    },
    CountryEntry {
        code: "NG",
        name: "Nigeria",
    },
    CountryEntry {
        code: "NL",
        name: "Netherlands",
    },
    CountryEntry {
        code: "NO",
        name: "Norway",
    },
    CountryEntry {
        code: "NZ",
        name: "New Zealand",
    },
    CountryEntry {
        code: "OM",
        name: "Oman",
    },
    CountryEntry {
        code: "PA",
        name: "Panama",
    },
    CountryEntry {
        code: "PE",
        name: "Peru",
    },
    CountryEntry {
        code: "PH",
        name: "Philippines",
    },
    CountryEntry {
        code: "PK",
        name: "Pakistan",
    },
    CountryEntry {
        code: "PL",
        name: "Poland",
    },
    CountryEntry {
        code: "PT",
        name: "Portugal",
    },
    CountryEntry {
        code: "QA",
        name: "Qatar",
    },
    CountryEntry {
        code: "RO",
        name: "Romania",
    },
    CountryEntry {
        code: "RS",
        name: "Serbia",
    },
    CountryEntry {
        code: "RU",
        name: "Russia",
    },
    CountryEntry {
        code: "SA",
        name: "Saudi Arabia",
    },
    CountryEntry {
        code: "SE",
        name: "Sweden",
    },
    CountryEntry {
        code: "SG",
        name: "Singapore",
    },
    CountryEntry {
        code: "SI",
        name: "Slovenia",
    },
    CountryEntry {
        code: "SK",
        name: "Slovakia",
    },
    CountryEntry {
        code: "SV",
        name: "El Salvador",
    },
    CountryEntry {
        code: "TH",
        name: "Thailand",
    },
    CountryEntry {
        code: "TJ",
        name: "Tajikistan",
    },
    CountryEntry {
        code: "TN",
        name: "Tunisia",
    },
    CountryEntry {
        code: "TR",
        name: "Turkey",
    },
    CountryEntry {
        code: "TW",
        name: "Taiwan",
    },
    CountryEntry {
        code: "TZ",
        name: "Tanzania",
    },
    CountryEntry {
        code: "UA",
        name: "Ukraine",
    },
    CountryEntry {
        code: "UG",
        name: "Uganda",
    },
    CountryEntry {
        code: "US",
        name: "United States",
    },
    CountryEntry {
        code: "UY",
        name: "Uruguay",
    },
    CountryEntry {
        code: "UZ",
        name: "Uzbekistan",
    },
    CountryEntry {
        code: "VE",
        name: "Venezuela",
    },
    CountryEntry {
        code: "VN",
        name: "Vietnam",
    },
    CountryEntry {
        code: "ZA",
        name: "South Africa",
    },
];
