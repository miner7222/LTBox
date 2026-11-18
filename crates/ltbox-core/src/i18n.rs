//! Localized log lines for backend crates via a translator the GUI installs at
//! startup and re-installs on every language switch. Backends call [`tr`]
//! without depending on the GUI's language state.

use std::sync::{OnceLock, RwLock};

type TranslatorFn = dyn Fn(&str) -> String + Send + Sync;

/// Translator installed by the GUI; absent in non-GUI contexts.
static TRANSLATOR: OnceLock<RwLock<Box<TranslatorFn>>> = OnceLock::new();

/// Install or swap the translator (the closure sits behind an `RwLock`, so
/// language switches take effect live).
pub fn set_translator<F>(f: F)
where
    F: Fn(&str) -> String + Send + Sync + 'static,
{
    let new_box: Box<TranslatorFn> = Box::new(f);
    match TRANSLATOR.get() {
        Some(lock) => {
            if let Ok(mut guard) = lock.write() {
                *guard = new_box;
            }
        }
        None => {
            let _ = TRANSLATOR.set(RwLock::new(new_box));
        }
    }
}

/// Resolve `key` via the installed translator; without one, return the key
/// itself.
pub fn tr(key: &str) -> String {
    if let Some(lock) = TRANSLATOR.get()
        && let Ok(guard) = lock.read()
    {
        return guard(key);
    }
    key.to_string()
}

/// Substitute named values in a translated template in one pass. Values are
/// opaque: braces in a file path or error must not become new placeholders.
pub fn format_template(template: &str, values: &[(&str, String)]) -> String {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find('}') else { break };
        let name = &rest[1..end];
        if let Some((_, value)) = values.iter().find(|(key, _)| *key == name) {
            result.push_str(value);
        } else {
            result.push_str(&rest[..=end]);
        }
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substituted_paths_are_not_interpreted_as_templates() {
        let values = [
            ("path", "C:/images/{error}/boot.img".into()),
            ("error", "timeout".into()),
        ];
        assert_eq!(
            format_template("{path}: {error}", &values),
            "C:/images/{error}/boot.img: timeout"
        );
        assert_eq!(
            format_template("{error} / {error} / {unknown}", &values),
            "timeout / timeout / {unknown}"
        );
        assert_eq!(format_template("literal {", &values), "literal {");
    }

    #[test]
    fn tr_routes_through_the_installed_translator() {
        // TRANSLATOR is a process-global; this is the only test that installs
        // one. The closure falls back to the key for anything it doesn't map,
        // matching the no-translator behavior, so test ordering is irrelevant.
        set_translator(|k| {
            if k == "hello" {
                "world".to_string()
            } else {
                k.to_string()
            }
        });
        assert_eq!(tr("hello"), "world");
        assert_eq!(tr("unmapped_key"), "unmapped_key");
    }
}
