## Decisions
- Only one run per project can go at a time, so no second run is started while one is running.
- The test build for 0.2.6 (tasks 001–006) is ready, with no GitHub release. The user wants to try the new features first and give the go-ahead.
- Until the user approves: no merge into main, no push to GitHub, no release.
- After approval: merge into main, push, and update the release. The version stays 0.2.6 unless the user asks for another number.
- When merging worker branches, verify that the test fix `adc207c1` (`page.scan` in `PICKERS`) isn't lost.
- For the 004 conflict, the assistant took the 002 version of `panels.rs` and ported 004's two lines (bundled fonts) into `text_panel.rs`.
- The "Scan using ECOSYS" window came from the autotests, not the app. The fix was to add `page.scan` to `PICKERS`. Scanning is still tested in `tests/scan.rs` with mock data.
- The assistant offered to make the "Scan another page" dialog optional: the scan would insert the page and show a notification, and a repeat scan would be started by a panel button. The user hasn't answered yet.
- Tasks 007 (tab switching) and 009 (performance) belonged to another project and were cancelled. Branch `auto/007` and its task and state entries were deleted. The old performance 009 was deleted before it started.
- Task 008 (stability audit and bugfixes) runs on branch `auto/008` from `feature/0.2.6`. Scope: panics (`unwrap`, indexing), hangs, background job races, leaks, unbounded caches, and closing a tab during work. Each fix needs a regression test, and features don't change.
- Task 009 (print paper auto-fit setting) is done on branch `auto/009` (commit 07abc4d9). It adds a Preferences setting, off by default (A4 stays the default). Checks: fmt, clippy, cargo test. The worker recorded paper-size paths in `reports/009.md`.
- When the user asks for "status", the assistant reads the reports, checks the diffs, runs the checks, merges the branches and builds a new test build.
- The assistant doesn't edit the repository for the tab-merge bug itself. The user edits the working copy. The assistant only investigates and creates tasks on request.

## Facts about the project
- Project: sintec-pdf, a PDF editor with text boxes, tabs, font handling and printing. Working branch: `feature/0.2.6`.
- Tasks 001–006 are done and merged into `feature/0.2.6`:
  - 001: font analysis WIP.
  - 002: floating text panel like Photoshop.
  - 003: bigger drag handle for text boxes.
  - 004: bundled substitute fonts (from `craft-fonts`). The patch is at `sintec-pdf/wip-0.2.7/004-partial.patch`.
  - 005: tab tear-off as an independent window and process. Its log shows it was blocked after `hard` (fmt and `pdfcraft-ui-egui` test failures, 10-16 19:51). No `reports/005.md` exists, and the log shows no fix.
  - 006: identify fonts by glyph widths.
- Task 008 is marked done in the queue, but its report is incomplete ("watchers armed, will report once gates finish"). Gates and fuzzer are unconfirmed. It touches `pdfcraft-ui-egui`. Not merged.
- Task 009 (`auto/009`, 07abc4d9): the worker reports that paper did not auto-fit before its change. The only path was the "Match page at 100%" button (`print_ui.rs`, around lines 835–850), plus "Auto portrait/landscape" orientation. The assistant found no auto-fit on dialog open. The user may have seen a different path, which must be checked by hand in the build. Not merged.
- Branches `auto/001`–`auto/006` are merged and can be deleted.
- Tab-merge regression reported by the user: dragging a tab onto another window's tab strip no longer joins it. Logic lives in `tear_off.rs` and `doc_windows.rs`. Cause unknown.
- Test build 0.2.6 is in `C:\Users\kochurin.a\sintec-pdf\.claude\worktrees\sintec-pdf\dist\release\`:
  - `sintec-pdf-0.2.6-windows-x64-setup.exe` (167 MB)
  - `sintec-pdf-0.2.6-windows-x64.msi`
  - `sintec-pdf-0.2.6-windows-x64-portable.zip`
- The build is unsigned because signing secrets aren't set in the assistant's environment. Windows may show a SmartScreen warning. The assistant doesn't know how the user signed 0.2.6 earlier.
- The assistant has not installed any build on the user's PC.
- Nothing has been merged into main or pushed to GitHub. No new build has been run since 001–006.
- Scan feature: the "Page scanned / Scan another page" dialog (`Dialog::ScanMore`, `scan_ui.rs:125`) opens after every inserted page when no other dialog is open. It closes with the "Готово" button.
- A folder `agent-worktrees\007` can't be deleted ("Permission denied"). A stopped worker process probably holds it. It is an empty copy with no commits.
- Session disconnects ("Stream closed", "previous session ended") kill background tasks. A disconnect mid tool call shows the user "(no answer)". The cause is on the interface or connection side.
- The assistant doesn't monitor runs automatically. It acts only when the user writes.
- fmt, clippy and all tests were green after the 001–006 merges.

## Open questions
- Build composition, which the assistant asked the user to pick (the user said "готовь билд"):
  1. Only 001–006, the same as the previous test build. Most predictable.
  2. With 009, not 008, until 008's final report exists.
  3. With 008 and 009. The assistant would first check `auto/008` gates; if they're not green, no build.
  4. Wait until 008 finishes and reports.
  The assistant recommended option 1 or 2 for a quick build. No answer yet.
- Whether 009 is the auto-fit the user meant, or whether the user saw auto-fit at dialog open through another path.
- Whether the user approves the test build, which leads to merge into main, push and release.
- Whether the user wants a signed build, and how 0.2.6 was signed.
- Whether the assistant should install the build on the user's PC.
- Manual checks still needed:
  - Text panel shows "installed / not installed" and the font list correctly (after the 004 port).
  - Drag handle for text boxes (003).
  - Real tab tear-off drag gesture (005).
  - Font identification by glyph widths (006).
  - Bundled substitute fonts (004).
- Tab-merge regression: what exactly happens (no join, new window, or nothing); whether it fails in the 0.2.6 test build, the repo build, or both; whether it started after the 008 merges or earlier. If the user confirms, a task would find the cause, fix it and add a regression test for moving a tab into another window.
- Whether to make the "Scan another page" dialog optional.
- Whether the version stays 0.2.6.
- Removing the leftover `agent-worktrees\007` folder once the process releases it.

## User preferences
- The user wants every task checked and run.
- The user expects the assistant to resolve blocked or paused tasks itself.
- The user wants a local build first, to try the new features by hand. Merge, push, PR and release happen only after explicit approval.
- The user writes in Russian (sometimes English). The assistant replies in Russian.
- For a "status" request, the user wants a brief answer.
- The user reports odd behavior as it happens and leaves it open on purpose for the assistant to investigate.
- The user asks directly about assistant failures, such as "(no answer)", and expects an honest explanation.
- The user wants thorough checkups and backchecking of the app.
- The user sometimes sends messages meant for another project and apologizes. The assistant adjusts and drops that work.
