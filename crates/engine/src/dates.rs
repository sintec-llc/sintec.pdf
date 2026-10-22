//! The date format Fill & Sign stamps (Preferences ▸ Date format), kept on the [`Session`] so
//! the app and the automation tools write the same text. Acrobat's Fill & Sign has no such
//! setting (only Acrobat Sign's account settings do); this one is per user and takes any
//! pattern in Acrobat's date letters, not just a fixed list, with month and weekday names in
//! any of [`DATE_LANGUAGES`].
//!
//! Patterns use `util.printd`'s date letters: `yyyy` `yy`, `mmmm` `mmm` `mm` `m`, `dddd` `ddd`
//! `dd` `d`. `\` shows the next character as is (`d \de mmmm` → `8 de October`).

use crate::Session;

/// The format until the user picks one (Acrobat's: US month/day/year).
pub const DEFAULT_DATE_FORMAT: &str = "m/d/yyyy";

/// Ready-made formats for Preferences ▸ Date format. Any other valid pattern works too.
pub const DATE_FORMATS: [&str; 12] = [
    "m/d/yyyy",
    "mm/dd/yyyy",
    "d/m/yyyy",
    "dd/mm/yyyy",
    "dd.mm.yyyy",
    "yyyy-mm-dd",
    "yyyy.mm.dd.",
    "d mmm yyyy",
    "d mmmm yyyy",
    "mmm d, yyyy",
    "mmmm d, yyyy",
    "dddd, mmmm d, yyyy",
];

/// The longest pattern kept (settings and tool arguments are untrusted).
pub const MAX_DATE_FORMAT_CHARS: usize = 64;

/// Month and weekday names in one language. Weekdays start on Sunday.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DateLanguage {
    /// The interface-language code (`en`, `ru`).
    pub code: &'static str,
    /// Its name in itself, for pickers.
    pub name: &'static str,
    pub months: [&'static str; 12],
    /// The month after a day number (`mmmm` when the pattern also shows `d` or `dd`), where
    /// the language changes it: Russian `8 октября`.
    pub months_with_day: Option<[&'static str; 12]>,
    pub months_short: [&'static str; 12],
    pub days: [&'static str; 7],
    pub days_short: [&'static str; 7],
}

/// The languages month and weekday names can be written in (Preferences ▸ Date format ▸ Language):
/// the interface languages, in the same order and with the same names.
pub const DATE_LANGUAGES: [DateLanguage; 2] = [
    DateLanguage {
        code: "en",
        name: "English",
        months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
        months_with_day: None,
        months_short: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
        days: ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"],
        days_short: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    },
    DateLanguage {
        code: "ru",
        name: "Русский",
        months: ["январь", "февраль", "март", "апрель", "май", "июнь", "июль", "август", "сентябрь", "октябрь", "ноябрь", "декабрь"],
        months_with_day: Some(["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"]),
        months_short: ["янв.", "февр.", "март", "апр.", "май", "июнь", "июль", "авг.", "сент.", "окт.", "нояб.", "дек."],
        days: ["воскресенье", "понедельник", "вторник", "среда", "четверг", "пятница", "суббота"],
        days_short: ["вс", "пн", "вт", "ср", "чт", "пт", "сб"],
    },
];

/// The date language for `code` (any case), or `None` when there's none.
pub fn date_language(code: &str) -> Option<&'static DateLanguage> {
    DATE_LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code.trim()))
}

/// The characters of `text` that Fill & Sign can't write into a PDF yet, each once. Its text is
/// drawn in Helvetica with WinAnsi encoding (Western European letters), so anything else (`ř`,
/// Cyrillic, Japanese) would become `?` in the file.
pub fn unwritable(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let mut buf = [0u8; 4];
        if c != '?' && pdfcraft_fonts::win_ansi(c.encode_utf8(&mut buf)) == b"?" && !out.contains(c) {
            out.push(c);
        }
    }
    out
}

/// `fmt` trimmed, when it's a usable date pattern: not too long, showing at least a year,
/// month or day, and no time letters (`H h M s t`) unless escaped with `\`.
pub fn check_date_format(fmt: &str) -> Result<&str, String> {
    let fmt = fmt.trim();
    if fmt.is_empty() {
        return Err("the date format is empty".into());
    }
    if fmt.chars().count() > MAX_DATE_FORMAT_CHARS {
        return Err(format!("the date format is longer than {MAX_DATE_FORMAT_CHARS} characters"));
    }
    let letters: String = unescaped(fmt).filter_map(|(c, escaped)| (!escaped).then_some(c)).collect();
    if let Some(c) = letters.chars().find(|c| matches!(c, 'H' | 'h' | 'M' | 's' | 't')) {
        return Err(format!("`{c}` is a time letter (H h M s t); put \\ before it to show it as is"));
    }
    if !letters.chars().any(|c| matches!(c, 'y' | 'm' | 'd')) {
        return Err("the date format shows no year (y), month (m) or day (d)".into());
    }
    // Only whole codes: `yyy` or `mmmmm` would silently show something else.
    for (c, n, _) in runs(fmt).into_iter().filter(|r| !r.2) {
        let fits = match c {
            'y' => n == 2 || n == 4,
            'm' | 'd' => (1..=4).contains(&n),
            _ => true,
        };
        if !fits {
            let code: String = std::iter::repeat_n(c, n).collect();
            let use_ = match c {
                'y' => "yy or yyyy",
                'm' => "m, mm, mmm or mmmm",
                _ => "d, dd, ddd or dddd",
            };
            return Err(format!("`{code}` is not a date code; use {use_}"));
        }
    }
    Ok(fmt)
}

/// `fmt` as runs of the same character with whether it's escaped: `dd.\mm` → (`d`, 2, no),
/// (`.`, 1, no), (`m`, 1, yes), (`m`, 1, no). Escaped characters are never grouped.
fn runs(fmt: &str) -> Vec<(char, usize, bool)> {
    let mut runs: Vec<(char, usize, bool)> = Vec::new();
    for (c, escaped) in unescaped(fmt) {
        match runs.last_mut() {
            Some((last, n, false)) if !escaped && *last == c => *n += 1,
            _ => runs.push((c, 1, escaped)),
        }
    }
    runs
}

/// The day `(year, month, day)` written in `fmt` with `lang`'s names. Anything but the date
/// letters is copied as is; a month or day out of range shows as its number.
pub fn format_day(fmt: &str, (y, m, d): (i64, u32, u32), lang: &DateLanguage) -> String {
    let runs = runs(fmt);
    let with_day = runs.iter().any(|&(c, n, escaped)| !escaped && c == 'd' && n <= 2);
    let month = (m as usize).checked_sub(1);
    let weekday = weekday(y, m, d);
    let mut out = String::new();
    for (c, n, escaped) in runs {
        match (c, n) {
            _ if escaped => out.push(c),
            ('y', 4..) => out.push_str(&format!("{:04}", y.clamp(0, 9999))),
            ('y', _) => out.push_str(&format!("{:02}", y.rem_euclid(100))),
            ('m', 3..) => {
                let names = if n >= 4 { lang.months_with_day.filter(|_| with_day).unwrap_or(lang.months) } else { lang.months_short };
                match month.and_then(|i| names.get(i)) {
                    Some(name) => out.push_str(name),
                    None => out.push_str(&m.to_string()),
                }
            }
            ('m', 2) => out.push_str(&format!("{m:02}")),
            ('m', _) => out.push_str(&m.to_string()),
            ('d', 3..) => {
                let names = if n >= 4 { lang.days } else { lang.days_short };
                match weekday.and_then(|w| names.get(w)) {
                    Some(name) => out.push_str(name),
                    None => out.push_str(&d.to_string()),
                }
            }
            ('d', 2) => out.push_str(&format!("{d:02}")),
            ('d', _) => out.push_str(&d.to_string()),
            _ => out.extend(std::iter::repeat_n(c, n)),
        }
    }
    out
}

/// The weekday (0 = Sunday) of a Gregorian date in years 1–9999, or `None` for an invalid one.
fn weekday(y: i64, m: u32, d: u32) -> Option<usize> {
    const T: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    if !(1..=9999).contains(&y) || !(1..=31).contains(&d) {
        return None;
    }
    let t = *T.get((m as usize).checked_sub(1)?)?;
    let y = y - i64::from(m < 3);
    Some((y + y / 4 - y / 100 + y / 400 + t + i64::from(d)).rem_euclid(7) as usize)
}

/// The characters of `fmt`, each with whether a `\` before it makes it literal.
fn unescaped(fmt: &str) -> impl Iterator<Item = (char, bool)> + '_ {
    let mut chars = fmt.chars();
    std::iter::from_fn(move || match chars.next()? {
        '\\' => Some(chars.next().map_or(('\\', true), |c| (c, true))),
        c => Some((c, false)),
    })
}

impl Session {
    /// Preferences ▸ Date format: the pattern Fill & Sign dates use. Rejects unusable patterns
    /// (see [`check_date_format`]) and keeps the previous one.
    pub fn set_date_format(&mut self, fmt: &str) -> Result<(), String> {
        let fmt = check_date_format(fmt)?;
        self.date_format = (fmt != DEFAULT_DATE_FORMAT).then(|| fmt.to_string());
        Ok(())
    }

    pub fn date_format(&self) -> &str {
        self.date_format.as_deref().unwrap_or(DEFAULT_DATE_FORMAT)
    }

    /// Preferences ▸ Date format ▸ Language: the language of month and weekday names, or
    /// `None` to follow the interface language. Rejects codes not in [`DATE_LANGUAGES`].
    pub fn set_date_language(&mut self, code: Option<&str>) -> Result<(), String> {
        self.date_language = match code {
            None => None,
            Some(c) => Some(date_language(c).map(|l| l.code.to_string()).ok_or_else(|| {
                let codes: Vec<&str> = DATE_LANGUAGES.iter().map(|l| l.code).collect();
                format!("unknown date language {c:?} (use one of {})", codes.join(", "))
            })?),
        };
        Ok(())
    }

    pub fn date_language(&self) -> Option<&str> {
        self.date_language.as_deref()
    }

    /// Today's local date in `fmt` (or the session's date format) with the names of `lang`
    /// (or the session's date language; English when neither is set or known).
    pub fn today_text(&self, fmt: Option<&str>, lang: Option<&str>) -> Result<String, String> {
        let fmt = match fmt {
            Some(f) => check_date_format(f)?,
            None => self.date_format(),
        };
        let lang = lang.or(self.date_language()).and_then(date_language).unwrap_or(&DATE_LANGUAGES[0]);
        Ok(format_day(fmt, self.today(), lang))
    }

    /// [`Session::today_text`] for writing into a PDF: refused when the date has characters
    /// Fill & Sign can't write yet (see [`unwritable`]), rather than saving them as `?`.
    pub fn today_text_for_pdf(&self, fmt: Option<&str>, lang: Option<&str>) -> Result<String, String> {
        let text = self.today_text(fmt, lang)?;
        match unwritable(&text) {
            bad if bad.is_empty() => Ok(text),
            bad => Err(format!(
                "\"{text}\" can't be written into the PDF yet: Fill & Sign text is Western European only ({bad}); pick a numeric format such as dd.mm.yyyy"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: (i64, u32, u32) = (2026, 3, 7);
    const EN: &DateLanguage = &DATE_LANGUAGES[0];

    fn in_lang(fmt: &str, code: &str) -> String {
        format_day(fmt, DAY, date_language(code).unwrap())
    }

    #[test]
    fn presets_write_the_day_in_their_own_order() {
        let shown: Vec<String> = DATE_FORMATS.iter().map(|f| format_day(f, DAY, EN)).collect();
        assert_eq!(
            shown,
            [
                "3/7/2026",
                "03/07/2026",
                "7/3/2026",
                "07/03/2026",
                "07.03.2026",
                "2026-03-07",
                "2026.03.07.",
                "7 Mar 2026",
                "7 March 2026",
                "Mar 7, 2026",
                "March 7, 2026",
                "Saturday, March 7, 2026",
            ]
        );
        assert!(DATE_FORMATS.iter().all(|f| check_date_format(f).is_ok()));
        assert_eq!(DATE_FORMATS[0], DEFAULT_DATE_FORMAT);
    }

    #[test]
    fn escaped_letters_are_shown_as_is() {
        assert_eq!(format_day("d \\de mmmm \\de yyyy", DAY, EN), "7 de March de 2026");
        assert_eq!(format_day("yyyy\\", DAY, EN), "2026\\");
        assert_eq!(check_date_format("d \\de mmmm"), Ok("d \\de mmmm"));
    }

    #[test]
    fn unusable_patterns_are_rejected() {
        assert!(check_date_format("   ").is_err());
        assert!(check_date_format("Date: dd/mm").unwrap_err().contains("`t`"));
        assert!(check_date_format("mm/dd HH:MM").is_err());
        assert!(check_date_format("Year").is_err(), "no date letters");
        assert!(check_date_format(&"d".repeat(MAX_DATE_FORMAT_CHARS + 1)).is_err());
        assert_eq!(check_date_format("  yyyy-mm-dd "), Ok("yyyy-mm-dd"));
    }

    #[test]
    fn only_whole_date_codes_are_accepted() {
        for (fmt, code) in [("yyyyyyyy", "`yyyyyyyy`"), ("yyy", "`yyy`"), ("d/m/y", "`y`"), ("mmmmm d", "`mmmmm`"), ("ddddd", "`ddddd`")] {
            let e = check_date_format(fmt).unwrap_err();
            assert!(e.contains(code) && e.contains("is not a date code"), "{fmt}: {e}");
        }
        for fmt in ["yy", "yyyy", "m", "mm", "mmm", "mmmm", "d", "dd", "ddd", "dddd", r"\y\y\y yyyy"] {
            assert!(check_date_format(fmt).is_ok(), "{fmt}");
        }
    }

    #[test]
    fn hostile_days_and_patterns_do_not_panic() {
        for fmt in ["mmmm", "mmm", "dddd", "ddd", "\\", "yyyyyyyyy", "é d ü", "d. mmmm"] {
            for day in [(i64::MAX, u32::MAX, u32::MAX), (i64::MIN, 0, 0), (2026, 13, 40), (0, 2, 29)] {
                for lang in &DATE_LANGUAGES {
                    let _ = format_day(fmt, day, lang);
                }
            }
        }
        assert_eq!(format_day("mmmm dddd", (2026, 13, 40), EN), "13 40", "out of range shows numbers");
    }

    #[test]
    fn the_session_stamps_today_in_its_format() {
        // 2023-11-14 (UTC) on the injected clock.
        let mut s = Session::new().with_clock(|| 1_700_000_000);
        assert_eq!(s.date_format(), DEFAULT_DATE_FORMAT);
        assert_eq!(s.today_text(None, None).as_deref(), Ok("11/14/2023"));
        s.set_date_format("dd.mm.yyyy").unwrap();
        assert_eq!(s.today_text(None, None).as_deref(), Ok("14.11.2023"));
        assert_eq!(s.today_text(Some("yyyy-mm-dd"), None).as_deref(), Ok("2023-11-14"));
        assert!(s.today_text(Some("HH"), None).is_err());
        assert!(s.set_date_format("nothing").is_err());
        assert_eq!(s.date_format(), "dd.mm.yyyy", "a rejected pattern keeps the previous one");
    }

    /// 21:30 on 14 Oct 2026 in São Paulo (UTC-3) is 00:30Z on the 15th. The date Fill & Sign
    /// stamps is the wall-clock day (14th); the time the file records (`/M`, `/ModDate`) is UTC.
    /// The test shortcut (`with_clock`) takes the injected time as local and says the 15th, which
    /// is why it cannot serve a host that knows both the UTC time and the zone.
    #[test]
    fn a_wall_clock_gives_the_local_day_and_keeps_utc_for_the_file() {
        let sao_paulo = Session::new().with_wall_clock(|| 1_792_024_200, || -3 * 3600);
        assert_eq!(sao_paulo.today(), (2026, 10, 14));
        assert_eq!(sao_paulo.today_text(None, None).as_deref(), Ok("10/14/2026"));
        assert_eq!(sao_paulo.signing_date(), "D:20261014213000-03'00'");
        assert_eq!(sao_paulo.stamp_by_line("Ada"), "By Ada at 9:30 pm, Oct 14, 2026");
        assert_eq!(sao_paulo.now_secs(), 1_792_024_200, "the file's instant stays in UTC");

        let utc = Session::new().with_wall_clock(|| 1_792_024_200, || 0);
        assert_eq!(utc.today(), (2026, 10, 15));

        let clock_only = Session::new().with_clock(|| 1_792_024_200);
        assert_eq!(clock_only.today(), (2026, 10, 15), "on its own, an injected clock counts as local time");
    }

    #[test]
    fn names_follow_the_date_language() {
        assert_eq!(in_lang("dddd, d mmmm yyyy", "ru"), "суббота, 7 марта 2026");
        assert_eq!(in_lang("ddd d mmm", "ru"), "сб 7 март");
        assert_eq!(in_lang("dddd, mmmm d", "en"), "Saturday, March 7");
        assert_eq!(date_language("RU").map(|l| l.code), Some("ru"));
        assert!(date_language("xx").is_none());
        assert!(date_language("de").is_none(), "only the interface languages");
        for l in &DATE_LANGUAGES {
            assert!(l.months.iter().chain(&l.months_short).chain(&l.days).chain(&l.days_short).all(|n| !n.is_empty()), "{}", l.code);
        }
    }

    #[test]
    fn russian_months_change_after_a_day() {
        assert_eq!(in_lang("d mmmm yyyy", "ru"), "7 марта 2026");
        assert_eq!(in_lang("mmmm yyyy", "ru"), "март 2026");
        // A weekday (dddd) is not a day number.
        assert_eq!(in_lang("dddd, mmmm", "ru"), "суббота, март");
    }

    #[test]
    fn the_session_writes_names_in_its_date_language() {
        let mut s = Session::new().with_clock(|| 1_700_000_000);
        s.set_date_format("dddd d mmmm").unwrap();
        assert_eq!(s.today_text(None, None).as_deref(), Ok("Tuesday 14 November"), "English by default");
        assert_eq!(s.today_text(None, Some("ru")).as_deref(), Ok("вторник 14 ноября"), "a caller's language");
        s.set_date_language(Some("RU")).unwrap();
        assert_eq!(s.date_language(), Some("ru"));
        assert_eq!(s.today_text(Some("mmmm yyyy"), None).as_deref(), Ok("ноябрь 2023"));
        let e = s.set_date_language(Some("xx")).unwrap_err();
        assert!(e.contains("en, ru"), "only the interface languages: {e}");
        assert_eq!(s.date_language(), Some("ru"), "an unknown language keeps the previous one");
        s.set_date_language(Some("EN")).unwrap();
        assert_eq!(s.today_text_for_pdf(None, None).as_deref(), Ok("Tuesday 14 November"));
        s.set_date_language(None).unwrap();
        assert_eq!(s.date_language(), None);
    }

    #[test]
    fn dates_the_pdf_cannot_hold_are_refused_not_damaged() {
        assert_eq!(unwritable("8. října 2026"), "ř");
        assert_eq!(unwritable("14 ноя"), "ноя");
        assert_eq!(unwritable("sábado, 7 de março – 2026?"), "", "Western European, dashes and ? are fine");
        let mut s = Session::new().with_clock(|| 1_700_000_000);
        s.set_date_format("d mmmm yyyy").unwrap();
        assert_eq!(s.today_text_for_pdf(None, Some("en")).as_deref(), Ok("14 November 2023"));
        let e = s.today_text_for_pdf(Some("dddd"), Some("ru")).unwrap_err();
        assert!(e.contains("вторник") && e.contains("can't be written into the PDF yet"), "{e}");
        assert!(s.today_text_for_pdf(Some("dd.mm.yyyy"), Some("ru")).is_ok(), "numbers are fine in any language");
    }
}
