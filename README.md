# symskill

Terminal UI and CLI for managing symlinked agent skills.

`symskill` treats one or more directories of `SKILL.md` packages as the source of truth and creates or removes per-skill symlinks in agent skill directories.

Link targets:

```text
~/.claude/skills
~/.codex/skills
~/.cursor/skills
```

Claude and Codex are the default link targets. Cursor also reads skills from `~/.claude/skills` and `~/.codex/skills`, so direct `~/.cursor/skills` links are optional and usually unnecessary when either Claude or Codex links are enabled.

## Usage

`--source` is required and must come before any subcommand. Repeat the flag or pass a comma-separated list to use several skill roots:

```bash
cargo run -- --source ~/skills
cargo run -- --source ~/skills/work,~/skills/personal
cargo run -- --source ~/skills/work --source ~/skills/personal
```

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

Link statuses are shown as symbols: `●` linked, `·` not linked, `▲` wrong target, `✗` blocked.
The panel below the table shows the selected skill's full description, its source path, and
the target of any wrong or blocked link. The header shows total link counts across all agents.

## Safety

- `link` refuses to overwrite any existing path.
- `unlink` only removes symlinks that point to the selected source skill.
- Real directories, files, and symlinks to other targets are shown as blocked or wrong.
- `wrong` means an existing symlink points to a different source than the current skill source. Use `fix` or `f` to replace those symlinks with links to the current source.
- CLI commands refuse to act when a skill name matches multiple source directories.
- Unreadable directories and malformed `SKILL.md` files are reported as warnings instead of aborting discovery.
