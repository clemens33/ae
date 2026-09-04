# Install

ae has one public command: `~/.local/bin/ae` is a symlink straight to an immutable,
versioned Rust core binary. There is no wrapper in between.

## Requirements

- [tmux](https://github.com/tmux/tmux)
- [git](https://git-scm.com/)
- bash ≥ 4.0
- At least one AI coding agent CLI on `PATH` (Claude Code, Codex, Gemini, Grok Build, OpenCode, or any other)

## One-line release install

```bash
curl -fsSL https://raw.githubusercontent.com/clemens33/ae/main/install | bash
```

It supports macOS Apple Silicon (`darwin-arm64`) and Linux x86_64, including
WSL2. Intel macOS, Linux ARM, and Windows/MSYS are rejected. The bundle and
`SHA256SUMS` are downloaded to temporary files and verified before extraction;
the matched three-member set — `ae-core`, `install`, and `SHA256SUMS` — is then
published read-only under `~/.ae/versions/<V>/` (the version directory and its
executable members 0555, the manifest 0444, so nothing can write through them
once installed), and `~/.local/bin/ae` is pointed straight at that version's
`ae-core` — a plain symlink, and the only pointer: there is no separate
`core/current` or `~/.ae/current` to keep in sync. Switching versions later is
one atomic rename of that symlink. Make sure `~/.local/bin` is on your `PATH`.
Set `AE_VERSION=2026.8.2` to pin a release.

## Build from source

Prerequisites: [rustup](https://rustup.rs/) and [just](https://github.com/casey/just) installed; then run `just rust-setup` once to provision the pinned toolchain.

```bash
git clone https://github.com/clemens33/ae.git ~/.local/share/ae
cd ~/.local/share/ae
just install
```

This runs the same canonical installer from a checkout, publishing a versioned immutable artifact under `~/.ae/versions` and atomically pointing `~/.local/bin/ae` at that version's `ae-core`. It compiles a native binary for this machine; static musl is a property of the CI-built release bundles.

## Verify

```bash
ae doctor
```

Walks a fixed checklist: tmux/git on `PATH`, config file, registered agent executables, sessions directory, and so on. The bash-version row went with ae's last bash; what doctor checks about the core itself is that a PUBLISHED one is not writable. Prints `OK / WARN / FAIL` per line and exits non-zero if anything failed.

## Upgrading

```bash
ae upgrade
```

`ae upgrade` downloads the latest release (or an `AE_VERSION` pin), verifies checksums before extraction, publishes a new immutable version read-only under `~/.ae/versions/<V>/`, then atomically repoints `~/.local/bin/ae` directly at that version's `ae-core`. The whole of that is the core's own code — it needs nothing on the machine but `tar`, and works on an installed generation that is otherwise too broken to run.

Stopped sessions consume the current version only on their next resume. Running sessions are reported by name and deferred until stop and resume; an upgrade never hot-rewrites their loaded helpers or daemon bodies.

## Uninstall

**Stop every ae session first.** Running sessions pin `ae_core` to `~/.ae/versions/V/ae-core`; deleting immutable versions while they run strands their helpers (`spawn`, `retire`, `watchdog`, and re-entry). The default uninstall removes only the public selector and leaves immutable version leaves in place:

```bash
# Default: remove only the public selector; keep immutable version leaves.
rm ~/.local/bin/ae
```

After confirming that all ae sessions are stopped, remove immutable leaves if you also want to reclaim installed generations:

```bash
# Safe only after every ae session is stopped.
rm -rf ~/.ae/versions
```

`~/.ae/` holds your session state, config, and shared session memory; these commands do not remove that data.
