# Keeping disk usage down

A checkout that has been used by several agents and worktrees can reach hundreds of GB. On 2026-10-21 one reached 534 GB. About 99% of it was Cargo build output (`target*/`), and the rest was agent worktrees. Everything below is regenerable; the cost is a rebuild.

## Where the space goes

| What | Typical size | Safe to delete? |
|---|---|---|
| `target/debug/deps` | 10–200 GB (grows with every dependency or feature change; old hashes are never removed) | Yes, when no build is running |
| `target-*/` (per-agent or per-task `CARGO_TARGET_DIR`s) | 15–115 GB each | Yes, when no build uses it |
| `agent-worktrees/*/target`, `agent-worktrees/target-*` | 15–20 GB each | Yes |
| `.claude/worktrees/*` | 0.03 GB each plus any `target` | Only after the branch is pushed or merged |
| `target/debug/incremental` | a few GB | Yes |

## Check first

1. Is a build running? A delete under a live build breaks it.
   `Get-CimInstance Win32_Process -Filter "Name='cargo.exe' OR Name='rustc.exe'" | Select CommandLine`
2. Is a worktree mid-merge or dirty? `git -C <worktree> status --porcelain`. Never delete a worktree with changes or unpushed commits (`git log --oneline @{u}..` or `origin/main..`).
3. Largest directories:
   `Get-ChildItem -Force -Directory | % { "{0} {1:N1}GB" -f $_.Name, ((gci $_.FullName -r -Force -File -EA 0 | measure Length -Sum).Sum/1GB) } | sort`

## Clean up

Delete build caches that nothing is using:

```powershell
Remove-Item -LiteralPath target-OLD -Recurse -Force
cargo clean            # the default target/
cargo clean --target-dir target-agents
```

Remove finished worktrees (this does not touch branches):

```powershell
git worktree list
git worktree remove agent-worktrees/NNN      # add --force only if you are sure it is disposable
git worktree prune
```

Delete a stale target directory by age, for example anything not written in 3 days:

```powershell
Get-ChildItem -Directory -Filter 'target*' | ? { $_.LastWriteTime -lt (Get-Date).AddDays(-3) } | Remove-Item -Recurse -Force
```

`LastWriteTime` of the directory itself can be misleading; check the newest file inside before relying on it.

## Prevent regrowth

- **One shared target per machine, not per agent.** Set `CARGO_TARGET_DIR` to one directory outside the repo (for example `C:\cargo-target`). Concurrent builds wait on its lock but share compiled dependencies, so ten agents cost 20 GB instead of 200 GB. Only give an agent its own target when it must build in parallel; delete it when the task ends.
- **Keep targets out of the repo tree**, so they are not scanned by editors, antivirus or `git status`.
- **Do not leave `target` inside agent worktrees.** When an agent finishes, remove its worktree and any target it made.
- **Trim debug info.** In `Cargo.toml`, `[profile.dev] debug = "line-tables-only"` (or `debug = false`) and `[profile.dev.package."*"] debug = false` cut `target/debug` by roughly half and speed up linking.
- **Sweep regularly.** `cargo install cargo-sweep`, then `cargo sweep --time 14` removes artifacts unused for 14 days from the current target (run after a build to also use `--installed`).
- **Weekly routine:** `git worktree prune`, delete finished `agent-worktrees/*` and `target-*`, and run `cargo sweep --time 7` on the shared target.
- **Watch the number.** `target/debug/deps` above about 30 GB means stale hashes have piled up; sweep or clean.
