use super::*;

const RU: fn() -> Lang = || Lang::from_code("ru").expect("ru registered");

/// Skips a test that needs the embedded catalogs when none were embedded: the interface is then
/// English only. (`build.rs` fails without `i18n/locales/`, so this is a safety net.)
macro_rules! needs_catalogs {
    () => {
        if !needs_catalogs() {
            return;
        }
    };
}

/// The rows of `text` (catalog format) as (context, English source, translation), escapes
/// undone; comments, blank and malformed lines left out.
fn rows(text: &str) -> Vec<(String, String, String)> {
    let unescape = |s: &str| s.replace("\\\\", "\u{0}").replace("\\n", "\n").replace("\\t", "\t").replace('\u{0}', "\\");
    text.lines()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut c = l.split('\t');
            let (Some(context), Some(source), Some(translation), None) = (c.next(), c.next(), c.next(), c.next()) else { return None };
            Some((unescape(context), unescape(source), unescape(translation)))
        })
        .collect()
}

/// The embedded PdfCraft catalog of `code` (not the shared one), `""` without one.
fn app_catalog(code: &str) -> &'static str {
    CRAFT_I18N.iter().find(|(c, ..)| *c == code).map_or("", |(_, _, app)| *app)
}

#[test]
fn the_macos_bundle_declares_every_interface_language() {
    // AppKit translates its own dialogs (open and save panels) only into the languages the app
    // bundle declares, so `CFBundleLocalizations` must list every entry of `LANGUAGES`.
    let plist = include_str!("../../../../packaging/macos/Info.plist.in");
    let block = plist.split("<key>CFBundleLocalizations</key>").nth(1).expect("the template declares CFBundleLocalizations");
    let block = block.split("</array>").next().expect("the declaration is an array");
    let declared: Vec<String> = block.split("<string>").skip(1).filter_map(|s| s.split("</string>").next()).map(str::to_lowercase).collect();
    let registered: Vec<String> = LANGUAGES.iter().map(|l| l.code.to_lowercase()).collect();
    assert_eq!(declared, registered, "packaging/macos/Info.plist.in and LANGUAGES list different languages");
}

#[test]
fn keyboard_key_names_have_a_separate_context_and_english_fallback() {
    let keys = fixture_lang("x-keys", "key\tDelete\tEntf\n\tDelete\tLöschen\n\tOpen\tÖffnen\n");
    let previous = current();
    set_current(keys);
    assert_eq!(key_name("Delete"), "Entf");
    assert_eq!(t("Delete"), "Löschen");
    assert_eq!(key_name("Open"), "Open", "a missing key name must not borrow a command translation");
    set_current(Lang::EN);
    assert_eq!(key_name("Delete"), "Delete");
    set_current(previous);
}

/// Regression test: a language picker uses its native name only when a face can draw it, otherwise its
/// English name.
#[test]
fn picker_names_fall_back_to_english_when_their_script_has_no_face() {
    assert_eq!(Lang::EN.picker_name(|_| false), "English");
    needs_catalogs!();
    assert_eq!(RU().picker_name(|_| true), "Русский", "the native name is kept when it can be drawn");
    assert_eq!(RU().picker_name(|_| false), "Russian", "no Cyrillic face: an English name, not an unrenderable label");
    for language in Lang::all() {
        assert!(!language.english_name().is_empty(), "{}", language.code());
        assert_ne!(language.english_name(), language.code(), "{} needs an English picker fallback", language.code());
    }
}

#[test]
fn tags_map_to_languages() {
    assert_eq!(lang_from_tag("en_US.UTF-8"), Some(Lang::EN));
    assert_eq!(lang_from_tag("C"), Some(Lang::EN));
    assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
    assert_eq!(lang_from_tag(""), None);
    assert_eq!(lang_from_tag("_"), None);
    needs_catalogs!();
    assert_eq!(lang_from_tag("ru_RU.UTF-8"), Some(RU()));
    assert_eq!(lang_from_tag("ru-BY"), Some(RU()));
    // Ukrainian and Kazakh systems get Russian rather than English (`RUSSIAN_FALLBACK`).
    assert_eq!(lang_from_tag("uk-UA"), Some(RU()));
    assert_eq!(lang_from_tag("kk"), Some(RU()));
    assert_eq!(lang_from_tag("ja_JP.UTF-8"), None, "Sintec.PDF offers English and Russian only");
    assert_eq!(lang_from_tag("fr-FR"), None);
}

#[test]
fn candidates_walk_from_specific_to_general() {
    assert_eq!(candidates("ru_RU.UTF-8"), ["ru-ru", "ru"]);
    assert_eq!(candidates("uk_UA"), ["uk-ua", "uk", "ru"]);
    assert_eq!(candidates("kk-KZ"), ["kk-kz", "kk", "ru"]);
    assert_eq!(candidates("zh-Hant-HK"), ["zh-hant-hk", "zh-hant", "zh"]);
    assert_eq!(candidates("de@euro"), ["de"]);
    assert!(candidates("").is_empty());
}

/// The locale variables, the first detection step: `LC_ALL` over `LC_MESSAGES` over `LANG`, an
/// unsupported value passed over, and Sintec.PDF's own mapping (Ukrainian and Kazakh → Russian).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn locale_variables_pick_the_language_with_pdfcraft_mappings() {
    let env = |vars: &'static [(&'static str, &'static str)]| move |name: &str| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string());
    assert_eq!(env_lang(env(&[])), None);
    assert_eq!(env_lang(env(&[("LANG", "en_US.UTF-8")])), Some(Lang::EN));
    needs_catalogs!();
    assert_eq!(env_lang(env(&[("LC_MESSAGES", "uk_UA.UTF-8"), ("LANG", "en_US.UTF-8")])), Some(RU()));
    assert_eq!(env_lang(env(&[("LC_ALL", "xx_YY.UTF-8"), ("LC_MESSAGES", "ru_RU.UTF-8")])), Some(RU()));
    if cfg!(not(target_os = "windows")) {
        // Outside Windows a neutral locale means English and ends the search.
        assert_eq!(env_lang(env(&[("LC_ALL", "C"), ("LANG", "ru_RU.UTF-8")])), Some(Lang::EN));
    }
}

/// The macOS preferred-languages list (`defaults read -g AppleLanguages`), the second step.
#[test]
fn macos_language_list_is_parsed() {
    assert_eq!(first_supported("("), None);
    needs_catalogs!();
    assert_eq!(first_supported("(\n    \"ru-RU\",\n    \"en-US\"\n)\n"), Some(RU()));
    assert_eq!(first_supported("(\n    \"xx-YY\",\n    \"kk-KZ\"\n)\n"), Some(RU()));
    assert_eq!(first_supported("(\n    \"fr-FR\",\n    \"en-US\"\n)\n"), Some(Lang::EN));
}

/// #2954: the web build's Auto language follows the browser's preferred languages in order,
/// skipping ones without a catalog, and is English only when none is supported.
#[test]
fn browser_language_list_is_followed_in_order() {
    assert_eq!(first_supported_tag(["", "xx"]), None);
    assert_eq!(first_supported_tag(Vec::<String>::new()), None);
    needs_catalogs!();
    assert_eq!(first_supported_tag(["xx-YY", "ru", "en-US"]), Some(RU()));
    assert_eq!(first_supported_tag(["en-GB", "ru-RU"]), Some(Lang::EN));
    assert_eq!(first_supported_tag([" uk-UA "]), Some(RU()));
    let long = std::iter::repeat_n("xx", 10_000).chain(["ru"]);
    assert_eq!(first_supported_tag(long), None, "only the first tags are read");
}

/// The Windows display languages (`sys_locale`), the last step: one tag per line.
#[test]
fn windows_display_language_list_is_read_in_order() {
    assert_eq!(first_supported("\r\n"), None);
    needs_catalogs!();
    assert_eq!(first_supported("ru-RU\r\n"), Some(RU()));
    assert_eq!(first_supported("en-US\r\n"), Some(Lang::EN));
    assert_eq!(first_supported("zh-CN\r\n"), None);
    assert_eq!(first_supported("xx-YY\r\nuk-UA\r\nen-US\r\n"), Some(RU()));
}

#[cfg(target_os = "windows")]
#[test]
fn neutral_locale_does_not_override_display_language() {
    needs_catalogs!();
    for tag in ["C", "C.UTF-8", "POSIX", "posix.UTF-8", "", "  C.UTF-8  ", "unsupported"] {
        assert_eq!(language_override(tag), None, "{tag}");
        assert_eq!(language_override(tag).or_else(|| first_supported("unsupported-XY\nru-RU\nen-US")), Some(RU()));
    }
    assert_eq!(["C.UTF-8", "POSIX", "ru_RU.UTF-8"].into_iter().find_map(language_override), Some(RU()));
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
    assert_eq!(Lang::from_pref("en"), Lang::EN);
    // `auto` and unknown codes follow the system (English under test).
    assert_eq!(Lang::from_pref(AUTO), Lang::EN);
    assert_eq!(Lang::from_pref("xx-unknown"), Lang::EN);
    assert_eq!(normalize_pref("Auto"), Some(AUTO));
    assert_eq!(normalize_pref("xx"), None);
    needs_catalogs!();
    assert_eq!(Lang::from_pref("ru"), RU());
    assert_eq!(Lang::from_pref("RU"), RU());
    assert_eq!(normalize_pref("RU"), Some("ru"));
}

/// PdfCraft offers exactly the languages in `LANGUAGES` when craft-i18n is embedded, and English
/// alone when it isn't: a missing catalog never fails, it shows English.
#[test]
fn the_languages_offered_are_the_embedded_ones_of_the_list() {
    let all: Vec<&str> = Lang::all().map(Lang::code).collect();
    if !catalogs_embedded() {
        assert_eq!(all, ["en"]);
        assert_eq!(Lang::from_code("fr"), None);
        assert_eq!(Lang::from_pref("fr"), Lang::EN, "a saved language without a catalog shows English");
        assert_eq!(normalize_pref("fr"), None);
        assert_eq!(tr(Lang::EN, "File"), "File");
        return;
    }
    let listed: Vec<&str> = LANGUAGES.iter().map(|l| l.code).collect();
    assert_eq!(all, listed, "every language Sintec.PDF lists has its catalog in craft-i18n");
    assert!(Lang::from_code("pseudo").is_none(), "the pseudo locale is never a setting");
    // craft-i18n has catalogs PdfCraft doesn't offer (yet); they are never offered here.
    for code in CRAFT_I18N.iter().map(|(c, ..)| *c).filter(|c| !listed.contains(c)) {
        assert_eq!(Lang::from_code(code), None, "{code} is not one of Sintec.PDF's languages");
    }
}

#[test]
fn lookups_fall_back_to_english() {
    assert_eq!(tr(Lang::EN, "File"), "File");
    assert!(!has(Lang::EN, "Save"));
    needs_catalogs!();
    assert_eq!(tr(RU(), "no such label"), "no such label");
    assert_eq!(tr(RU(), "File"), "Файл");
    assert_eq!(tr(RU(), "Документ.pdf"), "Документ.pdf");
    assert_eq!(tr_id(RU(), "no.such.id", "File"), "Файл");
    assert_eq!(tr_ctx(RU(), "no such context", "File"), "Файл");
    assert!(has(RU(), "Save"));
}

#[test]
fn catalog_kinds_are_parsed_and_looked_up() {
    let l = fixture_lang(
        "ja",
        "# c\n\tHello\tこんにちは\n@id\tfile.save\t保存する\nmenu\tWindows\tウィンドウ群\n@plural\t{n} file|{n} files\t{n} 個\n\n",
    );
    assert!(l.i18n().check_all().is_empty(), "{:?}", l.i18n().check_all());
    assert_eq!(tr(l, "Hello"), "こんにちは");
    assert_eq!(tr_id(l, "file.save", "Save"), "保存する");
    assert_eq!(tr_ctx(l, "menu", "Windows"), "ウィンドウ群");
    assert_eq!(tr_ctx(l, "other", "Windows"), "Windows");
    assert_eq!(trn(l, 0, "{n} file", "{n} files"), "0 個");
    assert_eq!(trn(l, 5, "{n} file", "{n} files"), "5 個");
}

/// A bad line is skipped and reported; the rest of the catalog still loads.
#[test]
fn malformed_lines_are_reported_not_fatal() {
    let l = fixture_lang("x-bad", "\tok\tはい\nno tabs here\n\tonly\n\ta\tb\tc\textra\n\t\tempty source\n\tOpen…\tOpen\n");
    assert_eq!(tr(l, "ok"), "はい");
    assert_eq!(tr(l, "Open…"), "Open…", "a row that drops the ellipsis is skipped");
    assert_eq!(l.i18n().check_all().len(), 5, "{:?}", l.i18n().check_all());
}

#[test]
fn catalogs_handle_escapes_crlf_and_distinct_contexts() {
    let l =
        fixture_lang("x-esc", "# comment\r\n\r\n\tLine\\nTab\\tPath\\\\\tOther\\nTab\\tPath\\\\\r\nweight\tLight\tThin\r\ntheme\tLight\tPale\r\n");
    assert!(l.i18n().check_all().is_empty(), "{:?}", l.i18n().check_all());
    assert_eq!(tr(l, "Line\nTab\tPath\\"), "Other\nTab\tPath\\");
    assert_eq!(tr_ctx(l, "weight", "Light"), "Thin");
    assert_eq!(tr_ctx(l, "theme", "Light"), "Pale");
}

#[test]
fn broken_catalog_falls_back_to_english() {
    let broken = fixture_lang("x-broken", "malformed\n\\");
    assert_eq!(tr(broken, "File"), "File");
    assert_eq!(trn(broken, 2, "{n} page", "{n} pages"), "2 pages");
}

#[test]
fn contextual_id_and_plural_lookups_have_english_fallbacks() {
    let l = fixture_lang(
        "de",
        "\tLight\tPlain light\nweight\tLight\tThin\n@id\tfile.open\tOpen dialog…\n@plural\t{n} page|{n} pages\tOne page: {n}|Many pages: {n}\n",
    );
    assert_eq!(tr_ctx(l, "weight", "Light"), "Thin");
    assert_eq!(tr_ctx(l, "theme", "Light"), "Plain light");
    assert_eq!(tr_ctx(l, "theme", "Unknown"), "Unknown");
    assert_eq!(tr_id(l, "file.open", "Reworded English…"), "Open dialog…");
    assert_eq!(tr_id(l, "unknown", "Light"), "Plain light");
    assert_eq!(tr_id(l, "unknown", "Unknown"), "Unknown");
    assert_eq!(trn(l, 1, "{n} page", "{n} pages"), "One page: 1");
    assert_eq!(trn(l, 0, "{n} page", "{n} pages"), "Many pages: 0");
    assert_eq!(trn(l, 1234, "{n} page", "{n} pages"), "Many pages: 1234", "no grouping, as before");
    assert_eq!(trn(l, u64::MAX, "{n} page", "{n} pages"), format!("Many pages: {}", u64::MAX));
}

#[test]
fn formatting_reorders_parameters_without_reinterpreting_user_text() {
    let args = [("file", "日本語-{n}.pdf"), ("n", "2")];
    assert_eq!(fmt("{n}: {file}; {unknown}", &args), "2: 日本語-{n}.pdf; {unknown}");
    assert_eq!(fmt("{file} / {file}", &args), "日本語-{n}.pdf / 日本語-{n}.pdf");
    assert_eq!(fmt("text {unfinished", &args), "text {unfinished");
    assert_eq!(fmt("}{n}{", &args), "}2{");
    assert_eq!(fmt("unchanged", &[]), "unchanged");
}

/// Every literal passed to `tl!(…)` or `crate::i18n::t(…)` in the UI source (test modules
/// aside), unescaped, including literals on a line of their own.
fn ui_literals() -> std::collections::BTreeSet<String> {
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
                let source = source.replace("crate::i18n::t(", "tl!(");
                let mut rest = source.split("#[cfg(test)]\nmod ").next().unwrap_or_default();
                while let Some((_, after)) = rest.split_once("tl!(") {
                    let after = after.trim_start();
                    let Some(after) = after.strip_prefix('"') else {
                        rest = after;
                        continue;
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
                    if literal_call {
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

/// Each translated label of a visible signature that runs into its value keeps a separator
/// at its end.
#[test]
fn signature_appearance_labels_keep_their_separator() {
    needs_catalogs!();
    let english = pdfcraft_engine::sign::AppearanceLabels::default();
    for lang in Lang::all().filter(|l| *l != Lang::EN) {
        for (field, label) in english.fields() {
            let said = tr(lang, label);
            if field != "signed_by" && !lang.rtl() {
                assert!(said.ends_with(' ') || said.ends_with('：'), "{}: the {field} label runs into its value: {said:?}", lang.code());
            }
        }
    }
}

#[test]
fn history_labels_keep_captured_names() {
    set_current(Lang::EN);
    assert_eq!(command_label("Undo Fill in Save {e}"), "Undo Fill in Save {e}");
    needs_catalogs!();
    set_current(RU());
    assert_eq!(action_label("Untranslated custom action"), "Untranslated custom action");
    assert_eq!(command_label("Undo Untranslated {e} action"), "Отменить: Untranslated {e} action", "a captured name is never read as a placeholder");
    assert_eq!(menu_label("file.open", "Open…"), "Открыть…");
    set_current(Lang::EN);
}

/// An `@id` row that drops the English label's trailing `…` or changes its `{placeholders}` is not
/// used, by the menus or by `tr_id`: the label's translation shows instead. craft-i18n checks
/// neither for `@id` rows, whose source is the command id.
#[test]
fn an_id_row_that_changes_the_label_s_shape_is_not_used() {
    let l = fixture_lang(
        "x-shape",
        "@id\tfile.open\tÖffnen\n@id\tfile.close\tSchließen {name}\n@id\tfile.save\tSichern…\n\tOpen…\tÖffnen…\n\tClose\tSchließen\n",
    );
    assert_eq!(tr_id(l, "file.open", "Open…"), "Öffnen…", "the row drops the ellipsis");
    assert_eq!(tr_id(l, "file.close", "Close"), "Schließen", "the row adds a placeholder");
    assert_eq!(tr_id(l, "file.save", "Save…"), "Sichern…", "a row that keeps the shape is used");
    set_current(l);
    assert_eq!(menu_label("file.open", "Open…"), "Öffnen…");
    assert_eq!(menu_label("file.close", "Close"), "Schließen");
    assert_eq!(menu_label("file.save", "Save…"), "Sichern…");
    set_current(Lang::EN);
    assert!(same_shape("{b} und {a}", "{a} and {b}"), "placeholders may be reordered");
    assert!(!same_shape("Öffnen {name…", "Open…"), "a placeholder left open");
    assert!(!same_shape("Öffnen name}…", "Open…"), "a closing brace without its opening one");
    assert!(!same_shape("{name}} öffnen", "Open {name}"), "a second closing brace");
}

/// Command ids go by PdfCraft's own `@id` rows only: the shared catalog's are keyed by the other
/// apps' command ids (in Bulgarian its `file.close` row is the id itself). The Undo/Redo phrases
/// are used only as a pair: a shared "Undo {action}" comes without its "Redo {action}".
#[test]
fn command_ids_and_undo_phrases_come_from_pdfcraft_s_own_rows() {
    let fx = fixture_lang(
        "x-own",
        "@id\tfile.close\tClose-FX\n\tUndo {action}\tFX-undo: {action}\n\tClose file\tClose file-FX\n\tUndo\tFX-U\n\tRedo\tFX-R\n",
    );
    assert_eq!(tr_id(fx, "file.close", "Close file"), "Close-FX", "a row of its own is used");
    set_current(fx);
    assert_eq!(menu_label("file.close", "Close file"), "Close-FX");
    assert_eq!(command_label("Undo Something"), "FX-U Something", "one phrase without its pair is not used");
    let pair = fixture_lang("x-pair", "\tUndo {action}\tFX-undo: {action}\n\tRedo {action}\tFX-redo: {action}\n");
    set_current(pair);
    assert_eq!(command_label("Undo Something"), "FX-undo: Something");
    assert_eq!(command_label("Redo Something"), "FX-redo: Something");
    set_current(Lang::EN);
    needs_catalogs!();
    for code in ["ru"] {
        let lang = Lang::from_code(code).expect("registered");
        if lang.has_own("@id", "file.close") {
            continue;
        }
        assert_eq!(tr_id(lang, "file.close", "Close file"), tr(lang, "Close file"), "{code}: file.close is not a shared row's");
        set_current(lang);
        assert_eq!(menu_label("file.close", "Close file"), command_label("Close file"), "{code}");
    }
    for code in ["ru"] {
        let lang = Lang::from_code(code).expect("registered");
        if has(lang, "Redo {action}") {
            continue;
        }
        set_current(lang);
        assert_eq!(command_label("Undo Something"), format!("{} Something", t("Undo")), "{code}");
        assert_eq!(command_label("Redo Something"), format!("{} Something", t("Redo")), "{code}");
    }
    set_current(Lang::EN);
}

#[test]
fn current_language_is_per_thread() {
    set_current(fixture_lang("x-thread", "\tFile\tファイル\n"));
    assert_eq!(t("File"), "ファイル");
    std::thread::spawn(|| assert_eq!(current(), Lang::EN)).join().unwrap();
    set_current(Lang::EN);
    assert_eq!(t("File"), "File");
}

#[test]
fn plurals_and_placeholders() {
    assert_eq!(trn(Lang::EN, 1, "{n} item", "{n} items"), "1 item");
    assert_eq!(trn(Lang::EN, 0, "{n} item", "{n} items"), "0 items");
    assert_eq!(trn(Lang::EN, 7, "{n} item", "{n} items"), "7 items");
    assert_eq!(trn(Lang::EN, 12345, "{n} item", "{n} items"), "12345 items");
    assert_eq!(fmt("{b} before {a}", &[("a", "x"), ("b", "y"), ("c", "z")]), "y before x");
    assert_eq!(fmt("{missing}", &[]), "{missing}");
    needs_catalogs!();
    // No catalog has this `@plural` entry: Russian falls back to the English forms.
    assert_eq!(trn(RU(), 7, "{n} test item", "{n} test items"), "7 test items");
}

/// PdfCraft's plural rules before craft-i18n (form index by count), kept to show the crate's
/// CLDR categories pick the same form of the same rows.
fn pdfcraft_plural_rule(code: &str) -> fn(u64) -> usize {
    fn one_other(n: u64) -> usize {
        usize::from(n != 1)
    }
    fn none(_: u64) -> usize {
        0
    }
    fn zero_one_other(n: u64) -> usize {
        usize::from(n > 1)
    }
    fn ro(n: u64) -> usize {
        if n == 1 {
            0
        } else if n == 0 || (1..=19).contains(&(n % 100)) {
            1
        } else {
            2
        }
    }
    fn lt(n: u64) -> usize {
        match (n % 10, n % 100) {
            (1, 11) => 2,
            (1, _) => 0,
            (2..=9, 12..=19) => 2,
            (2..=9, _) => 1,
            _ => 2,
        }
    }
    fn cs(n: u64) -> usize {
        match n {
            1 => 0,
            2..=4 => 1,
            _ => 2,
        }
    }
    fn ru_uk(n: u64) -> usize {
        match n % 100 {
            11..=14 => 2,
            _ => match n % 10 {
                1 => 0,
                2..=4 => 1,
                _ => 2,
            },
        }
    }
    fn ar(n: u64) -> usize {
        match (n, n % 100) {
            (0, _) => 0,
            (1, _) => 1,
            (2, _) => 2,
            (_, 3..=10) => 3,
            (_, 11..=99) => 4,
            _ => 5,
        }
    }
    fn pl(n: u64) -> usize {
        match (n, n % 100, n % 10) {
            (1, _, _) => 0,
            (_, 12..=14, _) => 2,
            (_, _, 2..=4) => 1,
            _ => 2,
        }
    }
    fn sl(n: u64) -> usize {
        match n % 100 {
            1 => 0,
            2 => 1,
            3 | 4 => 2,
            _ => 3,
        }
    }
    match code {
        "ja" | "zh-hans" | "zh-hant" | "tr" | "id" | "vi" | "th" | "lo" | "hi" | "ko" => none,
        "cs" => cs,
        "pt-br" | "fr" | "fa" => zero_one_other,
        "ru" | "uk" => ru_uk,
        "ar" => ar,
        "pl" => pl,
        "sl" => sl,
        "lt" => lt,
        "ro" => ro,
        _ => one_other,
    }
}

/// The counts the plural checks try: 0..=200, then large ones with every ending.
fn plural_counts() -> impl Iterator<Item = u64> {
    (0..=200).chain([1000, 1001, 1002, 1003, 1011, 1012, 1021, 1100, 1_000_000, 1_000_001, 2_000_000, 123_456_789, u64::MAX - 13, u64::MAX])
}

/// craft-i18n picks plural forms by CLDR category, PdfCraft by its own rules: for every language
/// the two choose the same form wherever they list as many forms. Where CLDR lists more (Hindi
/// and Turkish have two where PdfCraft had one), the catalog repeats the form, so the text is the
/// same; the embedded rows are checked for that and any that differ are reported.
#[test]
fn plural_forms_follow_the_rules_pdfcraft_had() {
    let mut split = Vec::new();
    // English has no catalog: its own two forms, `one` for 1 alone, as PdfCraft's rule had it.
    assert_eq!(plural_counts().map(|n| trn(Lang::EN, n, "1", "x")).filter(|s| s == "1").count(), 1);
    for info in LANGUAGES.iter().filter(|l| l.code != "en") {
        let rule = pdfcraft_plural_rule(info.code);
        let pdfcraft_forms = (0..=1000).map(rule).max().unwrap_or(0) + 1;
        // How many forms CLDR gives this language: the first form count whose row loads.
        let fixture = |forms: usize| {
            let row = (0..forms).map(|i| format!("F{i} {{n}}")).collect::<Vec<_>>().join("|");
            fixture_lang(info.code, Box::leak(format!("@plural\t{{n}} x|{{n}} xs\t{row}\n").into_boxed_str()))
        };
        let (forms, lang) =
            (1..=6).map(|f| (f, fixture(f))).find(|(_, l)| trn(*l, 5, "{n} x", "{n} xs").starts_with('F')).expect("a form count loads");
        if forms != pdfcraft_forms {
            split.push((info.code, pdfcraft_forms, forms));
            assert_eq!(pdfcraft_forms, 1, "{}: Sintec.PDF listed {pdfcraft_forms} forms, CLDR {forms}", info.code);
            continue;
        }
        for n in plural_counts() {
            assert_eq!(trn(lang, n, "{n} x", "{n} xs"), format!("F{} {n}", rule(n)), "{}: count {n}", info.code);
        }
    }
    let codes: Vec<&str> = split.iter().map(|(c, ..)| *c).collect();
    assert!(codes.is_empty(), "the languages CLDR gives more forms than Sintec.PDF had: {codes:?}");
    if !needs_catalogs() {
        return;
    }
    for (code, ..) in split {
        let lang = Lang::from_code(code).expect("registered");
        for (_, source, forms) in lang.catalog_texts().flat_map(rows).filter(|(c, ..)| c == "@plural") {
            let distinct: std::collections::BTreeSet<&str> = forms.split('|').collect();
            if distinct.len() > 1 {
                println!("{code}: {source:?} has forms that differ, so Sintec.PDF's single form is no longer the only text: {forms:?}");
            }
        }
    }
}

#[test]
fn language_persists_and_invalid_input_keeps_current_language() {
    let mut app = crate::PdfCraftApp::default();
    assert_eq!(app.language, AUTO);
    assert!(app.set_option("language", "xx").is_err());
    // Settings written before the system-locale default (no key) follow the system.
    let mut legacy = crate::PdfCraftApp::default();
    legacy.restore("{}");
    assert_eq!(legacy.language, AUTO);
    needs_catalogs!();
    app.set_option("language", "RU").unwrap();
    assert_eq!(app.language, "ru");
    assert!(app.set_option("language", "xx").is_err());
    assert_eq!(app.language, "ru");
    assert!(app.set_option("language", "ja").is_err(), "a language Sintec.PDF doesn't offer");
    let mut restored = crate::PdfCraftApp::default();
    restored.restore(&app.persist());
    assert_eq!(restored.language, "ru");
    restored.restore(r#"{"language":"xx"}"#);
    assert_eq!(restored.language, "ru");
    app.set_option("language", AUTO).unwrap();
    assert_eq!(app.language, AUTO);
}

#[test]
fn undo_and_redo_labels_use_the_catalog_phrase_when_there_is_one() {
    needs_catalogs!();
    set_current(RU());
    assert_eq!(command_label("Undo"), "Отменить");
    assert_eq!(command_label("Undo Untranslated custom action"), "Отменить: Untranslated custom action");
    set_current(Lang::EN);
    assert_eq!(command_label("Undo Untranslated custom action"), "Undo Untranslated custom action");
}

#[test]
fn contextual_labels_fall_back_to_the_plain_translation() {
    needs_catalogs!();
    assert_eq!(tr(RU(), "Type"), "Тип");
    // A context no catalog defines, so the fallback stays tested as catalogs fill their gaps.
    assert_eq!(tr_ctx(RU(), "no such context", "Type"), "Тип");
}

/// The code joins some labels with a space at either end, so these catalogs (PdfCraft's rows in
/// craft-i18n) keep exactly the source's, and write … rather than three dots.
#[test]
fn catalog_rows_keep_the_source_spacing_and_ellipsis() {
    needs_catalogs!();
    let edges = |s: &str| (s.len() - s.trim_start().len(), s.len() - s.trim_end().len());
    for code in ["ru"] {
        for (_, source, translation) in rows(app_catalog(code)) {
            assert!(!translation.contains("..."), "{code}: use … rather than three dots: {translation:?}");
            assert_eq!(edges(&translation), edges(&source), "{code}: stray whitespace: {translation:?}");
        }
    }
}

#[test]
fn tool_names_inside_sentences_are_lowercased() {
    set_current(Lang::EN);
    assert_eq!(in_sentence("Text Box"), "text box");
    needs_catalogs!();
    set_current(RU());
    assert_eq!(in_sentence("Текстовое поле"), "текстовое поле");
    set_current(Lang::EN);
}

#[test]
fn russian_is_registered() {
    needs_catalogs!();
    let ru = Lang::from_code("ru").expect("ru registered");
    assert_eq!(ru.name(), "Русский");
    assert_eq!(normalize_pref("RU"), Some("ru"));
    assert_eq!(lang_from_tag("ru_RU.UTF-8"), Some(ru));
    assert_eq!(lang_from_tag("ru-BY"), Some(ru));
    assert_eq!(first_supported("ru-RU\r\nen-US"), Some(ru));
    assert_eq!(tr(ru, "Rapport de l'utilisateur.pdf"), "Rapport de l'utilisateur.pdf");
    let mut app = crate::PdfCraftApp::default();
    app.set_option("language", "ru").unwrap();
    assert_eq!(app.language, "ru");
    let mut restored = crate::PdfCraftApp::default();
    restored.restore(&app.persist());
    assert_eq!(restored.language, "ru");
}

/// The language list is well-formed. Problems in the embedded catalogs are not failed: craft-i18n
/// checks its rows, a row PdfCraft can't use shows in English, and `translation_gaps_report`
/// lists the `@id` rows PdfCraft doesn't use ([`unused_id_rows`]).
#[test]
fn bundled_catalogs_are_consistent() {
    let mut codes = std::collections::HashSet::new();
    for l in &LANGUAGES {
        assert!(codes.insert(l.code), "duplicate code {}", l.code);
        assert!(l.code == l.code.to_ascii_lowercase() && !l.code.is_empty(), "{}", l.code);
    }
    assert_eq!(LANGUAGES.first().map(|l| l.code), Some("en"), "English first: the source and the fallback");
    if !needs_catalogs() {
        return;
    }
    let problems = I18N.check_all();
    if !problems.is_empty() {
        println!("{} catalog rows are skipped (they show in English):\n{problems:#?}", problems.len());
    }
}

/// The `@id` rows of the embedded PdfCraft catalogs that are never shown, for the translation gaps
/// report: a row for a command PdfCraft doesn't have, and one that doesn't keep its label's
/// placeholders or trailing `…` ([`same_shape`]), whose label's translation shows instead. One
/// line each under a heading; empty when there are none.
fn unused_id_rows() -> String {
    let mut out = Vec::new();
    for lang in Lang::all().filter(|l| *l != Lang::EN) {
        for (_, id, translation) in rows(app_catalog(lang.code())).into_iter().filter(|(c, ..)| c == "@id") {
            if id == "language.name" {
                continue;
            }
            let Some(command) = pdfcraft_engine::commands::command(&id) else {
                out.push(format!("{}: @id {id:?} names a command Sintec.PDF doesn't have", lang.code()));
                continue;
            };
            if !same_shape(&translation, command.label) {
                let label = command.label;
                out.push(format!("{}: @id {id:?} doesn't keep the placeholders or ellipsis of {label:?}: {translation:?}", lang.code()));
            }
        }
    }
    if out.is_empty() {
        return String::new();
    }
    format!("{} @id rows not used (their label's translation shows):\n{}", out.len(), out.iter().map(|row| format!("  {row}\n")).collect::<String>())
}

/// The (context, English source) keys PdfCraft looks up: the UI source's literals, the commands,
/// All tools, the engine's own wordings, and every key of every embedded PdfCraft catalog (which
/// covers contextual, plural and dynamically looked-up strings the scan can't see).
fn english_keys() -> std::collections::BTreeSet<(String, String)> {
    use std::collections::BTreeSet;
    let mut plain: BTreeSet<String> = ui_literals();
    let mut add = |s: &str| {
        plain.insert(s.to_string());
    };
    for command in pdfcraft_engine::commands::COMMANDS {
        add(command.label);
        command.menu.into_iter().for_each(&mut add);
    }
    for group in pdfcraft_engine::catalog::TOOL_GROUPS {
        add(group.label);
        for section in group.sections {
            add(section.title);
            section.items.iter().for_each(|item| add(item.label));
        }
    }
    pdfcraft_engine::catalog::TOOL_CATEGORIES.iter().for_each(|category| add(category.label));
    let engine: [&[&str]; 9] = [
        pdfcraft_engine::a11y::WORDINGS,
        pdfcraft_engine::a11y::REPORT_WORDS,
        pdfcraft_engine::pdfa::WORDINGS,
        pdfcraft_engine::sign::pdf::WORDINGS,
        pdfcraft_engine::sign::pdf::kind::ALL,
        pdfcraft_engine::print::WORDINGS,
        pdfcraft_engine::EDIT_LABELS,
        pdfcraft_engine::BATCH_LABELS,
        pdfcraft_engine::field_table::WORDINGS,
    ];
    engine.into_iter().flatten().for_each(|s| add(s));
    pdfcraft_engine::sign::AppearanceLabels::default().fields().into_iter().for_each(|(_, label)| add(label));
    crate::credits::NameMode::ALL.iter().for_each(|mode| add(mode.label()));
    for key in crate::credits::SortKey::ALL {
        let (label, header) = key.label();
        add(label);
        add(header);
    }
    crate::credits::MODEL_COLUMNS.iter().for_each(|s| add(s));
    // (context, source) keys; `@id` rows only reword a command label, which is in `plain`.
    let mut keys: BTreeSet<(String, String)> = plain.into_iter().map(|s| (String::new(), s)).collect();
    for (_, _, app) in CRAFT_I18N {
        keys.extend(rows(app).into_iter().filter(|(context, ..)| context != "@id").map(|(context, source, _)| (context, source)));
    }
    keys
}

/// The translation gaps report (`cargo xtask i18n-report`, a step of `cargo xtask ci`): for
/// each language, how many English strings it has no translation for, and the first few, and how
/// many are kept in English (a row whose translation is the English itself). It never fails: a
/// missing string shows in English, and translators fill it in craft-i18n. Without embedded
/// catalogs it says so.
///
/// The English strings are the UI source's literals, the commands, All tools, the engine's
/// own wordings, and every key of every embedded PdfCraft catalog (which covers contextual,
/// plural and dynamically looked-up strings the scan can't see).
#[test]
fn translation_gaps_report() {
    const SHOWN: usize = 3;
    if !catalogs_embedded() {
        println!("{REPORT_START}\nTranslation gaps: skipped, built without the interface catalogs (i18n/locales/)\n{REPORT_END}");
        return;
    }
    let keys = english_keys();
    let problems = I18N.check_all();
    let mut report = format!("Translation gaps: {} English strings; {} catalog rows skipped (they show in English)\n", keys.len(), problems.len());
    for lang in Lang::all().filter(|l| *l != Lang::EN) {
        let found = |context: &str, source: &str| lang.i18n().get(lang.lang, context, source).map(str::to_owned);
        let missing: Vec<&(String, String)> = keys
            .iter()
            .filter(|(context, source)| match context.as_str() {
                "" => found("", source).is_none(),
                // A contextual lookup falls back to the plain translation.
                context => found(context, source).is_none() && (context == "@plural" || found("", source).is_none()),
            })
            .collect();
        let kept = keys.iter().filter(|(context, source)| context.is_empty() && found("", source).as_ref() == Some(source)).count();
        let mut line = format!("  {:<8} {:>5} missing, {kept:>4} kept in English", lang.code(), missing.len());
        for (i, (context, source)) in missing.iter().take(SHOWN).enumerate() {
            let mut shown: String = source.chars().take(48).collect();
            if shown.len() < source.len() {
                shown.push('…');
            }
            let key = if context.is_empty() { format!("{shown:?}") } else { format!("{context} / {shown:?}") };
            line.push_str(if i == 0 { ": " } else { ", " });
            line.push_str(&key);
        }
        if missing.len() > SHOWN {
            line.push_str(", …");
        }
        report.push_str(&line);
        report.push('\n');
    }
    report.push_str(&unused_id_rows());
    println!("{REPORT_START}\n{report}{REPORT_END}");
}

/// The lines `cargo xtask i18n-report` prints from the test output lie between these.
const REPORT_START: &str = "=== translation gaps ===";
const REPORT_END: &str = "=== end of translation gaps ===";

/// Preferences ▸ Date format ▸ Language offers exactly the interface languages, in order, with
/// the interface's names for them when the catalogs are embedded.
#[test]
fn date_languages_are_the_interface_languages() {
    let interface: Vec<&str> = LANGUAGES.iter().map(|l| l.code).collect();
    let dates: Vec<&str> = pdfcraft_engine::dates::DATE_LANGUAGES.iter().map(|l| l.code).collect();
    assert_eq!(dates, interface);
    needs_catalogs!();
    let interface: Vec<(&str, &str)> = Lang::all().map(|l| (l.code(), l.name())).collect();
    let dates: Vec<(&str, &str)> = pdfcraft_engine::dates::DATE_LANGUAGES.iter().map(|l| (l.code, l.name)).collect();
    assert_eq!(dates, interface);
}

/// The stale-draft notice remains discoverable after the UI modules are reorganized.
#[test]
fn properties_stale_notice_is_in_translation_scan() {
    assert!(ui_literals().contains("The document changed. Reopen Document Properties to edit the current version."));
}

/// A report, not a check: the notifications pasting an image and pinning a tool show, in each
/// language that has no translation for them yet (they show in English).
#[test]
fn image_paste_and_home_notifications_are_translated() {
    for language in Lang::all().filter(|l| *l != Lang::EN) {
        for source in ["Search…", "Image pasted", "No image on clipboard to paste", "Pinned {tool} to Home", "Unpinned {tool} from Home"] {
            if !has(language, source) {
                println!("{}: no translation for {source:?} (shows in English)", language.code());
            }
        }
    }
}
