# Interface language

Sintec.PDF's interface is in **English** and **Russian**. Choose **Menu > Edit > Preferences…** (Command-comma on macOS, Ctrl-comma elsewhere) **> Interface language** and select **Auto**, **English** or **Русский**. The change applies immediately and persists between launches. Command ids, document contents and file names are unchanged.

Displayed keyboard shortcuts use the active interface language. Russian keeps the key-cap names (`Ctrl+K`, `Ctrl+Shift+S`) and translates only names that differ, such as `Пробел` for Space; macOS keeps its modifier symbols (`⌘`, `⇧`, `⌃`). Key names have their own `key` catalog context, so a key name can read differently from the command with the same English word. A missing key-name entry falls back to the English key name. Actual bindings and machine-facing command ids remain stable.

**Auto** (the default) follows the system language: `LC_ALL`, `LC_MESSAGES` or `LANG`, then the preferred-languages list on macOS. Any Russian locale (`ru`, `ru_RU`, `ru_BY`, `ru_KZ`, …) uses the Russian catalog, and so do Ukrainian and Kazakh ones (`uk`, `uk_UA`, `kk`, `kk_KZ`), whose users mostly read Russian too; every other language shows English. On Windows, **Auto** uses the Windows display languages, in preference order, when no supported locale is selected by those environment variables. Neutral `C`/`POSIX` locale values (including `C.UTF-8`) inherited from a launcher do not override the Windows display language; explicitly named supported locales such as `ru_RU.UTF-8` still do. The query asks Windows directly (`GetUserPreferredUILanguages`, through the `sys-locale` crate) without starting a process, and is cached for the process. If detection fails, Sintec.PDF shows English.

A language saved by an older version that shipped more languages (for example `ja` or `uk`) is no longer recognised, so the app follows the system language again.

The control channel exposes the setting through `ui.set`:

```json
{"method":"ui.set","params":{"key":"language","value":"ru"}}
```

The value is `auto`, `en` or `ru`, in any case. `ui.state` reports `language` as written in the preferences. Unknown values return an error without changing the current setting. Preferences saved before this setting existed follow the system language.

Dialogs, panels, menus, notices and history labels go through the catalog (`tl!`). The Russian catalog (about 2,190 entries) translates every registered command, every All tools entry and every `tl!("literal")` in the UI source; the tests fail when a new label is missing. Error details that come from the engine or the operating system are shown as they are, inside a translated frame («Удалить страницы: ошибка: …»). The command palette matches the translated label, the English label and the command id.

Russian plurals use the three CLDR forms: `1 страница`, `2 страницы`, `5 страниц`; 11–14 take the third form, while 21 and 22 return to the first and second. Where a count would have to agree with a noun that doesn't fit a plural entry, the text is written count-neutral («Выбрано страниц: 3»). Dates in a sentence use genitive month names (`7 марта 2026`), while calendar headings use the nominative (`Март 2026`). Russian needs only the bundled Inter and JetBrains Mono faces (tested in `tests/fonts.rs`).

File names and document titles in other scripts (Japanese, Chinese, Arabic, Hebrew, Telugu) still display: the optional [craft-fonts](https://github.com/storytold/craft-fonts) build input supplies fallback faces, and on desktop a character no embedded face has is drawn with one font already installed on the machine (`PDFCRAFT_SYSTEM_FONTS=0` turns this off). The tab strip, the recent-files list and Properties show right-to-left names in display order.

## How translations work

The system is the one PhotoCraft uses (`crates/ui-egui/src/i18n/`):

- Strings in the code stay in English and serve as lookup keys. At draw time, `tl!("Save")` returns the text in the current language, and falls back to English when there is no entry.
- The Russian catalog is `crates/ui-egui/src/i18n/ru.tsv`, with one entry per line: `context<TAB>English<TAB>translation`. An empty context marks a plain string. `@id` keys an entry by command id (for example `file.saveAs`), so one menu item can read differently from another with the same English label. `@plural` holds plural forms. Any other context disambiguates an English word with several meanings (`tl_ctx!` in UI code, for example `date` for month names inside a date). The header of `ru.tsv` documents the format and escapes.
- Placeholders (`{name}`) must appear in both columns. A trailing `…` must be kept. `i18n::fmt` fills placeholders in one pass, so a value such as a file name containing `{n}` is inserted as written.
- Catalogs are validated strictly: unknown escapes, unknown `@` contexts, placeholder or ellipsis mismatches, wrong plural form counts and duplicates are errors. The tests require the bundled catalog to have none; at run time a bad line is skipped (and logged) and that string shows in English.
- Catalogs are assets: each `.tsv` has an `ATTRIBUTION.toml` entry with `kind = "translation"` and its SHA-256, checked by `cargo xtask assets`.
- Translations are clean-room: write them from the meaning of the English text, never from another product's string tables.

## Adding a language

Sintec.PDF ships English and Russian only. Should another language be needed:

1. Copy the header of `ru.tsv` into `xx.tsv` and translate entries.
2. Add one row to `LANGUAGES` in `crates/ui-egui/src/i18n/mod.rs` (code, native name, catalog and plural rule) and the language's month and weekday names to `DATE_LANGUAGES` in `crates/engine/src/dates.rs`.
3. Add an `ATTRIBUTION.toml` entry for the catalog (`kind = "translation"`), then run `cargo xtask assets --write` and `cargo xtask assets`.

The Preferences dropdown, the system-language match and the catalog tests (format, duplicates, placeholders, plural forms, command ids) then pick it up.
