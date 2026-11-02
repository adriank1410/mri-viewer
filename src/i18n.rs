//! Tłumaczenia interfejsu.
//!
//! Każdy język to jeden plik `locales/<kod>.json` z płaską mapą „klucz → tekst”.
//! Brakujące klucze są brane z angielskiego (`en.json`), a gdy i tam ich nie ma – pokazywany jest sam klucz.
//! Wstawki mają postać `{nazwa}` i są wypełniane przez [`trf`] albo makro [`t!`].
//! Liczba mnoga: klucze z końcówkami `.one`, `.few`, `.many`, `.other` (reguły CLDR, patrz [`plural_category`]).
//!
//! Dodanie języka: nowy plik w `locales/` i jeden wiersz w [`LANGS`].

#![allow(dead_code)] // TODO: usunąć po podpięciu tłumaczeń w interfejsie

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{OnceLock, RwLock};

/// Opis jednego języka.
pub struct Lang {
    /// Kod BCP 47 (np. `pl`, `pt`, `zh`).
    pub code: &'static str,
    /// Nazwa w tym języku (do listy w ustawieniach).
    pub native_name: &'static str,
    data: &'static str,
}

macro_rules! lang {
    ($code:literal, $name:literal) => {
        Lang { code: $code, native_name: $name, data: include_str!(concat!("../locales/", $code, ".json")) }
    };
}

/// Dostępne języki. Pierwszy (angielski) jest zapasowy.
pub static LANGS: &[Lang] = &[
    lang!("en", "English"),
    lang!("pl", "Polski"),
];

type Table = HashMap<String, &'static str>;

struct State {
    code: &'static str,
    current: Table,
    fallback: Table,
}

fn state() -> &'static RwLock<State> {
    static S: OnceLock<RwLock<State>> = OnceLock::new();
    S.get_or_init(|| {
        let fallback = parse(LANGS[0].data);
        RwLock::new(State { code: LANGS[0].code, current: Table::new(), fallback })
    })
}

/// Wczytuje plik języka. Teksty są „wyciekane” raz na wczytanie, dzięki czemu [`tr`] zwraca `&'static str`.
fn parse(data: &'static str) -> Table {
    let map: HashMap<String, String> = serde_json::from_str(data).unwrap_or_default();
    map.into_iter().map(|(k, v)| (k, &*Box::leak(v.into_boxed_str()))).collect()
}

/// Ustawia język interfejsu. `None` albo nieznany kod = język systemu, a gdy go nie ma – angielski.
pub fn set_language(code: Option<&str>) {
    let wanted = code.map(str::to_string).unwrap_or_else(system_language);
    let lang = find(&wanted).unwrap_or(&LANGS[0]);
    let mut s = state().write().unwrap_or_else(|e| e.into_inner());
    if s.code == lang.code && (!s.current.is_empty() || lang.code == LANGS[0].code) {
        return;
    }
    s.code = lang.code;
    s.current = if lang.code == LANGS[0].code { Table::new() } else { parse(lang.data) };
}

/// Kod bieżącego języka.
pub fn current() -> &'static str {
    state().read().unwrap_or_else(|e| e.into_inner()).code
}

/// Dopasowuje kod (np. `pt-BR`, `zh_Hans_CN`, `de-AT`) do dostępnego języka.
pub fn find(code: &str) -> Option<&'static Lang> {
    let code = code.to_ascii_lowercase().replace('_', "-");
    let base = code.split('-').next().unwrap_or("");
    LANGS.iter().find(|l| l.code == code).or_else(|| LANGS.iter().find(|l| l.code == base))
}

/// Pierwszy obsługiwany język z preferencji systemu (lub `en`).
pub fn system_language() -> String {
    sys_locale::get_locales()
        .find_map(|l| find(&l).map(|lang| lang.code.to_string()))
        .unwrap_or_else(|| LANGS[0].code.to_string())
}

/// Tekst dla klucza w bieżącym języku.
pub fn tr(key: &str) -> &'static str {
    let s = state().read().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = s.current.get(key).or_else(|| s.fallback.get(key)) {
        return v;
    }
    drop(s);
    missing(key)
}

/// Brakujący klucz: zwracamy go samego (raz zapamiętany, żeby nie przeciekać pamięci przy każdej klatce).
fn missing(key: &str) -> &'static str {
    static M: OnceLock<RwLock<HashMap<String, &'static str>>> = OnceLock::new();
    let m = M.get_or_init(Default::default);
    if let Some(v) = m.read().unwrap_or_else(|e| e.into_inner()).get(key) {
        return v;
    }
    let v: &'static str = Box::leak(key.to_string().into_boxed_str());
    m.write().unwrap_or_else(|e| e.into_inner()).insert(key.to_string(), v);
    v
}

/// Tekst z wypełnionymi wstawkami `{nazwa}`.
pub fn trf(key: &str, args: &[(&str, &dyn Display)]) -> String {
    fill(tr(key), args)
}

fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = template.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

/// Kategoria liczby mnogiej wg CLDR dla liczb całkowitych.
pub fn plural_category(lang: &str, n: u64) -> &'static str {
    let (n10, n100) = (n % 10, n % 100);
    match lang {
        "pl" => {
            if n == 1 {
                "one"
            } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                "few"
            } else {
                "many"
            }
        }
        "uk" | "ru" => {
            if n10 == 1 && n100 != 11 {
                "one"
            } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                "few"
            } else {
                "many"
            }
        }
        "cs" | "sk" => match n {
            1 => "one",
            2..=4 => "few",
            _ => "other",
        },
        "fr" | "pt" => {
            if n <= 1 {
                "one"
            } else {
                "other"
            }
        }
        "zh" | "ja" | "ko" | "tr" | "vi" | "id" | "th" => "other",
        _ => {
            if n == 1 {
                "one"
            } else {
                "other"
            }
        }
    }
}

/// Tekst w liczbie mnogiej, np. `trn("count.images", 5)` → „5 obrazów”. Wstawka `{n}` dostaje liczbę.
pub fn trn(key: &str, n: u64) -> String {
    trn_args(key, n, &[])
}

/// Jak [`trn`], z dodatkowymi wstawkami.
pub fn trn_args(key: &str, n: u64, args: &[(&str, &dyn Display)]) -> String {
    let cat = plural_category(current(), n);
    let full = format!("{key}.{cat}");
    let s = state().read().unwrap_or_else(|e| e.into_inner());
    let template = s
        .current
        .get(&full)
        .or_else(|| s.current.get(&format!("{key}.other")))
        .copied()
        .or_else(|| {
            // angielski zapasowy: tylko one/other
            let en_cat = if n == 1 { "one" } else { "other" };
            s.fallback.get(&format!("{key}.{en_cat}")).copied()
        })
        .unwrap_or(key);
    drop(s);
    let mut all: Vec<(&str, &dyn Display)> = vec![("n", &n)];
    all.extend_from_slice(args);
    fill(template, &all)
}

/// `t!("klucz")` → `&'static str`; `t!("klucz", nazwa = wartość, …)` → `String`.
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::tr($key)
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::trf($key, &[$((stringify!($name), &$value as &dyn std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(s: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            let Some(j) = rest[i..].find('}') else { break };
            out.insert(rest[i + 1..i + j].to_string());
            rest = &rest[i + j + 1..];
        }
        out
    }

    /// Każdy plik języka: poprawny JSON, tylko klucze znane z angielskiego, te same wstawki.
    #[test]
    fn locale_files_are_consistent() {
        let en: HashMap<String, String> = serde_json::from_str(LANGS[0].data).expect("en.json");
        let base = |k: &str| -> String {
            for suf in [".one", ".few", ".many", ".other"] {
                if let Some(b) = k.strip_suffix(suf) {
                    return b.to_string();
                }
            }
            k.to_string()
        };
        let en_bases: BTreeSet<String> = en.keys().map(|k| base(k)).collect();
        for lang in LANGS {
            let map: HashMap<String, String> =
                serde_json::from_str(lang.data).unwrap_or_else(|e| panic!("{}.json: {e}", lang.code));
            for (k, v) in &map {
                assert!(en_bases.contains(&base(k)), "{}: nieznany klucz {k}", lang.code);
                let en_v = en.get(k).or_else(|| en.get(&format!("{}.other", base(k)))).or_else(|| en.get(&format!("{}.one", base(k))));
                if let Some(en_v) = en_v {
                    let mut want = placeholders(en_v);
                    want.remove("n");
                    let mut got = placeholders(v);
                    got.remove("n");
                    assert_eq!(got, want, "{}: inne wstawki w {k}", lang.code);
                }
            }
            // każdy klucz angielski musi mieć tłumaczenie (bez liczby mnogiej sprawdzamy dokładnie)
            for k in en.keys() {
                if base(k) == *k {
                    assert!(map.contains_key(k), "{}: brak klucza {k}", lang.code);
                }
            }
        }
    }

    #[test]
    fn plural_rules() {
        assert_eq!(plural_category("pl", 1), "one");
        assert_eq!(plural_category("pl", 3), "few");
        assert_eq!(plural_category("pl", 5), "many");
        assert_eq!(plural_category("pl", 22), "few");
        assert_eq!(plural_category("pl", 12), "many");
        assert_eq!(plural_category("en", 1), "one");
        assert_eq!(plural_category("en", 2), "other");
        assert_eq!(plural_category("uk", 21), "one");
    }

    #[test]
    fn finds_regional_variants() {
        assert_eq!(find("pl-PL").map(|l| l.code), Some("pl"));
        assert_eq!(find("en_US").map(|l| l.code), Some("en"));
        assert!(find("xx").is_none());
    }
}
