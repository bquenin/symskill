# symskill

Terminal UI and CLI for managing symlinked agent skills.

`symskill` treats one or more directories of `SKILL.md` packages as the source of truth and creates or removes per-skill symlinks in agent skill directories.

Link targets:

```text
~/.claude/skills
~/.codex/skills
~/.cursor/skills
```

Claude and Codex are the default link targets. Cursor always scans `~/.claude/skills` and `~/.codex/skills` as well (this cannot be disabled), so any skill linked for Claude or Codex is automatically available in Cursor. The cursor column reflects this: such skills show `○` in the TUI and `inherit` in `list` output. Direct `~/.cursor/skills` links are only needed for a skill linked to neither Claude nor Codex.

## Install

```bash
brew install bquenin/symskill/symskill
```

Or build from source with `cargo install --path .`.

## Usage

`--source` is required and must come before any subcommand. Repeat the flag or pass a comma-separated list to use several skill roots:

```bash
cargo run -- --source ~/skills
cargo run -- --source ~/skills/work,~/skills/personal
cargo run -- --source ~/skills/work --source ~/skills/personal
```

## Discovery

Each source is crawled recursively; any directory containing a `SKILL.md` (matched case-insensitively, so `skill.md` works too) is a skill. Hidden directories, `node_modules`, `__pycache__`, and `target` are skipped.

Nested skills are grouped by the directory that contains them. Directories shared by every skill in a source collapse away, so `--source repo` and `--source repo/skills` produce the same view:

```text
skills/
  jira/SKILL.md            -> jira
  ops/
    db-restore/SKILL.md    -> ops/db-restore
    pager-triage/SKILL.md  -> ops/pager-triage
```

The TUI shows each group as a section heading with its skills indented below; `list` prints the qualified `group/name`. CLI commands accept the bare skill name, or `group/name` when the bare name is ambiguous. Symlinks are always created flat as `<agent skills dir>/<name>`, regardless of nesting.

With no subcommand, the TUI launches. List skills and link status:

```bash
cargo run -- --source ~/skills list
```

Link or unlink one skill:

```bash
cargo run -- --source ~/skills link jira --agent claude --agent codex
cargo run -- --source ~/skills unlink jira --agent claude
cargo run -- --source ~/skills toggle jira --agent cursor
cargo run -- --source ~/skills fix jira --agent claude --agent codex
```

## TUI Keys

- `j` / `k` or arrow keys: move selection
- `1`: toggle Claude link
- `2`: toggle Codex link
- `3`: toggle direct Cursor link
- `a`: toggle all targets
- `f`: fix wrong symlinks for the selected skill
- `r`: reload
- `q`, `Esc`, or `Ctrl+C`: quit

Link statuses are shown as symbols: `●` linked, `·` not linked, `○` inherited (available to Cursor through a claude/codex link), `▲` wrong target, `✗` blocked.
The panel below the table shows the selected skill's full description, its source path, and
the target of any wrong or blocked link. The header shows total link counts across all agents.

## Safety

- `link` refuses to overwrite any existing path.
- `unlink` only removes symlinks that point to the selected source skill.
- Real directories, files, and symlinks to other targets are shown as blocked or wrong.
- `wrong` means an existing symlink points to a different source than the current skill source. Use `fix` or `f` to replace those symlinks with links to the current source.
- CLI commands refuse to act when a skill name matches multiple source directories; use `group/name` to disambiguate.
- Unreadable directories and malformed `SKILL.md` files are reported as warnings instead of aborting discovery.
