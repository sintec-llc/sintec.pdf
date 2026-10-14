//! UI localisation. Strings in code stay English and are the default lookup keys; a per-language
//! catalog (`*.tsv`, see `ru.tsv` for the format) maps them to display text at render time. Command
//! ids, document text, file names, the control channel, the CLI and MCP never see translated text,
//! so agents and scripts are unaffected. A string without a translation is shown in English, so
//! coverage can grow incrementally.
//!
//! The system is the one PhotoCraft uses (`photocraft/crates/ui-egui/src/i18n`), so catalogs and
//! habits carry over between the two apps.
//!
//! Sintec.PDF ships English and Russian only.
//!
//! # Adding a language
//! 1. Add `xx.tsv` next to `ru.tsv` (copy its header; translate from the *meaning* of the English
//!    text, clean-room, see `ru.tsv`).
//! 2. Add one row to [`LANGUAGES`] (code, native name, catalog, plural rule).
//!
//! That is all: the Preferences dropdown, the system-locale match and the catalog tests (parse,
//! placeholders, plural forms) pick it up from the registry.
//!
//! # Looking strings up
//! - [`tl!`](crate::tl) / [`t`]: a plain string in the current language. [`tr`]: in a given one.
//! - [`tr_ctx`] (`tl_ctx!` in UI code): when one English word needs different translations.
//! - [`tr_id`]: a command-id keyed string with the English label as fallback (menu items), so a
//!   translation survives rewording of the English text and can differ per command.
//! - [`trn`]: plural-aware (`{n}` is filled in). [`fmt`]: fill `{name}` placeholders after [`tr`];
//!   translators may reorder placeholders freely.

mod catalog;

use std::cell::Cell;
use std::sync::OnceLock;

use catalog::Catalog;

/// The `language` preference value that follows the system locale.
pub const AUTO: &str = "auto";

/// One supported UI language.
pub struct LangInfo {
    /// BCP 47 code, lowercase (`en`, `ru`). Also the `language` preference value.
    pub code: &'static str,
    /// The language's name in itself, shown in the Preferences dropdown.
    pub name: &'static str,
    /// Catalog file contents (empty for the built-in English).
    pub source: &'static str,
    /// Plural form index for a count (English: 0 = one, 1 = other; Russian: one, few, many). A
    /// catalog's `@plural` entries list one form per index.
    pub plural: fn(u64) -> usize,
    catalog: OnceLock<Catalog>,
}

fn plural_one_other(n: u64) -> usize {
    usize::from(n != 1)
}

/// Russian: 1 (but not 11) is `one`, 2–4 (but not 12–14) `few`, everything else `many`.
fn plural_russian(n: u64) -> usize {
    match n % 100 {
        11..=14 => 2,
        _ => match n % 10 {
            1 => 0,
            2..=4 => 1,
            _ => 2,
        },
    }
}

/// The registry. English first: it is the fallback and the source language.
pub static LANGUAGES: [LangInfo; 2] = [
    LangInfo { code: "en", name: "English", source: "", plural: plural_one_other, catalog: OnceLock::new() },
    // Russian; every `ru-*` locale (`ru-RU`, `ru-BY`, `ru-KZ` ...) resolves here.
    LangInfo { code: "ru", name: "Русский", source: include_str!("ru.tsv"), plural: plural_russian, catalog: OnceLock::new() },
];

impl LangInfo {
    /// How many plural forms the language's `@plural` entries list.
    fn plural_forms(&self) -> usize {
        (0..=1000).map(self.plural).max().unwrap_or(0).saturating_add(1)
    }

    fn catalog(&self) -> &Catalog {
        self.catalog.get_or_init(|| {
            let (catalog, errors) = Catalog::parse(self.source, self.plural_forms());
            for error in errors {
                log::warn!("{} interface catalog: {error}; that entry shows in English", self.code);
            }
            catalog
        })
    }
}

/// A language the UI can be shown in (a handle into [`LANGUAGES`]).
#[derive(Clone, Copy)]
pub struct Lang(&'static LangInfo);

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lang({})", self.0.code)
    }
}

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.0.code == other.0.code
    }
}

impl Eq for Lang {}

impl Lang {
    pub const EN: Lang = Lang(&LANGUAGES[0]);

    pub fn code(self) -> &'static str {
        self.0.code
    }

    /// A language by its exact code.
    pub fn from_code(code: &str) -> Option<Lang> {
        LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code)).map(Lang)
    }

    /// Resolve the `language` preference: a language code, or [`AUTO`] (and anything unknown,
    /// e.g. a code from a newer version) to follow the system locale.
    pub fn from_pref(pref: &str) -> Lang {
        Lang::from_code(pref).unwrap_or_else(system_lang)
    }

    /// Every registered language.
    pub fn all() -> impl Iterator<Item = Lang> {
        LANGUAGES.iter().map(Lang)
    }

    pub fn name(self) -> &'static str {
        self.0.name
    }

    fn catalog(self) -> &'static Catalog {
        self.0.catalog()
    }
}

/// The canonical `language` preference for user input: [`AUTO`] or a registered code (any case).
/// `None` for anything else, so callers can keep the current setting.
pub fn normalize_pref(pref: &str) -> Option<&'static str> {
    if pref.eq_ignore_ascii_case(AUTO) {
        return Some(AUTO);
    }
    Lang::from_code(pref).map(Lang::code)
}

/// Languages whose speakers mostly read Russian too: their locales fall back to the Russian
/// interface rather than English.
const RUSSIAN_FALLBACK: &[&str] = &["uk", "kk"];

/// Candidate language codes for a locale tag, most specific first: `ru_RU.UTF-8` →
/// `ru-ru`, `ru`; `uk_UA` → `uk-ua`, `uk`, `ru` ([`RUSSIAN_FALLBACK`]).
fn candidates(tag: &str) -> Vec<String> {
    let base = tag.split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    let parts: Vec<&str> = base.split('-').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for n in (1..=parts.len()).rev() {
        out.push(parts.get(..n).unwrap_or_default().join("-"));
    }
    if parts.first().is_some_and(|p| RUSSIAN_FALLBACK.contains(p)) {
        out.push("ru".to_string());
    }
    out
}

/// The registered language for a locale tag such as `ru_RU.UTF-8`, `ru-RU`; `None` if the
/// language isn't supported. `C`/`POSIX` mean English.
pub fn lang_from_tag(tag: &str) -> Option<Lang> {
    let cands = candidates(tag);
    if matches!(cands.first().map(String::as_str), Some("c" | "posix")) {
        return Some(Lang::EN);
    }
    cands.iter().find_map(|c| Lang::from_code(c))
}

/// The system language (cached). English when it can't be determined.
pub fn system_lang() -> Lang {
    // Tests drive the UI by its English labels whatever the developer's locale is. This covers
    // the unit tests; the integration tests link the library as it ships, where `cfg!(test)` is
    // off, so each of their harnesses pins the `language` option to `en` instead.
    if cfg!(test) {
        return Lang::EN;
    }
    static SYSTEM: OnceLock<Lang> = OnceLock::new();
    *SYSTEM.get_or_init(detect_system_lang)
}

#[cfg(not(target_arch = "wasm32"))]
fn detect_system_lang() -> Lang {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(l) = std::env::var(var).ok().and_then(|v| language_override(&v)) {
            return l;
        }
    }
    // Apps started from the Finder don't inherit LANG: use the macOS preferred-languages list.
    // The absolute path keeps a `defaults` earlier on PATH from running; any failure means English.
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("/usr/bin/defaults").args(["read", "-g", "AppleLanguages"]).output()
        && out.status.success()
        && let Some(l) = first_supported(&String::from_utf8_lossy(&out.stdout))
    {
        return l;
    }
    #[cfg(target_os = "windows")]
    if let Some(l) = windows_ui_language().as_deref().and_then(first_supported) {
        return l;
    }
    Lang::EN
}

/// A named locale can override the display language. On Windows, the neutral
/// C/POSIX locale inherited from a launcher says nothing about the user's UI language.
#[cfg(not(target_arch = "wasm32"))]
fn language_override(tag: &str) -> Option<Lang> {
    let tag = tag.trim();
    if cfg!(target_os = "windows") && matches!(candidates(tag).first().map(String::as_str), Some("c" | "posix")) {
        return None;
    }
    lang_from_tag(tag)
}

/// The Windows display languages in preference order, one tag per line, queried once when Auto
/// first resolves. `sys-locale` asks Windows directly (`GetUserPreferredUILanguages`), so no
/// process is started.
#[cfg(target_os = "windows")]
fn windows_ui_language() -> Option<String> {
    let tags: Vec<String> = sys_locale::get_locales().collect();
    (!tags.is_empty()).then(|| tags.join("\n"))
}

/// The first supported language in a macOS `defaults read` list or Windows UI-culture output.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn first_supported(list: &str) -> Option<Lang> {
    list.split(['(', ')', ',', '"', '\n']).map(str::trim).filter(|s| !s.is_empty()).find_map(lang_from_tag)
}

#[cfg(target_arch = "wasm32")]
fn detect_system_lang() -> Lang {
    Lang::EN
}

thread_local! {
    // Per thread, as in PhotoCraft: independent app and test threads (kittest harnesses run in
    // parallel) must not change each other's drawing language.
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::EN) };
}

/// Set the UI language for drawing (the shell calls this once per frame from the preference), so
/// widgets can translate without every call site carrying a language around.
pub fn set_current(lang: Lang) {
    CURRENT.set(lang);
}

/// `label` as it reads inside a sentence ("add a text box"): lowercased.
pub fn in_sentence(label: &str) -> std::borrow::Cow<'_, str> {
    std::borrow::Cow::Owned(label.to_lowercase())
}

/// The language the UI is drawn in.
pub fn current() -> Lang {
    CURRENT.get()
}

/// Does `lang` have a catalog entry for this plain string? (English never does: it is the source.)
pub fn has(lang: Lang, s: &str) -> bool {
    lang.catalog().plain(s).is_some()
}

/// Translate an English UI string into the current language ([`tr`] with [`current`]).
pub fn t(s: &str) -> &str {
    tr(current(), s)
}

/// The name of a keyboard key, distinct from a command such as Delete. Missing key-name
/// translations use the English key name, never a plain command translation.
pub fn key_name(s: &str) -> &str {
    key_name_impl(s)
}

/// Keyboard shortcuts written with the Mac's ⌘ ⇧ ⌥ in a string, as the platform writes them:
/// Ctrl+, Shift+ and Alt+ on Windows and Linux (Sintec.PDF; for strings that aren't built
/// from a key binding).
pub fn keys(s: &str) -> std::borrow::Cow<'_, str> {
    if cfg!(target_os = "macos") || !s.contains(['⌘', '⇧', '⌥']) {
        return std::borrow::Cow::Borrowed(s);
    }
    std::borrow::Cow::Owned(
        s.replace("⇧⌘", "Ctrl+Shift+").replace("⌥⌘", "Ctrl+Alt+").replace('⌘', "Ctrl+").replace('⇧', "Shift+").replace('⌥', "Alt+"),
    )
}

fn key_name_impl(s: &str) -> &str {
    current().catalog().contextual("key", s).unwrap_or(s)
}

/// Translate an English UI string; unknown strings come back unchanged.
pub fn tr(lang: Lang, s: &str) -> &str {
    lang.catalog().plain(s).unwrap_or(s)
}

/// How the layers below say their own sentences in the current language.
///
/// The checker, the preflight profiles and the signature panel hand over a whole English wording
/// — holes and all — and the document's own words apart from it, so this is all they need of us:
/// `f.said(&i18n::words)` says the sentence here, and `in_english` says it in English. An unknown
/// wording comes back as it stands, which is the English, so a catalog that is missing one is a
/// sentence in English rather than a gap.
pub fn words(wording: &str) -> String {
    t(wording).to_owned()
}

/// Like [`tr`], for an English string that needs a disambiguating `context`.
pub fn tr_ctx<'a>(lang: Lang, context: &str, s: &'a str) -> &'a str {
    lang.catalog().contextual(context, s).unwrap_or_else(|| tr(lang, s))
}

/// A string keyed by its command id, falling back to the translation of the English `label`.
pub fn tr_id<'a>(lang: Lang, id: &str, label: &'a str) -> &'a str {
    lang.catalog().id(id).unwrap_or_else(|| tr(lang, label))
}

/// Fill `{name}` placeholders in one pass. Unknown placeholders are left as written, and inserted
/// values are never read as templates again, so a file name containing `{n}` stays intact.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some((before, after_open)) = rest.split_once('{') {
        out.push_str(before);
        let Some((name, after_close)) = after_open.split_once('}') else {
            out.push('{');
            out.push_str(after_open);
            return out;
        };
        match args.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = after_close;
    }
    out.push_str(rest);
    out
}

/// A generated history label (the action part of "Undo …") in the current language. Captured
/// values (file and field names) are inserted as they are; unknown labels are looked up whole.
pub fn action_label(text: &str) -> String {
    for (prefix, template) in [
        ("Insert pages from ", "Insert pages from {name}"),
        ("Fill in ", "Fill in {name}"),
        ("Set the image of ", "Set the image of {name}"),
        ("Edit script of ", "Edit script of {name}"),
        ("Import ", "Import {name}"),
    ] {
        if let Some(value) = text.strip_prefix(prefix) {
            return fmt(t(template), &[("name", value)]);
        }
    }
    if let Some(key) = text.strip_prefix("Change ")
        && ["Title", "Author", "Subject", "Keywords", "Creator", "Producer"].contains(&key)
    {
        return fmt(t("Change {key}"), &[("key", t(key))]);
    }
    if let Some(n) = text.strip_prefix("Delete ").and_then(|rest| rest.strip_suffix(" fields")).and_then(|n| n.parse::<u64>().ok()) {
        return trn(current(), n, "Delete {n} field", "Delete {n} fields");
    }
    t(text).to_owned()
}

/// A history label in the current language, out of the engine's own two parts: the wording is
/// looked up whole and the document's own word goes into its `{}` afterwards, so a file or field
/// name is never read as part of the wording. This is exact where [`action_label`] has to guess
/// from the English text, and is what the engine's own labels go through.
pub fn edit_label(label: &pdfcraft_engine::Label) -> String {
    // A batch's wording is only the hole: it carries its name as text, which from below L7 is one
    // of `BATCH_LABELS` and so is looked up too.
    if let Some(name) = label.value.as_deref().filter(|_| label.wording == "{batch}") {
        return t(name).to_owned();
    }
    label.said(t(label.wording))
}

/// A command label such as "Undo Insert pages from a.pdf" in the current language. A catalog
/// whose word order doesn't put the verb first translates "Undo {action}" and "Redo {action}"
/// as whole phrases; otherwise the translated verb goes before the action.
pub fn command_label(text: &str) -> String {
    for (prefix, template) in [("Undo", "Undo {action}"), ("Redo", "Redo {action}")] {
        if let Some(action) = text.strip_prefix(prefix).and_then(|tail| tail.strip_prefix(' ')) {
            let action = action_label(action);
            if has(current(), template) {
                return fmt(t(template), &[("action", &action)]);
            }
            return format!("{} {action}", t(prefix));
        }
    }
    t(text).to_owned()
}

/// A registered command's menu label: its `@id` catalog entry if there is one, otherwise
/// [`command_label`] of the current English label.
pub fn menu_label(id: &str, label: &str) -> String {
    match current().catalog().id(id) {
        Some(translated) => translated.to_owned(),
        None => command_label(label),
    }
}

/// A plural-aware message: `one`/`other` are the English forms (with `{n}` where the count goes).
pub fn trn(lang: Lang, n: u64, one: &str, other: &str) -> String {
    let idx = (lang.0.plural)(n);
    let text = lang.catalog().plural(one, other, idx).unwrap_or(if n == 1 { one } else { other });
    fmt(text, &[("n", &n.to_string())])
}

#[cfg(test)]
mod tests {
    use super::catalog::{Entry, parse_entries, placeholders};
    use super::*;

    const RU: fn() -> Lang = || Lang::from_code("ru").expect("ru registered");

    #[test]
    fn only_english_and_russian_are_registered() {
        assert_eq!(Lang::all().map(Lang::code).collect::<Vec<_>>(), ["en", "ru"]);
        for gone in ["ja", "de", "uk", "zh-hans", "ar"] {
            assert_eq!(Lang::from_code(gone), None, "{gone}");
            assert_eq!(normalize_pref(gone), None, "{gone}");
            // A setting saved by an older version follows the system again.
            assert_eq!(Lang::from_pref(gone), Lang::EN, "{gone}");
        }
    }

    #[test]
    fn keyboard_key_names_have_a_separate_context_and_english_fallback() {
        for lang in Lang::all().filter(|l| *l != Lang::EN) {
            for key in ["Ctrl", "Shift", "Delete", "Esc", "Home", "End", "Space"] {
                assert!(lang.catalog().contextual("key", key).is_some(), "{}: {key}", lang.code());
            }
        }
        let previous = current();
        set_current(RU());
        assert_eq!(key_name("Space"), "Пробел");
        assert_eq!(key_name("Delete"), "Delete");
        assert_eq!(t("Delete"), "Удалить");
        assert_eq!(key_name("Open"), "Open", "a missing key name must not borrow a command translation");
        set_current(Lang::EN);
        assert_eq!(key_name("Delete"), "Delete");
        set_current(previous);
    }

    #[test]
    fn tags_map_to_languages() {
        assert_eq!(lang_from_tag("ru_RU.UTF-8"), Some(RU()));
        assert_eq!(lang_from_tag("ru-RU"), Some(RU()));
        assert_eq!(lang_from_tag("ru-KZ"), Some(RU()));
        assert_eq!(lang_from_tag("en_US.UTF-8"), Some(Lang::EN));
        assert_eq!(lang_from_tag("en-GB"), Some(Lang::EN));
        assert_eq!(lang_from_tag("C"), Some(Lang::EN));
        assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
        assert_eq!(lang_from_tag("fr_FR"), None);
        // Ukrainian and Kazakh fall back to Russian, not English.
        assert_eq!(lang_from_tag("uk-UA"), Some(RU()));
        assert_eq!(lang_from_tag("uk_UA.UTF-8"), Some(RU()));
        assert_eq!(lang_from_tag("kk-KZ"), Some(RU()));
        assert_eq!(lang_from_tag("kk"), Some(RU()));
        assert_eq!(lang_from_tag("be-BY"), None);
        assert_eq!(lang_from_tag(""), None);
        assert_eq!(lang_from_tag("_"), None);
    }

    #[test]
    fn candidates_walk_from_specific_to_general() {
        assert_eq!(candidates("ru_RU.UTF-8"), ["ru-ru", "ru"]);
        assert_eq!(candidates("ru"), ["ru"]);
        assert_eq!(candidates("sr-Latn-RS@latin"), ["sr-latn-rs", "sr-latn", "sr"]);
        assert_eq!(candidates("uk_UA"), ["uk-ua", "uk", "ru"]);
        assert_eq!(candidates("kk-KZ"), ["kk-kz", "kk", "ru"]);
        assert!(candidates("").is_empty());
    }

    #[test]
    fn macos_language_list_is_parsed() {
        assert_eq!(first_supported("(\n    \"ru-RU\",\n    \"en-US\"\n)\n"), Some(RU()));
        // An unsupported first choice falls through to the next supported one.
        assert_eq!(first_supported("(\n    \"fr-FR\",\n    \"en-US\"\n)\n"), Some(Lang::EN));
        assert_eq!(first_supported("("), None);
    }

    #[test]
    fn windows_display_language_list_is_parsed() {
        assert_eq!(first_supported("ru-RU\r\n"), Some(RU()));
        assert_eq!(first_supported("de-DE\r\nru-RU\r\nen-US\r\n"), Some(RU()));
        assert_eq!(first_supported("en-US\r\n"), Some(Lang::EN));
        assert_eq!(first_supported("fr-FR\r\n"), None);
        // A Ukrainian or Kazakh Windows without Russian in its list still gets Russian.
        assert_eq!(first_supported("uk-UA\r\n"), Some(RU()));
        assert_eq!(first_supported("kk-KZ\r\nen-US\r\n"), Some(RU()));
        assert_eq!(first_supported("\r\n"), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn neutral_locale_does_not_override_display_language() {
        for tag in ["C", "C.UTF-8", "POSIX", "posix.UTF-8", "", "  C.UTF-8  ", "unsupported"] {
            assert_eq!(language_override(tag), None, "{tag}");
            assert_eq!(language_override(tag).or_else(|| first_supported("unsupported-XY\nru-RU\nen-US")), Some(RU()));
        }
        assert_eq!(["C.UTF-8", "POSIX", "ru_RU.UTF-8"].into_iter().find_map(language_override), Some(RU()));
        assert_eq!(language_override("ru_RU.UTF-8"), Some(RU()));
        assert_eq!(language_override("en_US.UTF-8"), Some(Lang::EN));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_display_language_query_returns_a_locale_tag() {
        // One tag per line, as `first_supported` reads them: a machine with several display
        // languages reports them all, so each line is checked rather than the list as a whole.
        let list = windows_ui_language().expect("Windows should report a display language");
        let tags: Vec<&str> = list.lines().map(str::trim).filter(|t| !t.is_empty()).collect();
        assert!(!tags.is_empty(), "no display language in {list:?}");
        for tag in tags {
            assert!(tag.split('-').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric())), "not a locale tag: {tag:?}");
        }
    }

    #[test]
    fn preferences_resolve_with_fallback() {
        assert_eq!(Lang::from_pref("ru"), RU());
        assert_eq!(Lang::from_pref("RU"), RU());
        assert_eq!(Lang::from_pref("en"), Lang::EN);
        // `auto` and unknown codes follow the system (English under test).
        assert_eq!(Lang::from_pref(AUTO), Lang::EN);
        assert_eq!(Lang::from_pref("xx-unknown"), Lang::EN);
        assert_eq!(normalize_pref("RU"), Some("ru"));
        assert_eq!(normalize_pref("Auto"), Some(AUTO));
        assert_eq!(normalize_pref("xx"), None);
    }

    #[test]
    fn lookups_fall_back_to_english() {
        assert_eq!(tr(RU(), "no such label"), "no such label");
        assert_eq!(tr(Lang::EN, "File"), "File");
        assert_eq!(tr(RU(), "File"), "Файл");
        assert_eq!(tr(RU(), "Отчёт.pdf"), "Отчёт.pdf");
        assert_eq!(tr_id(RU(), "no.such.id", "File"), "Файл");
        assert_eq!(tr_ctx(RU(), "no such context", "File"), "Файл");
        assert!(has(RU(), "Save") && !has(Lang::EN, "Save"));
    }

    #[test]
    fn catalog_kinds_are_parsed_and_looked_up() {
        let (c, errors) =
            Catalog::parse("# c\n\tHello\tПривет\n@id\tfile.save\tСохранить файл\nmenu\tWindows\tОкна\n@plural\t{n} file|{n} files\t{n} файл\n\n", 1);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.plain("Hello"), Some("Привет"));
        assert_eq!(c.id("file.save"), Some("Сохранить файл"));
        assert_eq!(c.contextual("menu", "Windows"), Some("Окна"));
        assert_eq!(c.contextual("other", "Windows"), None);
        assert_eq!(c.plural("{n} file", "{n} files", 0), Some("{n} файл"));
        assert_eq!(c.plural("{n} file", "{n} files", 5), Some("{n} файл"), "an index past the forms clamps");
    }

    #[test]
    fn malformed_lines_are_reported_not_fatal() {
        let (entries, errors) = parse_entries("\tok\tда\nno tabs here\n\tonly\n\ta\tb\tc\textra\n\t\tempty source\n", 1);
        assert_eq!(entries.len(), 1);
        assert_eq!(errors.len(), 4, "{errors:?}");
        let (entries, errors) = parse_entries("\ta\\tb\tx\\ny\\\\z\n", 1);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(entries, [Entry { context: String::new(), source: "a\tb".into(), translation: "x\ny\\z".into() }]);
    }

    #[test]
    fn strict_validation_skips_only_the_bad_line() {
        for (text, forms, error) in [
            ("\tFile", 1, "context<TAB>"),
            ("\tFile\t", 1, "nonempty"),
            ("\tFile\\q\tFiles", 1, "unknown escape"),
            ("\tFile\tFiles\\", 1, "trailing backslash"),
            ("\tOpen…\tOpen", 1, "ellipsis"),
            ("\tOpen {file}\tOpen {name}", 1, "placeholders"),
            ("@plural\tpage\tPages", 1, "one|other"),
            ("@plural\t{n} page|{n} pages\t{n} pages", 2, "2 nonempty"),
            ("@plural\t{n} page|{n} pages\tPages", 1, "placeholders"),
            ("@unknown\tFile\tFiles", 1, "reserved context"),
        ] {
            // The bad line comes first; the good line after it still loads.
            let (catalog, errors) = Catalog::parse(&format!("{text}\n\tSave\tСохранить\n"), forms);
            assert_eq!(errors.len(), 1, "{text:?}: {errors:?}");
            assert!(errors[0].starts_with("line 1: ") && errors[0].contains(error), "{errors:?}");
            assert_eq!(catalog.plain("Save"), Some("Сохранить"), "{text:?}");
        }
        let (catalog, errors) = Catalog::parse("\tFile\tФайл\n\tFile\tДругое\n", 1);
        assert!(errors.len() == 1 && errors[0].contains("line 2: duplicate"), "{errors:?}");
        assert_eq!(catalog.plain("File"), Some("Файл"), "the first entry wins");
    }

    #[test]
    fn catalogs_handle_escapes_crlf_and_distinct_contexts() {
        let (catalog, errors) =
            Catalog::parse("# comment\r\n\r\n\tLine\\nTab\\tPath\\\\\tOther\\nTab\\tPath\\\\\r\nweight\tLight\tThin\r\ntheme\tLight\tPale\r\n", 1);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(catalog.plain("Line\nTab\tPath\\"), Some("Other\nTab\tPath\\"));
        assert_eq!(catalog.contextual("weight", "Light"), Some("Thin"));
        assert_eq!(catalog.contextual("theme", "Light"), Some("Pale"));
    }

    #[test]
    fn broken_catalog_falls_back_to_english() {
        static BROKEN: LangInfo =
            LangInfo { code: "broken", name: "Broken test catalog", source: "malformed\n\\", plural: plural_one_other, catalog: OnceLock::new() };
        assert_eq!(tr(Lang(&BROKEN), "File"), "File");
        assert_eq!(trn(Lang(&BROKEN), 2, "{n} page", "{n} pages"), "2 pages");
    }

    #[test]
    fn contextual_id_and_plural_lookups_have_english_fallbacks() {
        static TEST: LangInfo = LangInfo {
            code: "test",
            name: "Test catalog",
            source: "\tLight\tPlain light\nweight\tLight\tThin\n@id\tfile.open\tOpen dialog…\n@plural\t{n} page|{n} pages\tOne page: {n}|Many pages: {n}\n",
            plural: plural_one_other,
            catalog: OnceLock::new(),
        };
        let l = Lang(&TEST);
        assert_eq!(tr_ctx(l, "weight", "Light"), "Thin");
        assert_eq!(tr_ctx(l, "theme", "Light"), "Plain light");
        assert_eq!(tr_ctx(l, "theme", "Unknown"), "Unknown");
        assert_eq!(tr_id(l, "file.open", "Reworded English…"), "Open dialog…");
        assert_eq!(tr_id(l, "unknown", "Light"), "Plain light");
        assert_eq!(tr_id(l, "unknown", "Unknown"), "Unknown");
        assert_eq!(trn(l, 1, "{n} page", "{n} pages"), "One page: 1");
        assert_eq!(trn(l, 0, "{n} page", "{n} pages"), "Many pages: 0");
        assert_eq!(trn(l, u64::MAX, "{n} page", "{n} pages"), format!("Many pages: {}", u64::MAX));
    }

    #[test]
    fn formatting_reorders_parameters_without_reinterpreting_user_text() {
        let args = [("file", "Отчёт-{n}.pdf"), ("n", "2")];
        assert_eq!(fmt("{n}: {file}; {unknown}", &args), "2: Отчёт-{n}.pdf; {unknown}");
        assert_eq!(fmt("{file} / {file}", &args), "Отчёт-{n}.pdf / Отчёт-{n}.pdf");
        assert_eq!(fmt("text {unfinished", &args), "text {unfinished");
        assert_eq!(fmt("}{n}{", &args), "}2{");
        assert_eq!(fmt("unchanged", &[]), "unchanged");
    }

    /// #103: the missing-OCR-models messages are translated under their current English text
    /// (catalogs once keyed an older `PRINTCRAFT_MODELS` wording, which never matched).
    #[test]
    fn missing_ocr_models_messages_are_translated() {
        let error = pdfcraft_engine::ocr::OcrError::NoModels.to_string();
        let notice = "Text recognition isn't installed: its model files are missing. Reinstall Sintec.PDF, or set PDFCRAFT_MODELS to the folder that holds them.";
        assert!(include_str!("../ocr_ui.rs").contains(notice), "keep in step with ocr_ui.rs");
        assert!(has(RU(), notice) && has(RU(), &error));
        for lang in &LANGUAGES {
            assert!(!lang.source.contains("PRINTCRAFT"), "{}", lang.code);
        }
    }

    /// Every label Shared Field Properties shows, including those it passes through a variable
    /// (flag labels, errors, the font label), exists in the Russian catalog.
    #[test]
    fn bulk_field_properties_labels_are_in_the_russian_catalog() {
        let source = include_str!("../bulk_fields.rs");
        // The string literal starting right after `at`, unescaped.
        fn literal(rest: &str) -> Option<String> {
            let mut out = String::new();
            let mut chars = rest.chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => return Some(out),
                    '\\' => out.push(chars.next()?),
                    c => out.push(c),
                }
            }
            None
        }
        let mut labels = std::collections::BTreeSet::new();
        for marker in ["tl!(\"", "Err(\"", "ok_or(\"", "|| \"", "MISSING: &str = \"", "i18n::t(\"", "_ => \""] {
            for (at, _) in source.match_indices(marker) {
                labels.extend(source.get(at + marker.len()..).and_then(literal));
            }
        }
        // Flag labels: `(field_flags::NAME, "Label", inverted)`.
        for (at, _) in source.match_indices("(field_flags::") {
            let line = source.get(at..).and_then(|s| s.lines().next()).unwrap_or_default();
            if let Some(q) = line.find(", \"") {
                labels.extend(line.get(q + 3..).and_then(literal));
            }
        }
        labels.retain(|l| l.chars().any(char::is_alphabetic) && !l.contains("credits") && !l.starts_with("bulk"));
        for label in ["Mixed", "Custom font", "The document is no longer open."] {
            labels.insert(label.to_string());
        }
        assert!(labels.len() > 20 && labels.contains("Comb of characters"), "the scan found the dialog's labels: {labels:?}");
        let missing: Vec<_> = labels.iter().filter(|l| !has(RU(), l)).collect();
        assert!(missing.is_empty(), "Russian lacks Shared Field Properties labels: {missing:#?}");
    }

    /// Every `tl!("…")` literal in the UI source (test modules aside), unescaped.
    fn ui_literals() -> std::collections::BTreeSet<String> {
        scan_ui_literals(false)
    }

    // Lithuanian also checks direct lookup calls and multiline literal arguments.
    fn scan_ui_literals(expanded: bool) -> std::collections::BTreeSet<String> {
        let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        let mut literals = std::collections::BTreeSet::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).expect("UI source directory") {
                let path = entry.expect("UI source entry").path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|name| name != "i18n") {
                        stack.push(path);
                    }
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let source = std::fs::read_to_string(path).expect("UI source file").replace("\r\n", "\n");
                    let source = if expanded { source.replace("crate::i18n::t(", "tl!(") } else { source };
                    let mut rest = source.split("#[cfg(test)]\nmod ").next().unwrap_or_default();
                    while let Some((_, after)) = rest.split_once(if expanded { "tl!(" } else { "tl!(\"" }) {
                        let after = if expanded {
                            let after = after.trim_start();
                            let Some(after) = after.strip_prefix('"') else {
                                rest = after;
                                continue;
                            };
                            after
                        } else {
                            after
                        };
                        let mut escaped = false;
                        let end = after
                            .char_indices()
                            .find_map(|(i, c)| {
                                if c == '"' && !escaped {
                                    return Some(i);
                                }
                                escaped = c == '\\' && !escaped;
                                None
                            })
                            .expect("closed tl! literal");
                        let (raw, tail) = after.split_at(end);
                        let argument_end = tail.strip_prefix('"').expect("closing quote").trim_start();
                        let literal_call =
                            argument_end.starts_with(')') || argument_end.strip_prefix(',').is_some_and(|rest| rest.trim_start().starts_with(')'));
                        if tail.starts_with("\")") || (expanded && literal_call) {
                            let label: String = serde_json::from_str(&format!("\"{raw}\"")).expect("UI literal escapes");
                            literals.insert(label);
                        }
                        rest = tail.strip_prefix('"').expect("closing quote");
                    }
                }
            }
        }
        literals
    }

    #[test]
    fn current_language_is_per_thread() {
        set_current(RU());
        assert_eq!(t("File"), "Файл");
        std::thread::spawn(|| assert_eq!(current(), Lang::EN)).join().unwrap();
        set_current(Lang::EN);
        assert_eq!(t("File"), "File");
    }

    #[test]
    fn plurals_and_placeholders() {
        assert_eq!(trn(Lang::EN, 1, "{n} item", "{n} items"), "1 item");
        assert_eq!(trn(Lang::EN, 0, "{n} item", "{n} items"), "0 items");
        assert_eq!(trn(Lang::EN, 7, "{n} item", "{n} items"), "7 items");
        // No `@plural` entry: Russian falls back to the English forms.
        assert_eq!(trn(RU(), 7, "{n} gizmo", "{n} gizmos"), "7 gizmos");
        assert_eq!(fmt("{b} before {a}", &[("a", "x"), ("b", "y"), ("c", "z")]), "y before x");
        assert_eq!(fmt("{missing}", &[]), "{missing}");
        assert_eq!(placeholders("a {x} b {y} {"), ["x", "y"]);
    }

    #[test]
    fn language_persists_and_invalid_input_keeps_current_language() {
        let mut app = crate::PdfCraftApp::default();
        assert_eq!(app.language, AUTO);
        app.set_option("language", "RU").unwrap();
        assert_eq!(app.language, "ru");
        assert!(app.set_option("language", "xx").is_err());
        assert!(app.set_option("language", "ja").is_err(), "removed languages are not accepted");
        assert_eq!(app.language, "ru");
        let mut restored = crate::PdfCraftApp::default();
        restored.restore(&app.persist());
        assert_eq!(restored.language, "ru");
        restored.restore(r#"{"language":"xx"}"#);
        assert_eq!(restored.language, "ru");
        app.set_option("language", AUTO).unwrap();
        assert_eq!(app.language, AUTO);
        // Settings written before the system-locale default (no key) follow the system.
        let mut legacy = crate::PdfCraftApp::default();
        legacy.restore("{}");
        assert_eq!(legacy.language, AUTO);
    }

    #[test]
    fn russian_is_registered_with_its_plurals_and_date_months() {
        let ru = RU();
        assert_eq!(lang_from_tag("ru_RU.UTF-8"), Some(ru));
        assert_eq!(lang_from_tag("ru-RU"), Some(ru));
        for (n, form) in [(1, 0), (21, 0), (101, 0), (11, 2), (2, 1), (4, 1), (22, 1), (12, 2), (14, 2), (0, 2), (5, 2), (25, 2), (111, 2)] {
            assert_eq!(plural_russian(n), form, "plural_russian({n})");
        }
        assert_eq!(trn(ru, 3, "{n} page", "{n} pages"), "3 страницы");
        assert_eq!(trn(ru, 5, "{n} page", "{n} pages"), "5 страниц");
        // The calendar header uses the nominative, a date in a sentence the genitive.
        assert_eq!(tr(ru, "January"), "Январь");
        assert_eq!(tr_ctx(ru, "date", "January"), "января");
    }

    /// A catalog that translates "Undo {action}" as a whole phrase (a language that puts the verb
    /// elsewhere) uses it; one without it (Russian) keeps the verb in front of the action.
    #[test]
    fn undo_and_redo_labels_use_the_catalog_phrase_when_there_is_one() {
        static VERB_LAST: LangInfo = LangInfo {
            code: "verb-last",
            name: "Verb-last test catalog",
            source: "\tUndo\tundo\n\tUndo {action}\t{action}: undo\n\tRedo {action}\t{action}: redo\n",
            plural: plural_one_other,
            catalog: OnceLock::new(),
        };
        set_current(Lang(&VERB_LAST));
        assert_eq!(command_label("Undo"), "undo");
        assert_eq!(command_label("Undo Untranslated custom action"), "Untranslated custom action: undo");
        assert_eq!(command_label("Redo Untranslated custom action"), "Untranslated custom action: redo");
        set_current(RU());
        assert_eq!(command_label("Undo Untranslated custom action"), "Отменить Untranslated custom action");
        set_current(Lang::EN);
        assert_eq!(command_label("Undo Untranslated custom action"), "Undo Untranslated custom action");
    }

    #[test]
    fn tool_names_inside_sentences_are_lowercased() {
        set_current(Lang::EN);
        assert_eq!(in_sentence("Text Box"), "text box");
        set_current(RU());
        assert_eq!(in_sentence("Текстовое поле"), "текстовое поле");
        set_current(Lang::EN);
    }

    #[test]
    fn russian_is_registered() {
        let ru = RU();
        assert_eq!(ru.name(), "Русский");
        assert_eq!(normalize_pref("RU"), Some("ru"));
        assert_eq!(lang_from_tag("ru_RU.UTF-8"), Some(ru));
        assert_eq!(lang_from_tag("ru-BY"), Some(ru));
        assert_eq!(first_supported("ru-RU\r\nen-US"), Some(ru));
        assert_eq!(tr(ru, "File"), "Файл");
        assert_eq!(tr(ru, "Save as…"), "Сохранить как…");
        assert_eq!(tr(ru, "Bookmarks"), "Закладки");
        assert_eq!(tr(ru, "Layers"), "Слои");
        assert_eq!(tr(ru, "Rapport de l'utilisateur.pdf"), "Rapport de l'utilisateur.pdf");
        // one (1), few (2-4), many (0, 5+), with 11-14 exception
        assert_eq!((0..=3).map(|n| (ru.0.plural)(n)).collect::<Vec<_>>(), [2, 0, 1, 1]);
        assert_eq!(trn(ru, 0, "{n} page", "{n} pages"), "0 страниц");
        assert_eq!(trn(ru, 1, "{n} page", "{n} pages"), "1 страница");
        assert_eq!(trn(ru, 2, "{n} page", "{n} pages"), "2 страницы");
        assert_eq!(trn(ru, 5, "{n} page", "{n} pages"), "5 страниц");
        assert_eq!(trn(ru, 11, "{n} page", "{n} pages"), "11 страниц");
        assert_eq!(trn(ru, 21, "{n} page", "{n} pages"), "21 страница");
        assert_eq!(trn(ru, 22, "{n} page", "{n} pages"), "22 страницы");
        assert_eq!(trn(ru, 1, "{n} field", "{n} fields"), "1 поле");
        assert_eq!(trn(ru, 2, "{n} field", "{n} fields"), "2 поля");
        assert_eq!(trn(ru, 5, "{n} field", "{n} fields"), "5 полей");
        let mut app = crate::PdfCraftApp::default();
        app.set_option("language", "ru").unwrap();
        assert_eq!(app.language, "ru");
        let mut restored = crate::PdfCraftApp::default();
        restored.restore(&app.persist());
        assert_eq!(restored.language, "ru");
    }

    /// Russian translates every registered command and every All tools group, section and item.
    #[test]
    fn russian_covers_commands_and_catalogue() {
        let ru = RU();
        for command in pdfcraft_engine::commands::COMMANDS {
            assert!(has(ru, command.label), "missing command: {}", command.label);
            if let Some(menu) = command.menu {
                assert!(has(ru, menu), "missing menu: {menu}");
            }
        }
        for group in pdfcraft_engine::catalog::TOOL_GROUPS {
            assert!(has(ru, group.label), "missing group: {}", group.label);
            for section in group.sections {
                assert!(has(ru, section.title), "missing section: {}", section.title);
                for item in section.items {
                    assert!(has(ru, item.label), "missing item: {}", item.label);
                }
            }
        }
    }

    /// New tl!("literal") labels must not silently fall back to English.
    #[test]
    fn russian_covers_ui_literals() {
        let literals = ui_literals();
        assert!(literals.len() > 900, "source scan found only {} literals", literals.len());
        let missing: Vec<_> = literals.iter().filter(|label| !has(RU(), label)).collect();
        assert!(missing.is_empty(), "untranslated Russian UI literals: {missing:#?}");
    }

    #[test]
    fn russian_history_and_diagnostics_preserve_user_values() {
        let ru = RU();
        set_current(ru);
        assert_eq!(command_label("Undo Insert pages from Rapport {n}.pdf"), "Отменить Вставить страницы из Rapport {n}.pdf");
        assert_eq!(command_label("Redo Fill in Contact {key}"), "Повторить Заполнить Contact {key}");
        assert_eq!(action_label("Change Title"), "Изменить: Заголовок");
        assert_eq!(action_label("Custom action {n}"), "Custom action {n}");
        assert_eq!(action_label("Untranslated custom action"), "Untranslated custom action");
        assert_eq!(menu_label("file.open", "Open…"), "Открыть…");
        assert_eq!(fmt(t("This page couldn't be displayed.\n{e}"), &[("e", "OS error {n}")]), "Эту страницу не удалось показать.\nOS error {n}");
        // Sintec.PDF: «Выбрано страниц: N» reads right for every N (no plural forms for this key).
        assert_eq!(fmt(t("{n} pages selected"), &[("n", "3")]), "Выбрано страниц: 3");
        assert_eq!(tr(ru, "CheckBox"), "Флажок");
        set_current(Lang::EN);
        assert_eq!(command_label("Undo Fill in Save {e}"), "Undo Fill in Save {e}");
    }

    /// The OCR dialog shows each document language by its English name, translated.
    #[test]
    fn ocr_language_names_are_translated() {
        let missing: Vec<_> = pdfcraft_engine::ocr::LANGUAGES.iter().map(|l| l.1).filter(|name| !has(RU(), name)).collect();
        assert!(missing.is_empty(), "OCR language names missing from ru.tsv: {missing:?}");
    }

    /// Every bundled catalog is well-formed and consistent with its sources.
    #[test]
    fn bundled_catalogs_are_consistent() {
        let mut codes = std::collections::HashSet::new();
        for l in &LANGUAGES {
            assert!(codes.insert(l.code), "duplicate code {}", l.code);
            assert!(l.code == l.code.to_ascii_lowercase() && !l.name.is_empty(), "{}", l.code);
            let (entries, errors) = parse_entries(l.source, l.plural_forms());
            assert!(errors.is_empty(), "{}: {errors:?}", l.code);
            for e in entries.iter().filter(|e| e.context == "@id") {
                let command = pdfcraft_engine::commands::command(&e.source);
                let Some(command) = command else { panic!("{}: unknown command id {:?}", l.code, e.source) };
                assert_eq!(placeholders(&e.translation), placeholders(command.label), "{}: {}", l.code, e.source);
                assert_eq!(e.translation.ends_with('…'), command.label.ends_with('…'), "{}: ellipsis mismatch: {}", l.code, e.source);
            }
        }
    }

    /// Preferences ▸ Date format ▸ Language offers exactly the interface languages, in order.
    #[test]
    fn date_languages_are_the_interface_languages() {
        let interface: Vec<(&str, &str)> = Lang::all().map(|l| (l.code(), l.name())).collect();
        let dates: Vec<(&str, &str)> = pdfcraft_engine::dates::DATE_LANGUAGES.iter().map(|l| (l.code, l.name)).collect();
        assert_eq!(dates, interface);
    }
}
