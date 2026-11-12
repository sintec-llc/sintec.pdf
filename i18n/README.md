# Interface translations

Sintec.PDF's interface catalogs, in the format of [craft-i18n](https://github.com/storytold/craft-i18n)
(`docs/format.md` there): `locales/<code>/common.tsv` for strings shared by the Crafting Apps and
`locales/<code>/pdfcraft.tsv` for this app's own strings, which win over the shared ones.
`crates/ui-egui/build.rs` embeds every locale here that has a `pdfcraft.tsv`.

Sintec.PDF ships English and Russian. English is the source language: the strings in the code are
the lookup keys, so it has no catalog. `cargo test -p pdfcraft-ui-egui --test sintec_guard` fails
when a menu, tool or `tl!` label has no Russian. After editing a catalog, run
`cargo xtask assets --write` to refresh its SHA-256 in `ATTRIBUTION.toml`.

The Russian catalogs started from craft-i18n's `locales/ru/` at commit
`575cfdef63227b14daf173b8cc87928bfdfa325e` (MIT OR Apache-2.0, `licenses/craft-i18n-LICENSE-MIT.txt`),
with Sintec.PDF's own earlier catalog merged over it.
