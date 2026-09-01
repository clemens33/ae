# Install

ae has one public command: an immutable Rust core with the Bash pane glue it still needs.

## Requirements

- [tmux](https://github.com/tmux/tmux)
- [git](https://git-scm.com/)
- bash ≥ 4.0
- At least one AI coding agent CLI on `PATH` (Claude Code, Codex, Gemini, Grok Build, OpenCode, or any other)

Checkout prerequisites: [rustup](https://rustup.rs/) and [just](https://github.com/casey/just) installed; then run `just rust-setup` once to provision the pinned toolchain.

## Checkout install (active until the first Rust-era release tag)

```bash
git clone https://github.com/clemens33/ae.git ~/.local/share/ae
cd ~/.local/share/ae
just install
```

This runs the canonical installer from a checkout. It publishes a versioned immutable artifact under `~/.ae/versions` and atomically points `~/.local/bin/ae` at its public wrapper. Make sure `~/.local/bin` is on your `PATH`.
Checkout install compiles a native binary for this machine; static musl is a property of CI-built release bundles once the first Rust-era tag exists.

## One-line release install — activates with the first release tag

This is the release interface, shown now for reference. **Do not run it before the first Rust-era release tag and bundle exist.** It activates with that tag.

```bash
curl -fsSL https://raw.githubusercontent.com/clemens33/ae/main/install | bash
```

It supports macOS Apple Silicon (`darwin-arm64`) and Linux x86_64, including
WSL2. Intel macOS, Linux ARM, and Windows/MSYS are rejected. The bundle and
`SHA256SUMS` are downloaded to temporary files and verified before extraction;
the matched four-member set — `ae`, `ae-core`, `ae-glue`, and `install` — is then
published atomically under `~/.ae/versions`. Set `AE_VERSION=2026.8.2` to pin a release.

## Verify

```bash
ae doctor
```

Walks a fixed checklist: bash version, tmux/git on `PATH`, config file, registered agent executables, sessions directory, and so on. Prints `OK / WARN / FAIL` per line and exits non-zero if anything failed.

## Upgrading

```bash
ae upgrade
```

`ae upgrade` invokes its immutable sibling installer. It downloads the latest release (or an `AE_VERSION` pin), verifies checksums before extraction, installs a new immutable version, then atomically repoints the public wrapper and `core/current`.

`ae upgrade` exists now for repair and local fixture use; remote latest releases and `AE_VERSION` pins activate only after the first Rust-era tag. Until then, update the checkout and rerun `just install`.

Stopped sessions consume the current version only on their next resume. Running sessions are reported by name and deferred until stop and resume; an upgrade never hot-rewrites their loaded helpers or daemon bodies.

## Uninstall

**Stop every ae session first.** Running sessions pin `ae_path` to `~/.ae/versions/V/ae-glue`; deleting immutable versions while they run strands their helpers (`spawn`, `retire`, `watchdog`, and re-entry). The default uninstall removes only the public selector and leaves immutable version leaves in place:

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
