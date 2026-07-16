# Install

ae is a single bash script. There's nothing to build and nothing to update except the script itself.

## Requirements

- [tmux](https://github.com/tmux/tmux)
- [git](https://git-scm.com/)
- bash ≥ 4.0
- At least one AI coding agent CLI on `PATH` (Claude Code, Codex, Gemini, Grok Build, OpenCode, or any other)

## One-line install

```bash
curl -fsSL https://raw.githubusercontent.com/clemens33/ae/main/install | bash
```

This clones the repo into `~/.local/share/ae` and symlinks `~/.local/share/ae/ae` to `~/.local/bin/ae`. Make sure `~/.local/bin` is on your `PATH`.

## From a local clone

```bash
git clone https://github.com/clemens33/ae.git ~/.local/share/ae
~/.local/share/ae/install
```

The `install` script handles both cases — if you run it from inside a clone, it just symlinks the local copy.

## Verify

```bash
ae doctor
```

Walks a fixed checklist: bash version, tmux/git on `PATH`, config file, registered agent executables, sessions directory, and so on. Prints `OK / WARN / FAIL` per line and exits non-zero if anything failed.

## Upgrading

ae is a single script behind a symlink. Pull the latest:

```bash
cd ~/.local/share/ae
git pull
```

Helper scripts in existing sessions auto-regenerate on the next `ae <name>` start or resume — no migration step. To force-refresh helpers without reattaching, run:

```bash
ae doctor --refresh         # all sessions
ae doctor --refresh my-fix  # one session
```

## Uninstall

```bash
rm ~/.local/bin/ae
rm -rf ~/.local/share/ae
rm -rf ~/.ae
```

`~/.ae/` holds your session state, config, and shared session memory. Remove it only if you really want a clean slate.
