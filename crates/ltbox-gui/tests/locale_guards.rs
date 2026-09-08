use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use iced::advanced::graphics::text::{self as graphics_text, Paragraph as GraphicsParagraph};
use iced::advanced::text::{Alignment, LineHeight, Paragraph as _, Shaping, Text, Wrapping};
use iced::font::Weight;
use iced::{Font, Pixels, Size, alignment};

#[path = "../src/layout_constraints.rs"]
// Production consumes every budget in this module; the guard only reads the
// ones it measures, so its view of the module is legitimately partial.
#[allow(dead_code)]
mod layout_constraints;
use layout_constraints::*;

#[derive(Debug, Clone, Copy)]
enum RustTokenKind<'a> {
    Ident(&'a str),
    StringLiteral(&'a str),
    Punct(u8),
}

#[derive(Debug, Clone, Copy)]
struct RustToken<'a> {
    kind: RustTokenKind<'a>,
    line: usize,
}

#[derive(Debug, Default)]
struct TranslationSourceScan {
    rust_string_literals: BTreeSet<String>,
    called_keys: BTreeMap<String, BTreeSet<String>>,
}

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_dir() -> &'static Path {
    manifest_dir()
        .ancestors()
        .nth(2)
        .expect("ltbox-gui must live under <workspace>/crates")
}

fn load_locale(locale: &str) -> BTreeMap<String, String> {
    let path = manifest_dir().join("lang").join(format!("{locale}.json"));
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&source)
        .unwrap_or_else(|error| panic!("{} must parse: {error}", path.display()))
}

fn production_source(source: &str) -> &str {
    // Every in-source test module in this workspace is an end-of-file
    // `#[cfg(test)]` module (plus one Windows-only test module). Cutting at
    // that structural boundary keeps fallback probes and test assertions
    // from masquerading as production translation references.
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == "#[cfg(test)]" || trimmed.starts_with("#[cfg(all(test,") {
            return &source[..offset];
        }
        offset += line.len();
    }
    source
}

fn rust_tokens(source: &str) -> Vec<RustToken<'_>> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut line = 1;

    while index < bytes.len() {
        if bytes[index] == b'\n' {
            line += 1;
            index += 1;
            continue;
        }
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        if bytes[index..].starts_with(b"//") {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            index += 2;
            let mut depth = 1;
            while index < bytes.len() && depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    if bytes[index] == b'\n' {
                        line += 1;
                    }
                    index += 1;
                }
            }
            continue;
        }

        // Locale keys use ordinary literals, but accepting raw strings
        // keeps the guard accurate if a call site changes spelling style.
        if bytes[index] == b'r' {
            let mut quote = index + 1;
            while quote < bytes.len() && bytes[quote] == b'#' {
                quote += 1;
            }
            if quote < bytes.len() && bytes[quote] == b'"' {
                let hashes = quote - index - 1;
                let literal_line = line;
                let body_start = quote + 1;
                index = body_start;
                loop {
                    assert!(
                        index < bytes.len(),
                        "unterminated raw string in Rust source"
                    );
                    if bytes[index] == b'"'
                        && index + 1 + hashes <= bytes.len()
                        && bytes[index + 1..index + 1 + hashes]
                            .iter()
                            .all(|byte| *byte == b'#')
                    {
                        tokens.push(RustToken {
                            kind: RustTokenKind::StringLiteral(&source[body_start..index]),
                            line: literal_line,
                        });
                        index += 1 + hashes;
                        break;
                    }
                    if bytes[index] == b'\n' {
                        line += 1;
                    }
                    index += 1;
                }
                continue;
            }
        }

        if bytes[index] == b'"' {
            let literal_line = line;
            let body_start = index + 1;
            index = body_start;
            while index < bytes.len() && bytes[index] != b'"' {
                if bytes[index] == b'\\' {
                    index += 1;
                    assert!(index < bytes.len(), "unterminated escape in Rust string");
                }
                if bytes[index] == b'\n' {
                    line += 1;
                }
                index += 1;
            }
            assert!(index < bytes.len(), "unterminated string in Rust source");
            tokens.push(RustToken {
                kind: RustTokenKind::StringLiteral(&source[body_start..index]),
                line: literal_line,
            });
            index += 1;
            continue;
        }

        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            tokens.push(RustToken {
                kind: RustTokenKind::Ident(&source[start..index]),
                line,
            });
            continue;
        }

        if b"!()[]{},".contains(&bytes[index]) {
            tokens.push(RustToken {
                kind: RustTokenKind::Punct(bytes[index]),
                line,
            });
        }
        index += 1;
    }

    tokens
}

fn collect_called_literals<'a>(tokens: &'a [RustToken<'a>], open: usize) -> Vec<RustToken<'a>> {
    let mut literals = Vec::new();
    let mut nesting = 0;
    for token in &tokens[open + 1..] {
        match token.kind {
            RustTokenKind::Punct(b'(' | b'[' | b'{') => nesting += 1,
            RustTokenKind::Punct(b')') if nesting == 0 => break,
            RustTokenKind::Punct(b',') if nesting == 0 => break,
            RustTokenKind::Punct(b')' | b']' | b'}') => nesting -= 1,
            RustTokenKind::StringLiteral(_) => literals.push(*token),
            RustTokenKind::Ident(_) | RustTokenKind::Punct(_) => {}
        }
    }
    literals
}

fn collect_rust_sources(dir: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()))
        .map(|entry| entry.expect("workspace source entry must be readable"))
        .collect::<Vec<_>>();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn scan_production_translation_sources() -> TranslationSourceScan {
    let workspace = workspace_dir();
    let mut files = Vec::new();
    collect_rust_sources(&workspace.join("crates"), &mut files);

    let mut scan = TranslationSourceScan::default();
    for path in files {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let relative = path
            .strip_prefix(workspace)
            .expect("scanned source must be under the workspace")
            .display()
            .to_string()
            .replace('\\', "/");

        // Orphan detection intentionally follows every Rust literal in
        // the workspace, including static key arrays and assertions. The
        // undefined-call check below is narrower because test fallback
        // probes deliberately call translators with fake keys.
        for token in rust_tokens(&source) {
            if let RustTokenKind::StringLiteral(value) = token.kind {
                scan.rust_string_literals.insert(value.to_string());
            }
        }

        if path
            .strip_prefix(workspace)
            .expect("scanned source must be under the workspace")
            .components()
            .any(|component| component.as_os_str() == "tests")
        {
            continue;
        }
        let tokens = rust_tokens(production_source(&source));
        for (index, token) in tokens.iter().enumerate() {
            let RustTokenKind::Ident(name) = token.kind else {
                continue;
            };
            let (open_offset, is_translation_call) = match name {
                "tr" => (1, true),
                "tr_args" => (2, true),
                "t" if relative.starts_with("crates/ltbox-gui/src/") => (1, true),
                _ => (0, false),
            };
            if !is_translation_call {
                continue;
            }
            let Some(RustToken {
                kind: RustTokenKind::Punct(b'('),
                ..
            }) = tokens.get(index + open_offset)
            else {
                continue;
            };
            if name == "tr_args"
                && !matches!(
                    tokens.get(index + 1),
                    Some(RustToken {
                        kind: RustTokenKind::Punct(b'!'),
                        ..
                    })
                )
            {
                continue;
            }

            for literal in collect_called_literals(&tokens, index + open_offset) {
                let RustTokenKind::StringLiteral(key) = literal.kind else {
                    unreachable!();
                };
                scan.called_keys
                    .entry(key.to_string())
                    .or_default()
                    .insert(format!("{relative}:{}", literal.line));
            }
        }
    }
    scan
}

#[test]
fn locale_files_have_identical_key_sets() {
    let en = load_locale("en");
    let locales = [
        ("ko", load_locale("ko")),
        ("zh", load_locale("zh")),
        ("ru", load_locale("ru")),
        ("ja", load_locale("ja")),
    ];
    let en_keys = en.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let mut differences = Vec::new();

    for (locale, table) in &locales {
        let keys = table.keys().map(String::as_str).collect::<BTreeSet<_>>();
        let missing = en_keys.difference(&keys).copied().collect::<Vec<_>>();
        let extra = keys.difference(&en_keys).copied().collect::<Vec<_>>();
        if !missing.is_empty() {
            differences.push(format!(
                "crates/ltbox-gui/lang/{locale}.json is missing keys present in en.json: {}",
                missing.join(", ")
            ));
        }
        if !extra.is_empty() {
            differences.push(format!(
                "crates/ltbox-gui/lang/{locale}.json has keys absent from en.json: {}",
                extra.join(", ")
            ));
        }
    }

    assert!(
        differences.is_empty(),
        "locale key sets differ; add or remove the named keys so all five lang/*.json files match:\n- {}",
        differences.join("\n- ")
    );
}

#[test]
fn english_locale_keys_match_rust_sources() {
    let en = load_locale("en");
    let scan = scan_production_translation_sources();
    let orphans = en
        .keys()
        .filter(|key| !scan.rust_string_literals.contains(key.as_str()))
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut failures = Vec::new();

    if !orphans.is_empty() {
        failures.push(format!(
            "crates/ltbox-gui/lang/en.json has keys with no string-literal reference in Rust under crates/**/*.rs: {}\nRemove each orphan from all five lang/*.json files, or restore its Rust call site or key table.",
            orphans.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }

    let missing = scan
        .called_keys
        .iter()
        .filter(|(key, _)| !en.contains_key(key.as_str()))
        .map(|(key, locations)| {
            format!(
                "{key}: {}",
                locations.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        })
        .collect::<Vec<_>>();

    if !missing.is_empty() {
        failures.push(format!(
            "production t(...), tr(...), or tr_args!(...) calls reference keys absent from crates/ltbox-gui/lang/en.json:\n- {}\nAdd each key to all five lang/*.json files, or correct the named call site.",
            missing.join("\n- ")
        ));
    }

    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Codepoints one bundled subset can actually render.
///
/// Parsed by hand rather than through a font crate: this file already tokenizes
/// Rust by hand, and a guard whose whole job is to catch drift in a build input
/// should not add a build input of its own. Reads `cmap` subtable formats 4 and
/// 12, which is what `fontTools.subset` emits for these faces.
fn subset_codepoints(path: &Path) -> BTreeSet<u32> {
    let data = std::fs::read(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let be16 = |at: usize| u16::from_be_bytes([data[at], data[at + 1]]) as usize;
    let be32 = |at: usize| {
        u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize
    };

    let cmap = (0..be16(4))
        .map(|index| 12 + index * 16)
        .find(|&record| &data[record..record + 4] == b"cmap")
        .map(|record| be32(record + 8))
        .unwrap_or_else(|| panic!("{} has no cmap table", path.display()));

    let mut covered = BTreeSet::new();
    for encoding in 0..be16(cmap + 2) {
        let record = cmap + 4 + encoding * 8;
        let subtable = cmap + be32(record + 4);
        match be16(subtable) {
            4 => {
                let segments = be16(subtable + 6) / 2;
                for segment in 0..segments {
                    let end = be16(subtable + 14 + segment * 2) as u32;
                    let start = be16(subtable + 16 + segments * 2 + segment * 2) as u32;
                    // The trailing 0xFFFF segment is a required sentinel.
                    if start == 0xFFFF {
                        continue;
                    }
                    covered.extend(start..=end);
                }
            }
            12 => {
                for group in 0..be32(subtable + 12) {
                    let at = subtable + 16 + group * 12;
                    covered.extend(be32(at) as u32..=be32(at + 4) as u32);
                }
            }
            _ => {}
        }
    }
    covered
}

/// Characters the subsets deliberately do not carry.
///
/// Emoji come from the platform emoji font on every OS LTBox ships to, so a CJK
/// face carrying them would be dead weight. The list is explicit rather than an
/// "is this emoji" predicate so that adding one is a decision someone made.
const PLATFORM_FALLBACK_CHARS: &[char] = &['⛔'];

#[test]
fn every_localized_character_is_in_the_bundled_subset_for_its_locale() {
    // Mirrors `theme::font_family_for_language`: a locale renders through
    // exactly one bundled family, so coverage has to hold per locale rather
    // than across the union of all three. It also has to hold per weight — a
    // character present only in Regular renders through a fallback face
    // wherever the UI asks for medium or bold, which is visible as one glyph
    // in the wrong typeface inside an otherwise correct label.
    let families = [
        ("en", "KR"),
        ("ru", "KR"),
        ("ko", "KR"),
        ("ja", "JP"),
        ("zh", "SC"),
    ];

    let lang_dir = manifest_dir().join("lang");
    let on_disk = std::fs::read_dir(&lang_dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", lang_dir.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "json")
                .then(|| path.file_stem()?.to_str().map(str::to_owned))
                .flatten()
        })
        .collect::<BTreeSet<_>>();
    let checked = families
        .iter()
        .map(|(locale, _)| (*locale).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        on_disk, checked,
        "a lang/*.json file is not covered by this guard; add it above with the family \
         `theme::font_family_for_language` picks for it"
    );

    let mut failures = Vec::new();
    for (locale, region) in families {
        let mut used = BTreeSet::new();
        for value in load_locale(locale).values() {
            used.extend(value.chars());
        }
        used.retain(|ch| !ch.is_whitespace() && !PLATFORM_FALLBACK_CHARS.contains(ch));

        for weight in ["Regular", "Medium", "Bold"] {
            let face = format!("NotoSans{region}-{weight}.subset.otf");
            let covered = subset_codepoints(&manifest_dir().join("fonts/noto").join(&face));
            let missing = used
                .iter()
                .filter(|ch| !covered.contains(&(**ch as u32)))
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                failures.push(format!(
                    "{face} cannot render characters used by lang/{locale}.json: {}",
                    missing
                        .iter()
                        .map(|ch| format!("{ch} (U+{:04X})", **ch as u32))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "bundled font subsets are stale — a localized string uses a character they were not \
         built with, so it renders through a fallback face:\n- {}\n\nRegenerate them:\n    cd \
         crates/ltbox-gui/fonts/noto && python3 -m venv .venv && .venv/bin/pip install fonttools \
         brotli && .venv/bin/python regenerate.py",
        failures.join("\n- ")
    );
}

#[derive(Debug, Clone, Copy)]
enum CopySource {
    Key(&'static str),
    Literal {
        name: &'static str,
        value: &'static str,
    },
}

#[derive(Debug, Clone, Copy)]
struct CardSlot {
    titles: &'static [CopySource],
    descriptions: &'static [CopySource],
}

#[derive(Debug, Clone, Copy)]
struct ListRowCopy {
    label: CopySource,
    description: CopySource,
}

#[derive(Debug, Clone, Copy)]
struct ListRowSlot {
    rows: &'static [ListRowCopy],
}

#[derive(Debug, Clone, Copy)]
struct PickListSlot {
    options: &'static [CopySource],
}

#[derive(Debug, Clone, Copy)]
struct HeaderSlot {
    title: CopySource,
    action: CopySource,
}

#[derive(Debug, Clone, Copy)]
struct ActionRowSlot {
    actions: &'static [CopySource],
}

#[derive(Debug, Clone, Copy)]
enum SlotKind {
    Card(CardSlot),
    ListRow(ListRowSlot),
    PickList(PickListSlot),
    Header(HeaderSlot),
    ActionRow(ActionRowSlot),
}

#[derive(Debug, Clone, Copy)]
struct ConstrainedSlot {
    name: &'static str,
    kind: SlotKind,
}

const fn key(key: &'static str) -> CopySource {
    CopySource::Key(key)
}

const CARD_ONE_COLUMN_TITLES: &[CopySource] = &[key("verchoice_nightly")];
const CARD_ONE_COLUMN_DESCRIPTIONS: &[CopySource] = &[key("verchoice_nightly_desc")];

const CARD_TWO_COLUMN_TITLES: &[CopySource] = &[
    key("family_magisk"),
    key("family_ksu"),
    key("family_apatch"),
    key("family_skroot"),
    key("region_prc"),
    key("region_row"),
    key("flashtarget_other"),
    key("flashtarget_same"),
    key("datamode_keep"),
    key("datamode_wipe"),
    key("provider_magisk"),
    key("provider_magisk_forks"),
    key("provider_apatch"),
    key("provider_folkpatch"),
    key("provider_ksu"),
    key("provider_ksu_next"),
    key("provider_sukisu"),
    key("provider_resukisu"),
    key("rootmode_lkm"),
    key("rootmode_gki"),
    key("skroot_flavor_lite"),
    key("skroot_flavor_pro"),
    key("verchoice_stable"),
    key("verchoice_nightly"),
    key("nightly_auto"),
    key("nightly_manual"),
    key("unroottype_magisk_lkm"),
    key("unroottype_apatch_gki"),
];

const CARD_TWO_COLUMN_DESCRIPTIONS: &[CopySource] = &[
    key("family_magisk_desc"),
    key("family_ksu_desc"),
    key("family_apatch_desc"),
    key("family_skroot_desc"),
    key("region_prc_name"),
    key("region_row_name"),
    key("flashtarget_other_desc"),
    key("flashtarget_other_desc_prc"),
    key("flashtarget_other_desc_row"),
    key("flashtarget_same_desc"),
    key("flashtarget_same_desc_prc"),
    key("flashtarget_same_desc_row"),
    key("datamode_keep_desc"),
    key("datamode_wipe_desc"),
    key("provider_magisk_desc"),
    key("provider_magisk_forks_desc"),
    key("provider_apatch_desc"),
    key("provider_folkpatch_desc"),
    key("provider_ksu_desc"),
    key("provider_ksu_next_desc"),
    key("provider_sukisu_desc"),
    key("provider_resukisu_desc"),
    key("rootmode_lkm_desc"),
    key("rootmode_gki_desc"),
    key("skroot_flavor_lite_desc"),
    key("skroot_flavor_pro_desc"),
    key("verchoice_stable_desc"),
    key("verchoice_nightly_desc"),
    key("nightly_auto_desc"),
    key("nightly_manual_desc"),
    key("unroottype_magisk_lkm_desc"),
    key("unroottype_apatch_gki_desc"),
];

const SYSUPDATE_LIST_ROWS: &[ListRowCopy] = &[
    ListRowCopy {
        label: key("sysupdate_disable"),
        description: key("sysupdate_disable_desc"),
    },
    ListRowCopy {
        label: key("sysupdate_enable"),
        description: key("sysupdate_enable_desc"),
    },
    ListRowCopy {
        label: key("sysupdate_rescue"),
        description: key("sysupdate_rescue_desc"),
    },
    ListRowCopy {
        label: key("sysupdate_rescue"),
        description: key("sysupdate_rescue_req"),
    },
];

const LANGUAGE_OPTIONS: &[CopySource] = &[
    CopySource::Literal {
        name: "language_en",
        value: "English",
    },
    CopySource::Literal {
        name: "language_ko",
        value: "한국어",
    },
    CopySource::Literal {
        name: "language_zh",
        value: "中文",
    },
    CopySource::Literal {
        name: "language_ru",
        value: "Русский",
    },
    CopySource::Literal {
        name: "language_ja",
        value: "日本語",
    },
];
const THEME_OPTIONS: &[CopySource] = &[key("theme_system"), key("theme_light"), key("theme_dark")];
const THEME_SEED_OPTIONS: &[CopySource] = &[
    key("theme_seed_indigo"),
    key("theme_seed_teal"),
    key("theme_seed_rose"),
];
const DRIVER_OPTIONS: &[CopySource] = &[
    key("settings_qcom_driver_mode_userspace"),
    key("settings_qcom_driver_mode_kernel"),
];
const DIRECT_UPDATE_READY_ACTIONS: &[CopySource] = &[
    key("btn_close"),
    key("update_dialog_release_page"),
    key("update_dialog_install"),
];

// Keep each constrained widget as one row. Adding the next guard is a single
// row plus its copy-key list; the measurement and diagnostics stay shared.
const CONSTRAINED_SLOTS: &[ConstrainedSlot] = &[
    ConstrainedSlot {
        name: "wizard.selection-row.1-option",
        kind: SlotKind::Card(CardSlot {
            titles: CARD_ONE_COLUMN_TITLES,
            descriptions: CARD_ONE_COLUMN_DESCRIPTIONS,
        }),
    },
    ConstrainedSlot {
        name: "wizard.selection-row.multiple-options",
        kind: SlotKind::Card(CardSlot {
            titles: CARD_TWO_COLUMN_TITLES,
            descriptions: CARD_TWO_COLUMN_DESCRIPTIONS,
        }),
    },
    ConstrainedSlot {
        name: "system-update.action-list-row",
        kind: SlotKind::ListRow(ListRowSlot {
            rows: SYSUPDATE_LIST_ROWS,
        }),
    },
    ConstrainedSlot {
        name: "settings.language-pick-list",
        kind: SlotKind::PickList(PickListSlot {
            options: LANGUAGE_OPTIONS,
        }),
    },
    ConstrainedSlot {
        name: "settings.theme-pick-list",
        kind: SlotKind::PickList(PickListSlot {
            options: THEME_OPTIONS,
        }),
    },
    ConstrainedSlot {
        name: "settings.theme-seed-pick-list",
        kind: SlotKind::PickList(PickListSlot {
            options: THEME_SEED_OPTIONS,
        }),
    },
    ConstrainedSlot {
        name: "settings.qcom-driver-pick-list",
        kind: SlotKind::PickList(PickListSlot {
            options: DRIVER_OPTIONS,
        }),
    },
    ConstrainedSlot {
        // The disabled branch is a plain container today. Reserving the same
        // arrow allowance as its enabled sibling keeps both fixed-width
        // branches safe if it becomes a disabled pick list later.
        name: "settings.qcom-driver-disabled-pick-list",
        kind: SlotKind::PickList(PickListSlot {
            options: DRIVER_OPTIONS,
        }),
    },
    ConstrainedSlot {
        name: "advanced.region-target-popup-header",
        kind: SlotKind::Header(HeaderSlot {
            title: key("popup_select_region_target"),
            action: key("btn_cancel"),
        }),
    },
    ConstrainedSlot {
        name: "self-update.ready-action-row",
        kind: SlotKind::ActionRow(ActionRowSlot {
            actions: DIRECT_UPDATE_READY_ACTIONS,
        }),
    },
];

fn load_bundled_locale_fonts() {
    static LOAD: std::sync::Once = std::sync::Once::new();
    LOAD.call_once(|| {
        let mut font_system = graphics_text::font_system()
            .write()
            .expect("write Iced font system");
        for bytes in [
            include_bytes!("../fonts/noto/NotoSansKR-Regular.subset.otf").as_slice(),
            include_bytes!("../fonts/noto/NotoSansKR-Medium.subset.otf").as_slice(),
            include_bytes!("../fonts/noto/NotoSansJP-Regular.subset.otf").as_slice(),
            include_bytes!("../fonts/noto/NotoSansJP-Medium.subset.otf").as_slice(),
            include_bytes!("../fonts/noto/NotoSansSC-Regular.subset.otf").as_slice(),
            include_bytes!("../fonts/noto/NotoSansSC-Medium.subset.otf").as_slice(),
        ] {
            font_system.load_font(std::borrow::Cow::Borrowed(bytes));
        }
    });
}

fn locale_font(locale: &str, weight: Weight) -> Font {
    let family = match locale {
        "ja" => "Noto Sans JP",
        "zh" => "Noto Sans SC",
        _ => "Noto Sans KR",
    };
    Font {
        weight,
        ..Font::with_name(family)
    }
}

fn measure_text(locale: &str, value: &str, size: f32, weight: Weight, width: Option<f32>) -> Size {
    let paragraph = GraphicsParagraph::with_text(Text {
        content: value,
        bounds: Size::new(width.unwrap_or(100_000.0), 100_000.0),
        size: Pixels(size),
        line_height: LineHeight::default(),
        font: locale_font(locale, weight),
        align_x: Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: if width.is_some() {
            Wrapping::WordOrGlyph
        } else {
            Wrapping::None
        },
    });
    paragraph.min_bounds()
}

fn localized_copy(table: &BTreeMap<String, String>, source: CopySource) -> (&'static str, &str) {
    match source {
        CopySource::Key(key) => (
            key,
            table
                .get(key)
                .unwrap_or_else(|| panic!("constrained slot references missing locale key {key}")),
        ),
        CopySource::Literal { name, value } => (name, value),
    }
}

fn overflow_message(
    slot: &str,
    key: &str,
    locale: &str,
    measured: String,
    limit: String,
) -> String {
    format!(
        "slot={slot} key={key} locale={locale} measured={measured} limit={limit}; shorten the copy or change the slot's budget"
    )
}

#[test]
fn bundled_locale_copy_fits_constrained_layout_slots() {
    load_bundled_locale_fonts();
    let locales = ["en", "ko", "zh", "ru", "ja"];
    let mut failures = Vec::new();

    for slot in CONSTRAINED_SLOTS {
        for locale in locales {
            let table = load_locale(locale);
            match slot.kind {
                SlotKind::Card(card) => {
                    let row_width = WIZARD_LIST_MAX_WIDTH - 2.0 * WIZARD_STEP_HORIZONTAL_PADDING;
                    let text_width_limit = row_width
                        - 2.0 * WIZARD_LIST_HORIZONTAL_PADDING
                        - WIZARD_LIST_ICON_SIZE
                        - WIZARD_LIST_ICON_GAP;
                    let text_height_limit =
                        WIZARD_LIST_CARD_HEIGHT - 2.0 * WIZARD_LIST_VERTICAL_PADDING;
                    let one_label_line = LineHeight::default()
                        .to_absolute(Pixels(WIZARD_LIST_LABEL_SIZE))
                        .0;
                    let one_description_line = LineHeight::default()
                        .to_absolute(Pixels(WIZARD_LIST_DESC_SIZE))
                        .0;
                    let title_height_limit =
                        text_height_limit - WIZARD_LIST_TEXT_GAP - one_description_line;
                    let description_height_limit =
                        text_height_limit - WIZARD_LIST_TEXT_GAP - one_label_line;
                    let mut max_title = Size::ZERO;
                    let mut max_description = Size::ZERO;

                    for source in card.titles {
                        let (key, value) = localized_copy(&table, *source);
                        let measured = measure_text(
                            locale,
                            value,
                            WIZARD_LIST_LABEL_SIZE,
                            Weight::Medium,
                            Some(text_width_limit),
                        );
                        if measured.height > max_title.height {
                            max_title = measured;
                        }
                        if measured.width > text_width_limit + f32::EPSILON
                            || measured.height > title_height_limit + f32::EPSILON
                        {
                            failures.push(overflow_message(
                                slot.name,
                                key,
                                locale,
                                format!("{:.1}x{:.1}px", measured.width, measured.height),
                                format!("{text_width_limit:.1}x{title_height_limit:.1}px"),
                            ));
                        }
                    }

                    for source in card.descriptions {
                        let (key, value) = localized_copy(&table, *source);
                        let measured = measure_text(
                            locale,
                            value,
                            WIZARD_LIST_DESC_SIZE,
                            Weight::Normal,
                            Some(text_width_limit),
                        );
                        if measured.height > max_description.height {
                            max_description = measured;
                        }
                        if measured.width > text_width_limit + f32::EPSILON
                            || measured.height > description_height_limit + f32::EPSILON
                        {
                            failures.push(overflow_message(
                                slot.name,
                                key,
                                locale,
                                format!("{:.1}x{:.1}px", measured.width, measured.height),
                                format!("{text_width_limit:.1}x{description_height_limit:.1}px"),
                            ));
                        }
                    }

                    println!(
                        "HEADROOM slot={} locale={} label={:.1}x{:.1}/{:.1}x{:.1}px description={:.1}x{:.1}/{:.1}x{:.1}px row={:.1}x{:.1}px",
                        slot.name,
                        locale,
                        max_title.width,
                        max_title.height,
                        text_width_limit,
                        title_height_limit,
                        max_description.width,
                        max_description.height,
                        text_width_limit,
                        description_height_limit,
                        row_width,
                        WIZARD_LIST_CARD_HEIGHT,
                    );
                }
                SlotKind::ListRow(list_row) => {
                    let row_width = WIZARD_LIST_MAX_WIDTH - 2.0 * WIZARD_STEP_HORIZONTAL_PADDING;
                    let text_width_limit = row_width
                        - 2.0 * WIZARD_LIST_HORIZONTAL_PADDING
                        - WIZARD_LIST_GLYPH_ICON_SIZE
                        - WIZARD_LIST_ICON_GAP;
                    let text_height_limit =
                        WIZARD_LIST_CARD_HEIGHT - 2.0 * WIZARD_LIST_VERTICAL_PADDING;
                    let mut widest_label = 0.0_f32;
                    let mut widest_description = 0.0_f32;
                    let mut tallest_stack = 0.0_f32;

                    for row in list_row.rows {
                        let (label_key, label) = localized_copy(&table, row.label);
                        let (description_key, description) =
                            localized_copy(&table, row.description);
                        let label_size = measure_text(
                            locale,
                            label,
                            WIZARD_LIST_LABEL_SIZE,
                            Weight::Normal,
                            Some(text_width_limit),
                        );
                        let description_size = measure_text(
                            locale,
                            description,
                            WIZARD_LIST_DESC_SIZE,
                            Weight::Normal,
                            Some(text_width_limit),
                        );
                        let stack_height =
                            label_size.height + WIZARD_LIST_TEXT_GAP + description_size.height;
                        widest_label = widest_label.max(label_size.width);
                        widest_description = widest_description.max(description_size.width);
                        tallest_stack = tallest_stack.max(stack_height);

                        if label_size.width > text_width_limit + f32::EPSILON
                            || description_size.width > text_width_limit + f32::EPSILON
                            || stack_height > text_height_limit + f32::EPSILON
                        {
                            failures.push(overflow_message(
                                slot.name,
                                &format!("{label_key}+{description_key}"),
                                locale,
                                format!(
                                    "label={:.1}x{:.1}px description={:.1}x{:.1}px stack={stack_height:.1}px",
                                    label_size.width,
                                    label_size.height,
                                    description_size.width,
                                    description_size.height,
                                ),
                                format!("{text_width_limit:.1}x{text_height_limit:.1}px"),
                            ));
                        }
                    }

                    println!(
                        "HEADROOM slot={} locale={} label={:.1}px description={:.1}px width-limit={:.1}px stack={:.1}/{:.1}px row={:.1}x{:.1}px",
                        slot.name,
                        locale,
                        widest_label,
                        widest_description,
                        text_width_limit,
                        tallest_stack,
                        text_height_limit,
                        row_width,
                        WIZARD_LIST_CARD_HEIGHT,
                    );
                }
                SlotKind::PickList(pick_list) => {
                    let reserved = SETTINGS_PICK_LIST_TEXT_SIZE
                        + M3_FIELD_PADDING.left
                        + M3_FIELD_PADDING.left
                        + M3_FIELD_PADDING.right;
                    let mut widest = 0.0_f32;
                    for source in pick_list.options {
                        let (key, value) = localized_copy(&table, *source);
                        let text_width = measure_text(
                            locale,
                            value,
                            SETTINGS_PICK_LIST_TEXT_SIZE,
                            Weight::Normal,
                            None,
                        )
                        .width;
                        let measured = text_width + reserved;
                        widest = widest.max(measured);
                        if measured > SETTINGS_PICK_LIST_WIDTH + f32::EPSILON {
                            failures.push(overflow_message(
                                slot.name,
                                key,
                                locale,
                                format!("{measured:.1}px"),
                                format!("{SETTINGS_PICK_LIST_WIDTH:.1}px"),
                            ));
                        }
                    }
                    println!(
                        "HEADROOM slot={} locale={} measured={:.1}px budget={:.1}px",
                        slot.name, locale, widest, SETTINGS_PICK_LIST_WIDTH,
                    );
                }
                SlotKind::Header(header) => {
                    let (title_key, title) = localized_copy(&table, header.title);
                    let (action_key, action) = localized_copy(&table, header.action);
                    let title_width = measure_text(
                        locale,
                        title,
                        REGION_TARGET_POPUP_TITLE_SIZE,
                        Weight::Normal,
                        None,
                    )
                    .width;
                    let action_width = measure_text(
                        locale,
                        action,
                        REGION_TARGET_POPUP_ACTION_SIZE,
                        Weight::Normal,
                        None,
                    )
                    .width
                        + 2.0 * M3_BUTTON_H_PADDING;
                    let measured = title_width + action_width;
                    let limit = REGION_TARGET_POPUP_WIDTH - 2.0 * DIALOG_H_PADDING;
                    if measured > limit + f32::EPSILON {
                        failures.push(overflow_message(
                            slot.name,
                            &format!("{title_key}+{action_key}"),
                            locale,
                            format!("{measured:.1}px"),
                            format!("{limit:.1}px"),
                        ));
                    }
                    println!(
                        "HEADROOM slot={} locale={} measured={:.1}px budget={:.1}px",
                        slot.name, locale, measured, limit,
                    );
                }
                SlotKind::ActionRow(action_row) => {
                    // The production row has one leading fill spacer, so it
                    // contributes one more inter-child gap than the buttons
                    // alone would.
                    let mut measured =
                        DIRECT_UPDATE_DIALOG_ACTION_SPACING * action_row.actions.len() as f32;
                    let mut keys = Vec::with_capacity(action_row.actions.len());
                    for source in action_row.actions {
                        let (key, value) = localized_copy(&table, *source);
                        keys.push(key);
                        measured += measure_text(
                            locale,
                            value,
                            DIRECT_UPDATE_DIALOG_ACTION_SIZE,
                            Weight::Normal,
                            None,
                        )
                        .width
                            + 2.0 * M3_BUTTON_H_PADDING;
                    }
                    let limit = DIRECT_UPDATE_DIALOG_WIDTH - 2.0 * DIALOG_H_PADDING;
                    if measured > limit + f32::EPSILON {
                        failures.push(overflow_message(
                            slot.name,
                            &keys.join("+"),
                            locale,
                            format!("{measured:.1}px"),
                            format!("{limit:.1}px"),
                        ));
                    }
                    println!(
                        "HEADROOM slot={} locale={} measured={:.1}px budget={:.1}px",
                        slot.name, locale, measured, limit,
                    );
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "bundled localized copy outgrew a constrained layout slot:\n- {}",
        failures.join("\n- ")
    );
}

#[test]
fn compact_flash_and_root_step_labels_fit_default_content_width() {
    load_bundled_locale_fonts();
    const DEFAULT_CONTENT_WIDTH: f32 = 756.0;
    const COMPACT_TRACK_WIDTH: f32 = 110.0;
    const COMPACT_HORIZONTAL_PADDING: f32 = 48.0;
    const COMPACT_ITEM_GAPS: f32 = 20.0;
    const FLASH_STEP_KEYS: &[&str] = &[
        "flash_step_region",
        "flash_step_target",
        "flash_step_data",
        "flash_step_folder",
        "flash_step_confirm",
        "flash_step_flash",
    ];
    const ROOT_STEP_KEYS: &[&str] = &[
        "root_step_type",
        "root_step_mode",
        "root_step_provider",
        "root_step_version",
        "edl_loader_label",
        "root_step_confirm",
        "root_step_flash",
    ];

    let label_budget = DEFAULT_CONTENT_WIDTH
        - COMPACT_TRACK_WIDTH
        - COMPACT_HORIZONTAL_PADDING
        - COMPACT_ITEM_GAPS;
    for (flow, keys) in [("flash", FLASH_STEP_KEYS), ("root", ROOT_STEP_KEYS)] {
        let total = keys.len();
        for locale in ["en", "ko", "zh", "ru", "ja"] {
            let table = load_locale(locale);
            let mut widest = ("", 0.0_f32, String::new());
            for (index, key) in keys.iter().enumerate() {
                let label = table
                    .get(*key)
                    .unwrap_or_else(|| panic!("missing compact step label {key} for {locale}"));
                let indicator = format!("{} / {total} · {label}", index + 1);
                let width = measure_text(locale, &indicator, 12.0, Weight::Medium, None).width;
                if width > widest.1 {
                    widest = (key, width, indicator.clone());
                }
                assert!(
                    width <= label_budget + f32::EPSILON,
                    "flow={flow} key={key} locale={locale} indicator={indicator:?} measured={width:.1}px budget={label_budget:.1}px"
                );
            }
            println!(
                "HEADROOM compact-step flow={flow} locale={locale} key={} indicator={:?} measured={:.1}px budget={label_budget:.1}px",
                widest.0, widest.2, widest.1,
            );
        }
    }
}
