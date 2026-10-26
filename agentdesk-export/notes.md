# Project notes for agents (durable facts; appended by finished tasks)

- Sintec.PDF is a fork of PdfCraft (storytold/pdfcraft); interface languages are English and Russian only. Every new UI string needs a ru.tsv line; after editing ru.tsv run `python zz_rehash.py crates/ui-egui/src/i18n/ru.tsv`.
- Never commit fonts, PDFs from Downloads (they hold credentials) or corpus files. Fonts come from craft-fonts or `[[fetched]]` entries.
- Never panic in non-test code (CLAUDE.md): no unwrap/expect/indexing with input-derived positions; bound loops and allocations on PDF input.
- Font editing: FontFacts (crates/edit/src/text.rs) reads the embedded program's family/weight; PDFs often name fonts `CIDFont+F1`. Installed faces are embeddable when OS/2 fsType allows it (crates/fonts/src/embedding.rs validate_document_input).
- Gates: cargo fmt --all --check; cargo clippy (-D warnings); tests. UI tests use egui_kittest (crates/ui-egui/tests/).
- (001) Working-tree files are CRLF (core.autocrlf=true); edit with `newline=''` in scripts, and `ru.tsv` rows need a leading TAB (empty context column).
- (001) Tests that need a known installed-font set use `app.fonts.installed_override`; the editing harnesses set it to `standard_faces()`.
- (006) `Match` gained `Fingerprint` (Other < Alias < Fingerprint < Exact); any new `match` on it must handle that variant.
- (006) `FontFile` and `Wanted` struct literals now need `widths` (`FontFile` has no `Default`).
- (006) `ru.tsv` in this tree is LF, not CRLF as the brief said; check for `\r` before assuming a line ending.
- (003) In Edit text & images, `object_ui::page_input` runs before `edit_text_ui::page_input` and turns any press on empty page into a marquee, so new handles outside a box's rect must be exempted there (see `on_move_grip`).
- (003) Grip constants and hit logic: `MOVE_GRIP`, `MOVE_GRIP_SLOP`, `move_handle`, `handle_at` in `edit_text_ui.rs`.
- (004) Caladea is Latin-only; the other four families have Cyrillic.
- (004) `ru.tsv` has mixed LF/CRLF endings near the end. Edit it with byte-level inserts so the CRLF lines survive.
- (004) Tinos's licence comes from `googlefonts/tinos` because google/fonts has none for it.
- (009) Nothing in print_ui.rs picked the paper automatically before 009; only the explicit "Match page at 100%" button did.
- (009) `PrintDraft::match_page()` is the shared match logic; `PdfCraftApp::match_paper_to_page` gates it in `open_print()`.
- (009) Editing `ru.tsv` changes the sha256 in `ATTRIBUTION.toml`, so rerun `zz_rehash.py` after each edit.
- (010) Cross-process tab moves live in `crates/ui-egui/src/tab_transfer.rs`. The registry is in `RecoveryStore::default_dir().parent()/Windows`, and published rectangles are in physical screen pixels: (point + window origin) × pixels-per-point.
- (010) The app holds the listener in `PdfCraftApp.tab_peer` (not on wasm). Tests use `Peer::start_as(registry, fake_pid, wake)`; a test that drags to x=1600 in a 1400-point harness counts as "outside the window".
- (010) `tear_off.rs::hand_off_what` (file or recovery entry, plus page) is shared by tear-off and `send_tab`. `control::random_token` is now `pub(crate)`.
- (011) A press outside a text object's rect starts an `object_ui` marquee unless `edit_text_ui::on_move_grip` returns true. Any new grab zone outside the box must be added there.
- (011) Tests that edit text need `app.fonts.installed_override = Some(standard_faces())`. With an empty list the click opens the font-substitution flow, not the editor.
- (011) The floating "Format text" panel sits over the right edge of an open box and takes the pointer there.
- (012) The draft's paper default is A4 (595.28 × 841.89), not `PAPERS[0]`, which is US Letter. Orientation Auto in the print crate drives the Windows driver paper choice via the sheet size.
- (012) `ui-egui` keeps `print_ui` private; tests reach the draft through `PdfCraftApp::new().print_draft`.
- (013) `TextBlock.rect` is the text extent, not the box width. Handle-drag tests need wrapped multi-row text; a Courier fixture with 12 pt word steps works.
- (013) `DocView::set_zoom` is the way tests change zoom; `text_panel_rect()` shows where the panel is.
- (014) Block `rect` is always the text extent. The user-dragged box width lives only in `DocView.kept_width` and is applied to the cached `edit_lines` by `edit_text_ui::keep_width`; it is dropped on the next document-generation change.
- (014) `DocView::shown_text_boxes(page)` returns the boxes as shown (kept width included); tests should use it, because `doc.text_blocks` has the raw extent.

- 2026-10-21: user decided: after the upstream (0.9.0) sync, the release number is 0.3.0 (not 0.2.7 again). 0.2.7 was built/published first. Tasks 016-019 produce the sync; 019 must bump version to 0.3.0.
- (020) `tests/support/mod.rs` is the shared test module. Include it with `#[path = "support/mod.rs"] mod support;`.
- (020) Any test that clicks existing text must set `app.fonts.installed_override = Some(standard_faces())`.
- Gate {crates} expands to bare dir names (-p engine) which fail for pdfcraft-* packages on merge-state worktrees; quick gate set to fmt + clippy --workspace (2026-10-22). Task 021-023 split of v0.9.0 merge; auto/016 = merged 0.8.0 + 0.2.7.

## Lessons (2026-10-22, from the 0.2.7 release and the v0.9.0 merge)
- stop_run makes the auto-runner see "run is gone" and RESTART the task (counts as a crash, 3 crashes = auto-retry used up, and the cost of every restart is real). Never stop a run just to fix something; before add_task/stop, check that new tasks have `depends` (a task without depends starts at once from the base branch, not from the work I intended). After a stop, start the task with run_task by hand.
- A task builds its branch from its depends' branch. To give a dependent task my manual work, merge it into the dependency's branch (branch -f auto/NNN) BEFORE starting it.
- Do the prep (merge, tests, base branch) first, add the dependent tasks after; and write depends right in the first add_task.
- Gate {crates} expands to bare directory names (-p engine) and fails for pdfcraft-* packages; quick gate is fmt + clippy --workspace. Always sanity-check a gate command by hand once before a long run.
- Huge merges (v0.8.0, v0.9.0, ~250 commits) cost $15+ each as one task: split by area (core compile / text UI / the rest) and keep the previous failed attempt's cost in mind (old 017: $16 wasted).
- CI: macOS tests can fail where Windows passes (title bar 28 pt band, Ctrl vs Cmd); fix and wait for ALL four checks green BEFORE merging a PR; do not open a PR while a previous one is still open on the same branch.
- Never merge a PR with red checks; the user's other sessions may merge PRs early, so re-check PR state before pushing more commits to its branch.
- git push to sintec-llc fails with 403 under the janmaslovitz account: use `-c credential.helper= -c "credential.helper=!gh auth git-credential"` (active gh account is sintec-llc).
- Release: push sintec/main to sintec release branch; draft appears when all jobs pass. macOS arm64 runner can fail to be acquired ("capacity constraints"): rerun failed jobs (`gh run rerun <id> --failed`), took 4 tries on 2026-10-21.
- Signing: Windows is signed with Sintec LLC internal cert (secrets in `release` env; docs/internal-signing.md; files in Documents\Sintec.PDF Signing - don't open or copy them). macOS has no Apple secrets: ad-hoc, not notarized.
- Use a separate CARGO_TARGET_DIR (target-coworker) for my own cargo runs; target-agents once got a corrupt PDB. Python on this PC defaults to cp1251: use PYTHONUTF8=1 and encoding='utf-8'.
- Don't claim a state (e.g. "no signing") without checking every platform/log; the Windows job log showed signing was on.

## 2026-10-22
- User: after task 024 build a LOCAL build 0.3.0 (version bump from 0.2.7 to 0.3.0). Nothing pushed to GitHub, no release, no PR.
- Run_all started 10-22 evening: 022 -> 023 -> 024 -> 018 -> 019. auto/021 (merge of PdfCraft v0.9.0, fmt+clippy ok, tests unconfirmed) is the base of 022; not yet merged into feature/0.2.6.
- Plan after 024: review branches, merge chain into feature/0.2.6, bump version to 0.3.0, run checks, build via packaging (dist/release), no push.
- (019) `cargo xtask version set` doesn't update the internal `pdfcraft-* = { version = … }` requirements in the root `Cargo.toml`. Bump them by hand when the minor version changes.
- (019) The changelog entry for the work developed as 0.2.7 is filed under 0.3.0.
- (024) Reduce levels live only in `pdfcraft_engine::optimizer::Level` (`settings()`, `discard()`); `docs/file-size-levels.md` repeats the table and must change with it. `Session::reduced_bytes` now takes a `Level`; `estimate_job`/`EstimateJob::run` give the estimates.
- (024) `optimize::Settings` has a new `recompress_streams` field (default on) and `Stage::Streams`; `SaveOptions` has a new `best_compression` field. The progress-bar fractions changed (images take 0.05–0.70).
- (024) `remove_hidden` returns `NothingToApply` when nothing matched; the engine's `discard_hidden` treats that as "nothing discarded". Reuse it anywhere a discard is optional.
- (025) `text_lines` splits wide-gap TJ runs (for editing); `drawn_lines` doesn't, and the redaction audit uses `drawn_lines`. Their line numbers differ.
- (025) The ui-egui tests that type Arabic only run when built with `CRAFT_FONTS_DIR` (craft-fonts checkout at `C:/Users/kochurin.a/sintec-pdf/craft-fonts`); use a separate target dir for those builds.
- (025) Preferences now has a single scroll area, inside `js_ui::preferences_body`; `dialogs.rs` no longer wraps it.
- 10-22 night: tasks 018-025 done and fast-forwarded into feature/0.2.6 (7f63fccd, version 0.3.0, local only, not pushed). Local UNSIGNED build 0.3.0 in dist/release (setup.exe, msi, portable.zip); signing cert folder "Documents\Sintec.PDF code signing" does not exist on this PC. Full cargo test was green except 5 tests fixed by 025 (not re-run in full after). Not installed on user's PC. Needs manual check by user.
- (028) Cross-check Linux from Windows: `CC_x86_64_unknown_linux_gnu` pointing at a `zig cc -target x86_64-linux-gnu.2.35` wrapper that strips `--target=` (aws-lc-sys needs a C compiler), then `cargo clippy --target x86_64-unknown-linux-gnu`.
- (028) The Linux freedesktop id is `io.github.sintec_llc.sintec_pdf` and the packages install `sintec-pdf`/`sintec-pdf-cli`, with data in `share/sintec-pdf`; Windows and macOS keep `APP_ID = ai.storyteller.pdfcraft`.
- (028) `.tmp026/*.jpg` committed by task 026 fails `cargo xtask assets` on every CI job until removed.
- (030) kittest `Node.click()` uses the last frame's rect, so after opening a modal dialog run about 12 steps first; the Preferences dialog moves for several frames.
- (030) `tests/content.rs` failed once in a full-package run and passed on re-runs, so it may be flaky.

- Signing secrets + instructions: C:\Users\kochurin.a\AppData\Local\AgentDesk\projects\sintec-pdf\signing (pfx, PFX-PASSWORD.txt, SIGNING.md). Outside git. Original: Documents\Sintec.PDF Signing.
- (032) The print draft (`PrintDraft`) is not persisted. Orientation lives only in the in-memory dialog state.
- (033) `pdfcraft_engine::substitute_fit(facts, face_bytes)` is the one fitting rule for a substitute face, used by both the preview and the commit; `Document::substitute_chars` says which letters the commit draws in the substitute.
- (033) `BlockStyle::substitutes` (original font resource → loaded `PCFull…` resource) beats `registered_substitute`'s guess. Compare styles with `BlockStyle::is_plain()`, not `== default()`.
- (033) In egui 0.36, `FontTweak.scale` only scales how glyphs are drawn, not how far they advance, so spacing has to be adjusted per letter (as `editor_job` does).
