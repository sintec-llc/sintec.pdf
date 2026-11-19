# Moving the project to another AgentDesk

The repository holds the code. The work queue (tasks, reports, notes, state, chat) lives in
AgentDesk's own project folder and is **not** in git:

    %LOCALAPPDATA%\AgentDesk\projects\sintec-pdf

Use AgentDesk's built-in export and import. Do not copy that folder by hand.

## Export (old machine)

1. No run may be going (`queue_status` says idle). Stop one first.
2. Run:

       "C:\Program Files\AgentDesk\AgentDesk.exe" export sintec-pdf <file>.zip

   Add `--logs` only if the run logs are wanted. The zip holds `project.toml`, `notes.md`,
   `state.json`, `tasks/`, `reports/` and the chat.
3. **Signing is not in that zip.** Copy the `signing\` folder of the project folder (code-signing
   `.pfx`, `.cer`, `PFX-PASSWORD.txt`, `SIGNING.md`) into it, as `signing/`, only if the user asks
   for it. Then the zip holds a private key and its password: keep it private, never commit it,
   never attach it to a release or a pull request.
4. Code goes by git: push the branch to GitHub (the repository account is `sintec-llc`).

## Import (new machine)

1. Clone the repository and check out `main`. Put a `craft-fonts` checkout next to it as in CI
   (see `AGENTS.md` §1.4).
2. Run:

       "C:\Program Files\AgentDesk\AgentDesk.exe" import-file <file>.zip --repo <path of the clone> [--name sintec-pdf] [--account <account>]

3. Open `project.toml` of the new project and fix what belongs to the old machine: `[repo] path`,
   `worktrees`, `[gates.env] CARGO_TARGET_DIR`, `[git] account`.
4. Put `signing/` from the zip into the new project folder by hand. `import-file` is not known to
   restore it.
5. Check: `projects` lists it, `status sintec-pdf` shows the tasks and the queue is idle.

## Rules for agents

- Never put secrets (signing files, tokens, passwords) in git, a report, `notes.md` or a chat
  message. Name the place they live, never their content.
- Do not delete the old project folder until the user confirms the new AgentDesk works.
- Build caches (`target*`) are not part of the move; they are rebuilt.
