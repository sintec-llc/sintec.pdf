//! The interface fonts with and without the optional craft-fonts build input (`CRAFT_FONTS_DIR`).

use egui::epaint::text::{Fonts, TextOptions};
use egui::{Color32, FontFamily, FontId};
use pdfcraft_ui_egui::theme;

const JAPANESE: &str = "日本語の文字";
const ARABIC: &str = "واحد اثنين";
const TELUGU: &str = "తెలుగు ఫైల్";

fn families() -> Vec<FontId> {
    vec![FontId::proportional(13.0), FontId::monospace(13.0), theme::medium(13.0), theme::semibold(17.0)]
}

/// Lays `text` out in every interface family and returns each galley's width.
fn layout_widths(fonts: &mut Fonts, text: &str) -> Vec<f32> {
    let mut view = fonts.with_pixels_per_point(2.0);
    families().into_iter().map(|id| view.layout_no_wrap(text.to_owned(), id, Color32::BLACK).size().x).collect()
}

/// Built with craft-fonts, Japanese text renders with real glyphs (no tofu) in every family,
/// from a craft-fonts face placed after the app's own fonts.
#[test]
fn japanese_ui_text_uses_craft_fonts() {
    if pdfcraft_fonts::ui_japanese_fonts().is_empty() {
        eprintln!("skipping japanese_ui_text_uses_craft_fonts: built without craft-fonts (set CRAFT_FONTS_DIR to run it)");
        return;
    }
    let defs = theme::font_definitions();
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let stack = &defs.families[&family];
        let first_jp = stack.iter().position(|n| n.starts_with("BIZ UDPGothic")).expect("BIZ UDPGothic is a fallback");
        let own = stack.iter().position(|n| n == "Inter" || n == "JetBrainsMono").expect("the app's own font");
        assert!(own < first_jp, "{family:?}: {stack:?}");
        assert!(stack[first_jp..].iter().all(|n| pdfcraft_fonts::CRAFT_FONTS.iter().any(|f| f.name() == *n)), "{stack:?}");
    }
    let mut fonts = Fonts::new(TextOptions::default(), defs);
    for id in families() {
        assert!(fonts.has_glyphs(&id, JAPANESE), "{id:?} lacks {JAPANESE}");
    }
    // Real glyphs are about a full em wide each; tofu boxes and missing glyphs are not.
    for w in layout_widths(&mut fonts, JAPANESE) {
        assert!(w > 13.0 * 0.8 * JAPANESE.chars().count() as f32, "{w}");
    }
}

/// Built with a craft-fonts Arabic face, Arabic text has real glyphs in every family, from a
/// face placed after the app's own fonts.
#[test]
fn arabic_ui_text_uses_craft_fonts() {
    let arabic: Vec<String> = pdfcraft_fonts::ui_arabic_fonts().iter().map(|f| f.name()).collect();
    if arabic.is_empty() {
        eprintln!("skipping arabic_ui_text_uses_craft_fonts: no Arab face bundled (set CRAFT_FONTS_DIR with an Arabic face to run it)");
        return;
    }
    let defs = theme::font_definitions();
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let stack = &defs.families[&family];
        let first_ar = stack.iter().position(|n| arabic.contains(n)).expect("an Arab face is a fallback");
        let own = stack.iter().position(|n| n == "Inter" || n == "JetBrainsMono").expect("the app's own font");
        assert!(own < first_ar, "{family:?}: {stack:?}");
    }
    let mut fonts = Fonts::new(TextOptions::default(), defs);
    for id in families() {
        assert!(fonts.has_glyphs(&id, ARABIC), "{id:?} lacks {ARABIC}");
    }
}

/// Built with a craft-fonts Telugu face, Telugu text has real glyphs in every family, from a
/// face placed after the app's own fonts, and is shaped: the conjunct క్ష is narrower than క్
/// and ష held apart by a zero-width non-joiner.
#[test]
fn telugu_ui_text_uses_craft_fonts() {
    let telugu: Vec<String> = pdfcraft_fonts::CRAFT_FONTS.iter().filter(|f| f.covers("Telu")).map(|f| f.name()).collect();
    if telugu.is_empty() {
        eprintln!("skipping telugu_ui_text_uses_craft_fonts: no Telu face bundled (set CRAFT_FONTS_DIR with a Telugu face to run it)");
        return;
    }
    let defs = theme::font_definitions();
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let stack = &defs.families[&family];
        let first_te = stack.iter().position(|n| telugu.contains(n)).expect("a Telu face is a fallback");
        let own = stack.iter().position(|n| n == "Inter" || n == "JetBrainsMono").expect("the app's own font");
        assert!(own < first_te, "{family:?}: {stack:?}");
    }
    let mut fonts = Fonts::new(TextOptions::default(), defs);
    for id in families() {
        assert!(fonts.has_glyphs(&id, TELUGU), "{id:?} lacks {TELUGU}");
    }
    let joined = layout_widths(&mut fonts, "క్ష");
    let apart = layout_widths(&mut fonts, "క్\u{200c}ష");
    for (j, a) in joined.iter().zip(&apart) {
        assert!(j < a, "క్ష is one conjunct, narrower than క్‌ష: {j} vs {a}");
    }
}

/// On a machine with a suitable installed font, the installed definitions end every family
/// with it and Arabic text has glyphs; the embedded-only definitions never name it.
#[test]
fn system_fallback_fills_missing_scripts() {
    assert!(!theme::font_definitions().font_data.contains_key(theme::SYSTEM_FALLBACK));
    let defs = theme::installed_font_definitions(theme::CjkPreference::Default);
    if !defs.font_data.contains_key(theme::SYSTEM_FALLBACK) {
        eprintln!("skipping system_fallback_fills_missing_scripts: no installed fallback font (or PDFCRAFT_SYSTEM_FONTS=0)");
        assert!(defs.families.values().all(|stack| !stack.iter().any(|n| n == theme::SYSTEM_FALLBACK)));
        return;
    }
    for (family, stack) in &defs.families {
        assert_eq!(stack.last().map(String::as_str), Some(theme::SYSTEM_FALLBACK), "{family:?}: {stack:?}");
        assert_eq!(stack.iter().filter(|n| *n == theme::SYSTEM_FALLBACK).count(), 1, "{family:?}");
    }
    let mut fonts = Fonts::new(TextOptions::default(), defs);
    for id in families() {
        assert!(fonts.has_glyphs(&id, ARABIC), "{id:?} lacks {ARABIC}");
        assert!(fonts.has_glyphs(&id, "Sintec.PDF"), "{id:?}");
    }
    assert!(layout_widths(&mut fonts, ARABIC).iter().all(|w| w.is_finite() && *w > 0.0));
}

/// On a machine with a suitable installed Devanagari font, the installed definitions include
/// it and Hindi text has glyphs; the embedded-only definitions never name it.
#[test]
fn system_fallback_covers_devanagari() {
    assert!(!theme::font_definitions().font_data.contains_key(theme::SYSTEM_DEVANAGARI_FALLBACK));
    let defs = theme::installed_font_definitions(theme::CjkPreference::Default);
    if !defs.font_data.contains_key(theme::SYSTEM_DEVANAGARI_FALLBACK) {
        eprintln!("skipping system_fallback_covers_devanagari: no installed Devanagari fallback font (or PDFCRAFT_SYSTEM_FONTS=0)");
        assert!(defs.families.values().all(|stack| !stack.iter().any(|n| n == theme::SYSTEM_DEVANAGARI_FALLBACK)));
        return;
    }
    for (family, stack) in &defs.families {
        assert!(stack.iter().any(|n| n == theme::SYSTEM_DEVANAGARI_FALLBACK), "{family:?}: {stack:?}");
        assert_eq!(stack.iter().filter(|n| *n == theme::SYSTEM_DEVANAGARI_FALLBACK).count(), 1, "{family:?}");
    }
    let mut fonts = Fonts::new(TextOptions::default(), defs);
    const DEVANAGARI: &str = "हिंदी पाठ सम्पादन";
    for id in families() {
        assert!(fonts.has_glyphs(&id, DEVANAGARI), "{id:?} lacks {DEVANAGARI}");
    }
    assert!(layout_widths(&mut fonts, DEVANAGARI).iter().all(|w| w.is_finite() && *w > 0.0));
}

/// On a machine with an installed Telugu font, the installed definitions include it and Telugu
/// text has glyphs, so the language's own name in Preferences is not boxes (#1531). macOS always
/// ships Kohinoor Telugu, so there the test never skips.
#[test]
fn system_fallback_covers_telugu() {
    assert!(!theme::font_definitions().font_data.contains_key(theme::SYSTEM_TELUGU_FALLBACK));
    let defs = theme::installed_font_definitions(theme::CjkPreference::Default);
    let off = std::env::var_os("PDFCRAFT_SYSTEM_FONTS").is_some_and(|v| v == "0");
    if !defs.font_data.contains_key(theme::SYSTEM_TELUGU_FALLBACK) && (off || !cfg!(target_os = "macos")) {
        eprintln!("skipping system_fallback_covers_telugu: no installed Telugu fallback font (or PDFCRAFT_SYSTEM_FONTS=0)");
        assert!(defs.families.values().all(|stack| !stack.iter().any(|n| n == theme::SYSTEM_TELUGU_FALLBACK)));
        return;
    }
    for (family, stack) in &defs.families {
        assert_eq!(stack.iter().filter(|n| *n == theme::SYSTEM_TELUGU_FALLBACK).count(), 1, "{family:?}: {stack:?}");
    }
    let mut fonts = Fonts::new(TextOptions::default(), defs);
    for id in families() {
        assert!(fonts.has_glyphs(&id, TELUGU), "{id:?} lacks {TELUGU}");
    }
    assert!(layout_widths(&mut fonts, TELUGU).iter().all(|w| w.is_finite() && *w > 0.0));
}

/// On a machine with installed Sinhala and Tamil fonts, the installed definitions include them
/// and Sinhala and Tamil text, such as a file name, has glyphs instead of boxes (#1688). macOS
/// always ships Sinhala Sangam MN and Tamil Sangam MN, so there the test never skips.
#[test]
fn system_fallback_covers_sinhala_and_tamil() {
    for (name, text) in [(theme::SYSTEM_SINHALA_FALLBACK, "ශ්‍රී ලංකා ගොනුව"), (theme::SYSTEM_TAMIL_FALLBACK, "தமிழ் கோப்பு")]
    {
        assert!(!theme::font_definitions().font_data.contains_key(name));
        let defs = theme::installed_font_definitions(theme::CjkPreference::Default);
        let off = std::env::var_os("PDFCRAFT_SYSTEM_FONTS").is_some_and(|v| v == "0");
        if !defs.font_data.contains_key(name) && (off || !cfg!(target_os = "macos")) {
            eprintln!("skipping {name}: no installed fallback font (or PDFCRAFT_SYSTEM_FONTS=0)");
            assert!(defs.families.values().all(|stack| !stack.iter().any(|n| n == name)));
            continue;
        }
        for (family, stack) in &defs.families {
            assert_eq!(stack.iter().filter(|n| *n == name).count(), 1, "{family:?}: {stack:?}");
        }
        let mut fonts = Fonts::new(TextOptions::default(), defs);
        for id in families() {
            assert!(fonts.has_glyphs(&id, text), "{id:?} lacks {text}");
        }
        assert!(layout_widths(&mut fonts, text).iter().all(|w| w.is_finite() && *w > 0.0));
    }
}

/// Without craft-fonts the interface fonts still install and lay out any text (Japanese falls
/// back to egui's replacement glyph) without panicking; Latin text is unaffected.
#[test]
fn ui_fonts_work_without_craft_fonts() {
    let mut fonts = Fonts::new(TextOptions::default(), theme::font_definitions());
    for id in families() {
        assert!(fonts.has_glyphs(&id, "Sintec.PDF"), "{id:?}");
        assert_eq!(fonts.has_glyphs(&id, JAPANESE), !pdfcraft_fonts::ui_japanese_fonts().is_empty(), "{id:?}");
    }
    assert!(layout_widths(&mut fonts, JAPANESE).iter().all(|w| w.is_finite() && *w > 0.0));
    assert!(layout_widths(&mut fonts, "Sintec.PDF").iter().all(|w| *w > 20.0));
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
}

/// The Russian catalog is drawn entirely by the app's own faces (Inter, JetBrains Mono): no
/// letter falls through to egui's defaults or a CJK fallback. (egui's `has_glyphs` can't answer
/// this: with only the primary face it is also the replacement face.)
#[test]
fn primary_ui_fonts_cover_the_russian_catalog() {
    use skrifa::MetadataProvider as _;
    let defs = theme::font_definitions();
    assert_eq!(&defs.families[&FontFamily::Monospace][..2], ["JetBrainsMono", "Inter"]);
    let fallback = &defs.font_data["Inter"];
    let fallback_font = skrifa::FontRef::from_index(&fallback.font, fallback.index).unwrap();
    let fallback_charmap = fallback_font.charmap();
    for (code, catalog) in [("ru", include_str!("../src/i18n/ru.tsv"))] {
        let mut text = String::from("áčďéěíňóřšťúůýžÁČĎÉĚÍŇÓŘŠŤÚŮÝŽãõçâêôàÃÕÇÂÊÔÀñÑüÜäÄöÖőűß„“¿¡«»…ґҐєЄіІїЇ");
        for line in catalog.lines().filter(|l| !l.starts_with('#')) {
            if let Some(translation) = line.split('\t').nth(2) {
                text.extend(translation.chars().filter(|c| !c.is_whitespace()));
            }
        }
        for name in ["Inter", "Inter-Medium", "Inter-SemiBold", "JetBrainsMono"] {
            let data = &defs.font_data[name];
            let font = skrifa::FontRef::from_index(&data.font, data.index).unwrap();
            let charmap = font.charmap();
            let mut missing: Vec<char> =
                text.chars().filter(|c| charmap.map(*c).is_none() && (name != "JetBrainsMono" || fallback_charmap.map(*c).is_none())).collect();
            missing.sort_unstable();
            missing.dedup();
            assert!(missing.is_empty(), "{code}: {name} lacks {missing:?}");
        }
    }
}
