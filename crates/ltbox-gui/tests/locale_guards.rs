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

/// Every locale shipped in `lang/`, English first. Discovered from the
/// directory so a new `lang/*.json` is checked automatically.
fn locale_codes() -> Vec<String> {
    let lang_dir = manifest_dir().join("lang");
    let mut codes = std::fs::read_dir(&lang_dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", lang_dir.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "json").then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect::<Vec<_>>();
    codes.sort_by_key(|code| (code != "en", code.clone()));
    assert_eq!(
        codes.first().map(String::as_str),
        Some("en"),
        "lang/en.json must exist"
    );
    codes
}

/// Every shipped locale except English.
fn translated_locale_codes() -> Vec<String> {
    locale_codes().into_iter().skip(1).collect()
}

fn template_parameters(template: &str) -> BTreeSet<String> {
    template
        .split('{')
        .skip(1)
        .filter_map(|tail| tail.split_once('}'))
        .map(|(name, _)| name.to_owned())
        .collect()
}

#[test]
fn log_template_parameters_match_in_every_locale() {
    let english = load_locale("en");
    for locale in translated_locale_codes() {
        let table = load_locale(&locale);
        for (key, template) in &english {
            if key.starts_with("live_") || key.starts_with("log_") {
                assert_eq!(
                    template_parameters(template),
                    template_parameters(&table[key]),
                    "{locale}/{key}: localized log must preserve every named parameter"
                );
            }
        }
    }
}

#[test]
fn literal_log_macro_calls_supply_the_complete_template() {
    fn inspect(stream: proc_macro2::TokenStream, table: &BTreeMap<String, String>, file: &Path) {
        use proc_macro2::TokenTree;
        let tokens: Vec<_> = stream.into_iter().collect();
        for (index, token) in tokens.iter().enumerate() {
            if let TokenTree::Group(group) = token {
                inspect(group.stream(), table, file);
            }
            if !matches!(token, TokenTree::Ident(name) if name == "tr_args") {
                continue;
            }
            let (Some(TokenTree::Punct(bang)), Some(TokenTree::Group(args))) =
                (tokens.get(index + 1), tokens.get(index + 2))
            else {
                continue;
            };
            if bang.as_char() != '!' {
                continue;
            }
            let args: Vec<_> = args.stream().into_iter().collect();
            let Some(TokenTree::Literal(key)) = args.first() else {
                continue;
            };
            let Ok(key) = syn::parse_str::<syn::LitStr>(&key.to_string()) else {
                continue;
            };
            let key = key.value();
            if !(key.starts_with("live_") || key.starts_with("log_")) {
                continue;
            }
            let Some(template) = table.get(&key) else {
                continue;
            };
            let supplied: BTreeSet<_> = args
                .windows(3)
                .filter_map(|window| match window {
                    [
                        TokenTree::Punct(comma),
                        TokenTree::Ident(name),
                        TokenTree::Punct(equal),
                    ] if comma.as_char() == ',' && equal.as_char() == '=' => Some(name.to_string()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                template_parameters(template),
                supplied,
                "{}: {key}",
                file.display()
            );
        }
    }
    let mut files = Vec::new();
    collect_rust_sources(&workspace_dir().join("crates"), &mut files);
    let table = load_locale("en");
    for file in files {
        if !file.components().any(|part| part.as_os_str() == "src") {
            continue;
        }
        let source = production_source(&std::fs::read_to_string(&file).unwrap());
        inspect(source.parse().unwrap(), &table, &file);
    }
}

fn production_source(source: &str) -> String {
    use syn::{spanned::Spanned, visit::Visit};
    struct TestItems {
        ranges: Vec<proc_macro2::Span>,
    }
    fn requires_test(meta: &syn::Meta) -> bool {
        if meta.path().is_ident("test") {
            return true;
        }
        let syn::Meta::List(list) = meta else {
            return false;
        };
        let Ok(items) = list.parse_args_with(
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
        ) else {
            return false;
        };
        if list.path.is_ident("all") {
            items.iter().any(requires_test)
        } else if list.path.is_ident("any") {
            !items.is_empty() && items.iter().all(requires_test)
        } else {
            false
        }
    }
    impl<'ast> Visit<'ast> for TestItems {
        fn visit_item(&mut self, item: &'ast syn::Item) {
            let attrs: &[syn::Attribute] = match item {
                syn::Item::Fn(v) => &v.attrs,
                syn::Item::Mod(v) => &v.attrs,
                syn::Item::Const(v) => &v.attrs,
                syn::Item::Static(v) => &v.attrs,
                syn::Item::Impl(v) => &v.attrs,
                syn::Item::Struct(v) => &v.attrs,
                syn::Item::Enum(v) => &v.attrs,
                syn::Item::Trait(v) => &v.attrs,
                syn::Item::Type(v) => &v.attrs,
                syn::Item::Use(v) => &v.attrs,
                syn::Item::Macro(v) => &v.attrs,
                syn::Item::ForeignMod(v) => &v.attrs,
                syn::Item::Union(v) => &v.attrs,
                _ => &[],
            };
            if attrs.iter().any(|a| {
                a.path().is_ident("cfg")
                    && a.parse_args::<syn::Meta>().is_ok_and(|m| requires_test(&m))
            }) {
                self.ranges.push(item.span());
            } else {
                syn::visit::visit_item(self, item);
            }
        }
    }
    let parsed = syn::parse_file(source).expect("Rust source must parse for locale checks");
    let mut visitor = TestItems { ranges: Vec::new() };
    visitor.visit_file(&parsed);
    let mut line_starts = vec![0];
    line_starts.extend(source.match_indices('\n').map(|(index, _)| index + 1));
    let mut bytes = source.as_bytes().to_vec();
    for span in visitor.ranges {
        let start = span.start();
        let end = span.end();
        let start = line_starts[start.line - 1] + start.column;
        let end = line_starts[end.line - 1] + end.column;
        for byte in &mut bytes[start..end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(bytes).expect("masked source remains UTF-8")
}

#[test]
fn production_scan_keeps_code_after_test_modules() {
    let source = r#"#[cfg(test)]
#[path = "external.rs"] mod tests;
fn production() { tr("real_key"); }
#[cfg(all(test, windows))] mod windows_tests { fn sample() { tr("fake_key"); } }
fn later() { tr("later_key"); }
"#;
    let filtered = production_source(source);
    assert!(filtered.contains("real_key"));
    assert!(filtered.contains("later_key"));
    assert!(!filtered.contains("fake_key"));
    assert!(!filtered.contains("external.rs"));
    assert_eq!(filtered.lines().count(), source.lines().count());
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

        // Test-only sources: integration tests, and `*_tests.rs` files that a
        // `#[cfg(test)] #[path = "..."] mod` pulls out of their parent.
        if path
            .strip_prefix(workspace)
            .expect("scanned source must be under the workspace")
            .components()
            .any(|component| component.as_os_str() == "tests")
            || path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_tests.rs"))
        {
            continue;
        }
        let production = production_source(&source);
        let tokens = rust_tokens(&production);
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
    let locales = translated_locale_codes()
        .into_iter()
        .map(|locale| {
            let table = load_locale(&locale);
            (locale, table)
        })
        .collect::<Vec<_>>();
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
        "locale key sets differ; add or remove the named keys so every lang/*.json file matches:\n- {}",
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
            "crates/ltbox-gui/lang/en.json has keys with no string-literal reference in Rust under crates/**/*.rs: {}\nRemove each orphan from every lang/*.json file, or restore its Rust call site or key table.",
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
            "production t(...), tr(...), or tr_args!(...) calls reference keys absent from crates/ltbox-gui/lang/en.json:\n- {}\nAdd each key to every lang/*.json file, or correct the named call site.",
            missing.join("\n- ")
        ));
    }

    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Codepoints one bundled subset can actually render.
///
/// Parsed by hand to avoid adding a font-crate dependency to a drift guard.
/// Reads `cmap` subtable formats 4 and 12, which `fontTools.subset` emits for
/// these faces.
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
/// Emoji come from the platform emoji font, so the CJK faces do not carry them.
/// The list is explicit so that adding one is a deliberate decision.
const PLATFORM_FALLBACK_CHARS: &[char] = &['⛔'];

#[test]
fn every_localized_character_is_in_the_bundled_subset_for_its_locale() {
    // Mirrors `theme::font_family_for_language`: a locale renders through one
    // bundled family, so coverage must hold per locale and per weight; a
    // character only in Regular falls back to another face at medium or bold.
    let mut failures = Vec::new();
    for locale in locale_codes() {
        let region = locale_family(&locale).trim_start_matches("Noto Sans ");
        let mut used = BTreeSet::new();
        for value in load_locale(&locale).values() {
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

/// One wizard flow's app bar: the flow title, whether the bar also carries the
/// "Writes to Device" chip, and every subtitle key the flow shows beside them.
#[derive(Debug, Clone, Copy)]
struct AppBarFlow {
    title: &'static str,
    writes_to_device: bool,
    subtitles: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct AppBarSubtitleSlot {
    flows: &'static [AppBarFlow],
}

#[derive(Debug, Clone, Copy)]
enum SlotKind {
    Card(CardSlot),
    ListRow(ListRowSlot),
    PickList(PickListSlot),
    Header(HeaderSlot),
    ActionRow(ActionRowSlot),
    AppBarSubtitle(AppBarSubtitleSlot),
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
    key("provider_bakasu"),
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
    key("provider_bakasu_desc"),
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
const DIRECT_UPDATE_READY_ACTIONS: &[CopySource] =
    &[key("btn_close"), key("update_dialog_install")];
/// The device-choice dialog uses the same 520 px card as the update dialog.
const DEVICE_CHOICE_DIALOG_ACTIONS: &[CopySource] = &[key("btn_cancel"), key("btn_use_device")];

/// `loader_picker_subtitle` picks one of these on every loader step.
const LOADER_SUBTITLES: [&str; 3] = [
    "loader_picker_subtitle_manifest",
    "loader_picker_subtitle_standard",
    "loader_picker_subtitle_unknown",
];
const EXEC_SUBTITLE: &str = "exec_executing_subtitle";

const fn loader_flow(title: &'static str, writes_to_device: bool) -> AppBarFlow {
    AppBarFlow {
        title,
        writes_to_device,
        subtitles: &[
            LOADER_SUBTITLES[0],
            LOADER_SUBTITLES[1],
            LOADER_SUBTITLES[2],
            EXEC_SUBTITLE,
        ],
    }
}

// Each flow's title is the one its view passes to `wizard_action_bar*`; the
// subtitles are the ticket-listed app-bar keys for that flow. Some adv_src_*
// keys are listed though their step shows the loader subtitle instead. Every
// flow also lists its `AdvAction::desc_key()`, the Advanced confirm subtitle.
const APP_BAR_FLOWS: &[AppBarFlow] = &[
    AppBarFlow {
        title: "nav_flash",
        writes_to_device: false,
        subtitles: &[
            "flash_region_subtitle",
            "flash_target_subtitle",
            "flash_data_subtitle",
            "flash_folder_desc",
            "flash_bootloader_efisp_desc",
            "flash_bootloader_pick_subtitle",
            EXEC_SUBTITLE,
        ],
    },
    AppBarFlow {
        title: "nav_root",
        writes_to_device: false,
        subtitles: &[EXEC_SUBTITLE],
    },
    loader_flow("nav_sysupdate", false),
    AppBarFlow {
        title: "nav_unroot",
        writes_to_device: false,
        subtitles: &[
            LOADER_SUBTITLES[0],
            LOADER_SUBTITLES[1],
            LOADER_SUBTITLES[2],
            "unroot_folderdesc_boot",
            "unroot_folderdesc_boot_vbmeta",
            "unroot_folderdesc_init_boot",
            "unroot_folderdesc_init_boot_vbmeta",
            EXEC_SUBTITLE,
        ],
    },
    loader_flow("nav_konabess", false),
    AppBarFlow {
        title: "adv_dump_partitions",
        writes_to_device: false,
        subtitles: &[
            "adv_dump_partitions_desc",
            LOADER_SUBTITLES[0],
            LOADER_SUBTITLES[1],
            LOADER_SUBTITLES[2],
            "adv_src_dump_partitions",
            EXEC_SUBTITLE,
        ],
    },
    AppBarFlow {
        title: "adv_dump_physical",
        writes_to_device: false,
        subtitles: &[
            "adv_dump_physical_desc",
            LOADER_SUBTITLES[0],
            LOADER_SUBTITLES[1],
            LOADER_SUBTITLES[2],
            "adv_src_dump_physical",
            EXEC_SUBTITLE,
        ],
    },
    AppBarFlow {
        title: "adv_flash_partitions",
        writes_to_device: true,
        subtitles: &[
            "adv_flash_partitions_desc",
            LOADER_SUBTITLES[0],
            LOADER_SUBTITLES[1],
            LOADER_SUBTITLES[2],
            "adv_src_flash_partitions",
            EXEC_SUBTITLE,
        ],
    },
    AppBarFlow {
        title: "adv_flash_physical",
        writes_to_device: true,
        subtitles: &[
            "adv_flash_physical_desc",
            LOADER_SUBTITLES[0],
            LOADER_SUBTITLES[1],
            LOADER_SUBTITLES[2],
            "adv_src_flash_physical",
            EXEC_SUBTITLE,
        ],
    },
    AppBarFlow {
        title: "adv_simple_flash",
        writes_to_device: true,
        subtitles: &["adv_simple_flash_desc", "flash_folder_desc", EXEC_SUBTITLE],
    },
    AppBarFlow {
        title: "adv_region_convert",
        writes_to_device: false,
        subtitles: &[
            "adv_region_convert_desc",
            "adv_src_region_convert",
            "adv_region_target_subtitle",
            EXEC_SUBTITLE,
        ],
    },
    AppBarFlow {
        title: "adv_image_info",
        writes_to_device: false,
        subtitles: &["adv_image_info_desc", "adv_src_image_info", EXEC_SUBTITLE],
    },
    AppBarFlow {
        title: "adv_patch_devinfo",
        writes_to_device: true,
        subtitles: &[
            "adv_patch_devinfo_desc",
            LOADER_SUBTITLES[0],
            LOADER_SUBTITLES[1],
            LOADER_SUBTITLES[2],
            "adv_src_patch_devinfo",
            EXEC_SUBTITLE,
        ],
    },
    AppBarFlow {
        title: "adv_detect_arb",
        writes_to_device: false,
        subtitles: &[
            "adv_detect_arb_desc",
            LOADER_SUBTITLES[0],
            LOADER_SUBTITLES[1],
            LOADER_SUBTITLES[2],
            "adv_src_detect_arb",
            EXEC_SUBTITLE,
        ],
    },
    AppBarFlow {
        title: "adv_patch_arb",
        writes_to_device: false,
        subtitles: &[
            "adv_patch_arb_desc",
            "adv_src_patch_arb_folder",
            EXEC_SUBTITLE,
        ],
    },
    AppBarFlow {
        title: "adv_convert_xml",
        writes_to_device: false,
        subtitles: &["adv_convert_xml_desc", "adv_src_convert_xml", EXEC_SUBTITLE],
    },
    AppBarFlow {
        title: "adv_rebuild_vbmeta",
        writes_to_device: false,
        subtitles: &[
            "adv_rebuild_vbmeta_desc",
            "adv_src_rebuild_vbmeta",
            EXEC_SUBTITLE,
        ],
    },
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
        // The disabled branch is a plain container, not a pick list, but
        // reserving the same arrow allowance as its enabled sibling keeps
        // both fixed-width branches safe if that changes.
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
    ConstrainedSlot {
        name: "dashboard.device-choice-dialog-action-row",
        kind: SlotKind::ActionRow(ActionRowSlot {
            actions: DEVICE_CHOICE_DIALOG_ACTIONS,
        }),
    },
    ConstrainedSlot {
        name: "wizard.app-bar-subtitle",
        kind: SlotKind::AppBarSubtitle(AppBarSubtitleSlot {
            flows: APP_BAR_FLOWS,
        }),
    },
];

// Mirrors the Compact app bar (`large_top_app_bar_with_chip` and
// `writes_to_device_chip` in src/view/components.rs) at the 820 px minimum
// window, beside the 80 px rail (`SIDEBAR_RAIL_WIDTH`, src/widgets.rs).
const APP_BAR_WINDOW_MIN_WIDTH: f32 = 820.0;
const APP_BAR_RAIL_WIDTH: f32 = 80.0;
const APP_BAR_HORIZONTAL_PADDING: f32 = 24.0;
const APP_BAR_ITEM_SPACING: f32 = 14.0;
const APP_BAR_TITLE_SIZE: f32 = 22.0;
const APP_BAR_SUBTITLE_SIZE: f32 = 12.0;
const APP_BAR_SUBTITLE_LINE_HEIGHT: f32 = 16.0;
const APP_BAR_SUBTITLE_MAX_LINES: f32 = 2.0;
const WRITES_CHIP_ICON_SIZE: f32 = 14.0;
const WRITES_CHIP_ICON_GAP: f32 = 4.0;
const WRITES_CHIP_TEXT_SIZE: f32 = 11.0;
const WRITES_CHIP_HORIZONTAL_PADDING: f32 = 10.0;

/// Word-wrapped paragraph size at a fixed line height, as the app bar
/// subtitle lays itself out.
fn measure_word_wrapped(
    locale: &str,
    value: &str,
    size: f32,
    line_height: f32,
    width: f32,
) -> Size {
    let paragraph = GraphicsParagraph::with_text(Text {
        content: value,
        bounds: Size::new(width, 100_000.0),
        size: Pixels(size),
        line_height: LineHeight::Absolute(Pixels(line_height)),
        font: locale_font(locale, Weight::Normal),
        align_x: Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::Word,
    });
    paragraph.min_bounds()
}

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

/// Mirrors `theme::font_family_for_language`.
fn locale_family(locale: &str) -> &'static str {
    match locale {
        "ja" => "Noto Sans JP",
        "zh" => "Noto Sans SC",
        _ => "Noto Sans KR",
    }
}

fn locale_font(locale: &str, weight: Weight) -> Font {
    Font {
        weight,
        ..Font::with_name(locale_family(locale))
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
    let locales = locale_codes();
    let mut failures = Vec::new();

    for slot in CONSTRAINED_SLOTS {
        for locale in locales.iter().map(String::as_str) {
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
                    let title_width =
                        measure_text(locale, title, DIALOG_HEADLINE_SIZE, Weight::Normal, None)
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
                SlotKind::AppBarSubtitle(app_bar) => {
                    let height_limit = APP_BAR_SUBTITLE_MAX_LINES * APP_BAR_SUBTITLE_LINE_HEIGHT;
                    let (chip_key, chip_label) = localized_copy(&table, key("adv_writes_chip"));
                    let chip_width = WRITES_CHIP_ICON_SIZE
                        + WRITES_CHIP_ICON_GAP
                        + measure_text(
                            locale,
                            chip_label,
                            WRITES_CHIP_TEXT_SIZE,
                            Weight::Normal,
                            None,
                        )
                        .width
                        + 2.0 * WRITES_CHIP_HORIZONTAL_PADDING;
                    let mut tightest = (String::new(), f32::MAX, Size::ZERO);
                    for flow in app_bar.flows {
                        let (title_key, title) = localized_copy(&table, key(flow.title));
                        let title_width =
                            measure_text(locale, title, APP_BAR_TITLE_SIZE, Weight::Medium, None)
                                .width;
                        let mut width_limit = APP_BAR_WINDOW_MIN_WIDTH
                            - APP_BAR_RAIL_WIDTH
                            - 2.0 * APP_BAR_HORIZONTAL_PADDING
                            - title_width
                            - APP_BAR_ITEM_SPACING;
                        if flow.writes_to_device {
                            width_limit -= APP_BAR_ITEM_SPACING + chip_width;
                        }
                        for subtitle_key in flow.subtitles {
                            let (subtitle_key, subtitle) =
                                localized_copy(&table, key(subtitle_key));
                            let measured = measure_word_wrapped(
                                locale,
                                subtitle,
                                APP_BAR_SUBTITLE_SIZE,
                                APP_BAR_SUBTITLE_LINE_HEIGHT,
                                width_limit.max(0.0),
                            );
                            if measured.width > width_limit + f32::EPSILON
                                || measured.height > height_limit + f32::EPSILON
                            {
                                let chip = if flow.writes_to_device {
                                    format!("+{chip_key}")
                                } else {
                                    String::new()
                                };
                                failures.push(overflow_message(
                                    slot.name,
                                    &format!("{title_key}{chip}+{subtitle_key}"),
                                    locale,
                                    format!("{:.1}x{:.1}px", measured.width, measured.height),
                                    format!("{width_limit:.1}x{height_limit:.1}px"),
                                ));
                            }
                            if width_limit < tightest.1 {
                                tightest =
                                    (format!("{title_key}+{subtitle_key}"), width_limit, measured);
                            }
                        }
                    }
                    println!(
                        "HEADROOM slot={} locale={} tightest={} subtitle={:.1}x{:.1}/{:.1}x{:.1}px",
                        slot.name,
                        locale,
                        tightest.0,
                        tightest.2.width,
                        tightest.2.height,
                        tightest.1,
                        height_limit,
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
        for locale in locale_codes() {
            let locale = locale.as_str();
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

#[test]
fn timeout_and_capacity_units_follow_the_selected_locale() {
    let expected = [
        ("en", "30 s", "16 GB"),
        ("ko", "30초", "16 GB"),
        ("ja", "30秒", "16 GB"),
        ("zh", "30 秒", "16 GB"),
        ("ru", "30 с", "16 ГБ"),
        ("fr", "30\u{a0}s", "16\u{a0}Go"),
    ];
    for locale in locale_codes() {
        assert!(
            expected.iter().any(|(listed, ..)| *listed == locale),
            "lang/{locale}.json has no expected timeout/capacity units; add them above"
        );
    }
    for (locale, seconds, gigabytes) in expected {
        let table = load_locale(locale);
        let timeout = table["err_active_slot_detect_failed"]
            .replace("{timeout}", "30")
            .replace("{detail}", "");
        assert!(timeout.contains(seconds), "{locale}: {timeout}");
        assert!(!timeout.contains("30s"));
        assert_eq!(table["unit_gigabytes"].replace("{value}", "16"), gigabytes);
    }
}
