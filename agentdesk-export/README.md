# AgentDesk export: Sintec.PDF

State of the AgentDesk project at release 0.3.1 (tasks 001-036 done).

To continue in another AgentDesk:
1. Clone this repo and check out the code (branch `export/0.3.1` or `main`), put the `craft-fonts` checkout next to it as in CI.
2. Create a project "sintec-pdf" in the new AgentDesk and copy `tasks/`, `reports/`, `state.json`, `notes.md`, `chat_memory.md`, `project.toml` into its project folder.
3. Edit `project.toml`: `[repo] path`, `worktrees`, `[gates.env] CARGO_TARGET_DIR` are Windows paths of the old machine; `[git] account` is the GitHub account name.
4. Not included on purpose: `signing/` (secrets), chat history, run logs.
