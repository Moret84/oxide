//! On-screen texts of Rust and Swift from `translations.json` (`t`, `tf!` placeholders, `tp!` plurals); French until the app sets the language.
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    French,
    English,
}

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_language(language: Language) {
    ENGLISH.store(language == Language::English, Ordering::Relaxed);
}
pub fn language() -> Language {
    if english() {
        Language::English
    } else {
        Language::French
    }
}
pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

#[derive(Deserialize)]
pub struct Entry {
    pub fr: String,
    pub en: String,
}
impl Entry {
    fn text(&self, language: Language) -> &str {
        match language {
            Language::French => &self.fr,
            Language::English => &self.en,
        }
    }
}

/// The bundled catalog: key to French and English text.
pub fn catalog() -> &'static HashMap<String, Entry> {
    static CATALOG: OnceLock<HashMap<String, Entry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../translations.json")).expect("translations.json")
    })
}
/// The text of `key` in the current language; the key itself when missing, so a gap shows instead of crashing.
pub fn t(key: &str) -> &'static str {
    match catalog().get_key_value(key) {
        Some((_, entry)) => entry.text(language()),
        None => {
            debug_assert!(false, "missing translation: {key}");
            // Keys are literals in practice; leaking one is harmless.
            Box::leak(key.to_owned().into_boxed_str())
        }
    }
}
/// Every text in the current language, for Swift.
pub fn table() -> HashMap<String, String> {
    let language = language();
    catalog()
        .iter()
        .map(|(key, entry)| (key.clone(), entry.text(language).to_owned()))
        .collect()
}
/// `text` with each `{name}` replaced by its value.
pub fn fill(text: &str, args: &[(&str, String)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}
/// `key.one` or `key.other` for `n`: French takes the singular for 0 and 1, English for 1.
pub fn plural_key(key: &str, n: u64) -> String {
    let one = match language() {
        Language::French => n <= 1,
        Language::English => n == 1,
    };
    format!("{key}.{}", if one { "one" } else { "other" })
}

/// `t(key)` with its `{name}` placeholders filled: `tf!("key", name = value, …)`.
#[macro_export]
macro_rules! tf {
    ($key:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::fill($crate::t($key), &[$((stringify!($name), ($value).to_string())),*])
    };
}
/// The plural form of `key` for `n`, with `{n}` and the other placeholders filled.
#[macro_export]
macro_rules! tp {
    ($key:literal, $n:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        let n = $n;
        $crate::fill(
            $crate::t(&$crate::plural_key($key, n as u64)),
            &[("n", n.to_string()) $(, (stringify!($name), ($value).to_string()))*],
        )
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_placeholders() {
        assert_eq!(
            fill(
                "{n} sur {total}",
                &[("n", "2".into()), ("total", "5".into())]
            ),
            "2 sur 5"
        );
    }

    /// Both languages of every entry say something and use the same placeholders.
    #[test]
    fn entries_are_complete() {
        let placeholders = |text: &str| {
            let mut names: Vec<String> = text
                .split('{')
                .skip(1)
                .filter_map(|rest| rest.split_once('}').map(|(name, _)| name.to_owned()))
                .filter(|name| {
                    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                })
                .collect();
            names.sort();
            names.dedup();
            names
        };
        for (key, entry) in catalog() {
            assert!(
                !entry.fr.is_empty() && !entry.en.is_empty(),
                "{key} is empty"
            );
            assert_eq!(
                placeholders(&entry.fr),
                placeholders(&entry.en),
                "{key} placeholders"
            );
        }
    }

    /// Every key the Rust crates and the Swift app ask for is in the catalog.
    #[test]
    fn every_used_key_exists() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let mut missing = Vec::new();
        for dir in ["crates", "apps/macos/Sources/Oxide"] {
            visit(&format!("{root}/{dir}"), &mut |path, source| {
                for key in used_keys(source) {
                    let plural = || {
                        catalog().contains_key(&format!("{key}.one"))
                            && catalog().contains_key(&format!("{key}.other"))
                    };
                    if !catalog().contains_key(&key) && !plural() {
                        missing.push(format!("{path}: {key}"));
                    }
                }
            });
        }
        assert!(
            missing.is_empty(),
            "missing translations:\n{}",
            missing.join("\n")
        );
    }

    fn visit(dir: &str, f: &mut dyn FnMut(&str, &str)) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.to_string_lossy().into_owned();
            if path.is_dir() {
                // This crate's own docs and tests use example keys.
                if !name.ends_with("/target")
                    && !name.contains("/.build")
                    && !name.ends_with("/teams-i18n")
                {
                    visit(&name, f);
                }
            } else if (name.ends_with(".rs") || name.ends_with(".swift"))
                && let Ok(source) = std::fs::read_to_string(&path)
            {
                f(&name, &source);
            }
        }
    }

    /// Keys passed as literals to `t(`, `tf!(`, `tp!(` (Rust) and `L(`, `Lp(` (Swift).
    fn used_keys(source: &str) -> Vec<String> {
        let mut keys = Vec::new();
        for call in ["t(\"", "tf!(\"", "tp!(\"", "L(\"", "Lp(\""] {
            for (at, _) in source.match_indices(call) {
                let before = source[..at].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let rest = &source[at + call.len()..];
                if let Some(end) = rest.find('"') {
                    let key = &rest[..end];
                    if key.contains('.')
                        && key.chars().all(|c| c.is_alphanumeric() || "._".contains(c))
                    {
                        keys.push(key.to_owned());
                    }
                }
            }
        }
        keys
    }
}
