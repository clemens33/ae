# Pre-P5 coexistence: `ae-next` beside `ae`

**Status:** scope ratified (five cross-model rounds) → built (items 1-7) → hardened through three
cross-model code-review rounds (13 BLOCKER, 4 IMPORTANT, 3 NIT folded in; final verdict SHIP) →
**landed on `rust-rewrite`** → **canary in progress** (2026-08-30: outcomes 1, 2 and 4 PASS;
3 awaits a test bot — see the checklist). Update this line as it moves.

## Decision (binding, 2026-08-30)

The installed `ae` stays exactly as it is: `~/.local/bin/ae` → the `main` checkout, state in
`~/.ae`, the default tmux server, the bash Telegram bridge. **Nothing in this slice touches it.**

A second command, **`ae-next`**, runs the `rust-rewrite` hybrid (bash glue + Rust core):

| Rule | Shape |
|---|---|
| Isolated state | `AE_HOME=~/.ae-next` — config, sessions, archive, worktrees, locks, Telegram offset. The wrapper also pins `CONFIG_FILE=$AE_HOME/config`, so an inherited `CONFIG_FILE` cannot redirect it |
| Isolated tmux | named server `ae-next` (`AE_TMUX_SERVER=ae-next`, kind `name`); a second instance takes another name via `AE_NEXT_TMUX_SERVER` with its own `AE_NEXT_HOME` — parallel canaries. **The overrides cannot alias the installed instance**: the name `default` and a home that resolves to `~/.ae` (or whose final root is a symlink) are refused by installer and wrapper alike |
| Immutable core | installed at `$AE_HOME/core/<version>/ae`, directory and binary `0555` (`$AE_HOME/core/` itself stays writable so the next version can be added); **never `target/release` in place** |
| Core authority | **next-owned and persistent**: the installer writes `$AE_HOME/core/current` (one line, an expanded absolute path), and `ae` itself reads it — `_ae_core_bin_input` resolves `AE_CORE_BIN` → `$CONFIG_DIR/core/current` → `workspace.core`. Because `AE_HOME` already propagates to every re-entry (launch env, `tmux set-environment`, the compact child command, helper and watchdog re-exec of `ae_path`), every path resolves the intended immutable version with **no environment to go stale**. The wrapper unsets `AE_CORE_BIN` and never exports it. On the wrapper path no config source can repoint the core, because the wrapper refuses to run unless `core/current` names an absolute executable; `ae` itself falls through to `workspace.core` only when the pointer is absent or unusable (missing, relative, symlinked), and the installer never writes that key |
| Ownership | a session is managed only by the command that created it — structurally, since neither command can see the other's state dir or server |
| Telegram | one bot token is never polled by two bridges at once: the seeded config carries **no reusable credentials**, and the bridge spawn is **guarded** (below) |
| Not P5 | no change to `~/.local/bin/ae`, no installer default flip, no retirement |

**End-state intent (a later, separate GO):** promote `ae-next` to `ae`; keep today's `ae` as
`ae-legacy` for rollback. Not designed here.

## What already exists (verified in the tree; nothing below is new work)

| Mechanism | Where | Notes |
|---|---|---|
| `AE_HOME` relocates all state | `ae` top (`CONFIG_DIR="$AE_HOME"`), launch env, `tmux set-environment` | the isolated integration harness runs on exactly this |
| `CONFIG_FILE` is env-overridable; a cwd `.ae/config` is read after it, last match wins | `ae:58`, `AE_LOCAL_CONFIG`, `get_config` | why coexistence-owned facts must not live in config |
| `AE_TMUX_SERVER` + `AE_TMUX_SERVER_KIND` ambient shim | `_ae_install_tmux_shim`, installed unconditionally at startup | session meta records `tmux_server`/`tmux_server_kind`; helpers re-export them; the watchdog re-invokes `telegram _supervise` on the session's server; the Rust core's `tmux.rs` emits `-L <name>`. Contract row SC-1410c |
| Core resolution: `AE_CORE_BIN`, then `$CONFIG_DIR/core/current` (this slice), then config `workspace.core`; never `PATH`; **no `~` expansion**; a symlinked `current` is refused before reading | `_ae_core_bin_input`, `_ae_core_current_input` | the persistent file is what makes the installed core authoritative on every re-entry; a `~` path would pin an unusable core |
| Sessions pin the core | `_ae_core_bind` writes `ae_core` + `ae_core_version`; `_ae_core_try` re-verifies path + `--version` on every call | mismatch or unbound → bash body with one visible complaint; `compact` alone hard-requires the core |
| Telegram spawn chokepoint | `_telegram_spawn_daemon`, the only spawn path for `start`, autostart and `_supervise` | its "already running" check sees only its own tmux server — a named-server instance is independent of the default-server bridge, which is exactly the double-poll hazard |

The motivating exhibit for the immutable path: at the time of writing `target/release/ae`
reports `0.2.18` while the tree is `0.2.20`. An in-place build is a binary that changes
underneath the sessions that pinned it; the integration suite hit exactly that as spurious
`compact` failures during a concurrent rebuild.

## The slice

1. **`contrib/ae-next/ae-next`** — the tracked wrapper (bash, shellcheck-covered), symlinked as
   `~/.local/bin/ae-next`. It:
   - sets `AE_HOME="${AE_NEXT_HOME:-$HOME/.ae-next}"` and `CONFIG_FILE="$AE_HOME/config"`
     (overriding anything inherited);
   - sets `AE_TMUX_SERVER="${AE_NEXT_TMUX_SERVER:-ae-next}"`, `AE_TMUX_SERVER_KIND=name`;
   - **unsets `AE_CORE_BIN`** (one warning to stderr when it was set) and never exports it: the
     core comes from `$AE_HOME/core/current` through `ae` itself, so nothing env-borne can go
     stale in a long-lived tmux server; the wrapper only pre-checks that `core/current` names an
     absolute executable and otherwise **refuses** (exit 2) with the exact instruction
     `just next-install`;
   - `exec`s the repo's `ae "$@"`, located relative to the wrapper's real location with the
     `install` script's symlink-safe, BSD-safe idiom (no `readlink -f`).
2. **`contrib/ae-next/install`** — idempotent, run as `just next-install`; every path below
   honours `AE_NEXT_HOME`:
   - the core: `cargo build --release --locked`, or a prebuilt binary via
     `AE_NEXT_CORE_BIN=<path>` (tests, CI artifacts); read its `--version` → `VER`; refuse
     unless `VER` equals `AE_VERSION` in `ae` (they move together);
   - install at `$AE_HOME/core/VER/ae`, directory and file `0555`, `$AE_HOME/core/` left
     `0755`: absent → copy; byte-identical → no-op; **different bytes → refuse** ("bump the
     version");
   - write `$AE_HOME/core/current` = the expanded absolute path (temp + rename);
   - seed `$AE_HOME/config` from `~/.ae/config` when absent, with the whole `[telegram]`
     section replaced by `enabled = false` — **no `token_file`, `chat_id`, `allowed_user_ids`**
     survive the copy; a re-run never touches an existing config; no `workspace.core` is
     written (the core has exactly one source: `core/current`);
   - symlink `~/.local/bin/ae-next` → the wrapper;
   - never writes `~/.ae`, `~/.local/bin/ae`, or the `install` script;
   - hardening (second cross-model review + delta review): `AE_NEXT_HOME` must be absolute,
     contain no `.` or `..` component (a non-existent intermediate defeats path resolution, so
     `~/missing/../.ae` would alias the installed home), must not resolve to `~/.ae`, and its
     final root must not be a symlink — the wrapper checks the same rules, byte-identical, so an
     install can never become a run-refusal; `AE_NEXT_TMUX_SERVER` must match the session-name
     grammar (`^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$` — no slash, no dot components, so `./default`
     cannot alias the installed socket) and may not be `default`; owned nodes are classified once with lstat (`core/` and the version dir real
     directories; the installed binary, `core/current` and the config absent or regular
     non-symlink files — `current` is never followed through a symlink); the core leaf is
     written temp + chmod + rename into an absent path, an identical writable core is hardened
     to 0555, and the `ae-next` link is published by temp symlink + rename after refusing a
     directory or symlink-to-directory destination.
3. **Two small glue changes in `ae`.**
   **(a) The `core/current` source** — `_ae_core_bin_input` gains one source between the env
   and the config: `$CONFIG_DIR/core/current`, read as one trimmed line, used only when it is an
   absolute path (a relative or `~` value is ignored with the same fail-quiet shape as an
   unusable config value). General, not next-specific: P5 can install through the same pointer.
   **(b) Telegram ownership guard** (`_telegram_spawn_daemon`; marked
   `COEXISTENCE (pre-P5): delete at the entry flip`). Active only when `AE_TMUX_SERVER` is
   set (a named-server instance). Before spawning: probe the **default** tmux server —
   selected explicitly and independently of the caller's environment:
   `env -u TMUX -u TMUX_TMPDIR tmux -L default`, so tmux resolves its fixed default socket
   (`/tmp/tmux-<uid>/default`; measured: `-L` outranks `$TMUX`, and a missing server errors
   rather than falling back) — for a live `ae-telegram` session, tri-state as in #28. **No
   environment input selects that server**: neither a knob of ours nor `TMUX_TMPDIR`, which is
   the same bypass in tmux's own variable — an empty alternate directory would read as "no old
   bridge" while the real one polls. The one remaining seam is `PATH`, the product's universal
   trust boundary for every tmux call, not a guard-specific input. Absent → spawn. Present → read this instance's token and the default home's token
   (`$HOME/.ae/config` → `telegram.token_file`, `~` expanded): **different → spawn; equal or
   either unreadable → refuse** with the exact instruction ("another ae instance's Telegram
   bridge is live on the default tmux server with the same bot token: run `ae telegram stop`,
   or give ae-next a distinct bot token"). Unknown (tmux rc 2) → refuse. The same probe also
   looks for a default-server `ae-aewatch` sidecar (it kills competing bridges on every server)
   and refuses with the instruction to stop it — never killing another instance's process.
   **Check → spawn → re-check:** the installed `ae` is frozen on `main` and can honor no new
   claim, so after its own bridge starts the guard probes once more; if the default bridge
   appeared meanwhile with the same (or an unreadable) token, it kills **its own** bridge and
   refuses. Residual, accepted: a window of seconds in which both may poll, bounded by the
   re-check. Visibility: the refusal is loud for `telegram start` and visible in `telegram
   status`/`doctor` (the bridge is simply not running). Autostart (launch, `_supervise`) now
   records the fixed redacted category and timestamp in `~/.ae/telegram/autostart-refusal`,
   emits `telegram_autostart_refused` in the current session's `events.jsonl`, and surfaces
   the last record through `status`/`doctor` even while the bridge is down. Functionally benign
   for the same-token case: the default-server bridge keeps serving.
4. **Core: `ae _net-probe <host> [--port N]`** — resolves `host:port` with
   `std::net::ToSocketAddrs`; prints `ok <n>` or `error: <class>`; exit 0/1; no TLS, no token.
   The musl DNS/NSS instrument. A core behavior change → **version `0.2.21`** (`AE_VERSION`,
   `Cargo.toml`, `Cargo.lock` together). Ratified by the scope review as the leaner honest
   proof.
5. **CI (Linux leg):** after the static-musl proof, run the musl binary's
   `_net-probe api.telegram.org` and require `ok` — the proof of record for musl DNS/NSS.
6. **Tests.**
   - `tests/integration`, section **"ae-next coexistence: installer + wrapper"** (isolated
     HOME, prebuilt stub core that prints `ae <AE_VERSION>`, a stub target `ae` that dumps env
     + argv): install creates `core/VER/ae` at `0555` with `core/` at `0755`; `core/current`
     holds the expanded absolute path; a second run is a no-op; changed bytes are refused; an
     **upgrade install** (a second version) succeeds beside the first and repoints `current`;
     the seed strips every credential key and forces `enabled = false`; a re-run leaves an
     edited config alone; the symlink exists; `~/.ae` and `~/.local/bin/ae` are never
     written; an `AE_NEXT_HOME=<other>` install writes only there. Wrapper: the env contract
     (`AE_HOME`, `CONFIG_FILE`, `AE_TMUX_SERVER`, kind, `AE_CORE_BIN`) at the defaults AND
     under both overrides; **conflict cases** — inherited `CONFIG_FILE`, inherited
     `AE_CORE_BIN`, and a cwd `.ae/config` carrying `workspace.core = /tmp/evil` all lose;
     missing `core/current` → exit 2 naming `just next-install`; argv passthrough.
   - `tests/integration`, section **"ae-next coexistence: real ae through the symlink"**:
     install with `AE_NEXT_CORE_BIN=target/debug/ae`, launch a session through
     `~/.local/bin/ae-next`, then assert: the session's tmux session exists on the `ae-next`
     server and **not** on the harness's default server; `ae-next list` shows it and the plain
     `ae` (default home, default server) does not; the session meta pins `ae_core=` the
     immutable path and `ae_core_version=` its version; a generated helper's `_lib` re-exports
     the next server; `ae-next end` archives under `$AE_HOME/archive`.
   - `tests/integration`, section **"ae-next coexistence: Telegram ownership guard"**. Test
     isolation is the `PATH` seam only: a **passthrough fake tmux** on `PATH` answers
     `-L default list-sessions` from a fixture and `exec`s the real tmux for everything else, and
     records the environment it was invoked with. `command tmux` bypasses functions, not `PATH`.
     Cases: default-server bridge live + same token → refuse, no `new-session`; live + distinct
     token → spawn; absent → spawn; tmux unknown → refuse; `AE_TMUX_SERVER` unset → guard
     skipped. **Bypass discriminators:** (1) probe from inside a pane of the named server
     (`TMUX` set to it) with the fixture live and equal credentials → refuse; (2) the caller's
     `TMUX_TMPDIR` set to an empty alternate directory, fixture live, equal credentials → refuse,
     and the fake's recorded environment shows both `TMUX` and `TMUX_TMPDIR` absent — if the
     guard ever stops clearing them, this case goes red.
   - `tests/integration`, section **"ae-next coexistence: upgrade + re-entry"**: install v1,
     launch a session (pins v1); install v2 (`core/current` repointed, v1 leaf intact); a NEW
     session pins v2 while the v1 session's helpers keep v1; kill the bridge and let
     `_supervise` restart it — the restart resolves v2 through `core/current` with no fallback
     (the same passthrough fake answers the guard's default-server probe "absent" here, so the
     case is deterministic on a machine whose real default server carries a live bridge);
     `compact` a session — the child launch binds the intended version, never empty.
   - Unit (`tests/unit`): `_ae_core_bin_input` precedence env → `core/current` → config; a
     relative or `~` `current` is ignored; a missing file falls through to config.
   - Rust: `_net-probe` unit tests (`localhost` → ok; a `.invalid` name → exit 1).
   - Both scripts join `just lint`. Mutation controls: delete the "different bytes → refuse"
     branch and the "equal token → refuse" branch one at a time; the matching test must go RED.
7. **Docs.** This file (install, use, canary, rollback); a README pointer; the VISION
   end-state line; the AGENTS.md structure entry for `contrib/ae-next`.

Complexity: M. Zero P5 artifacts.

## Install and use

```
just next-install          # build, install core/<ver>, write core/current, seed config, link ae-next
ae-next doctor             # core bound? server ae-next? telegram disabled?
ae-next --local mysession  # a session on the ae-next server, state under ~/.ae-next
ae-next list               # sees only ae-next sessions; `ae list` sees only the old ones

# a second, fully parallel instance (own state, own server, own core install):
AE_NEXT_HOME=~/.ae-next2 just next-install
AE_NEXT_HOME=~/.ae-next2 AE_NEXT_TMUX_SERVER=ae-next2 ae-next --local other
```

Upgrading after a version bump: `just next-install` again installs `core/<newver>/ae` beside
the old one and repoints `core/current`. Running sessions keep their pinned old path — still
present, still immutable — so nothing degrades underneath them.

## Canary (after the slice ships; human-run, dogfooded)

All four outcomes, then no open blocker:

- [x] **Lifecycle + archive** — PASS 2026-08-30 (second run; core 0.2.23). Session on the `ae-next`
      server only (the default server never listed it); meta pinned the immutable core; `memo`,
      `send`, `peek` and `events.jsonl` worked from outside the pane; `ae-next end` archived
      (`040c64e3…`) and left no live state or server. Two by-design refusals seen on the way:
      `state` from outside a pane refuses (state is pane-attributed), and a Codex worker in a
      brand-new directory shows its directory-trust chooser — operator consent, never answered
      by ae. The FIRST run found a real bug: the final attach `exec`'d tmux past the server
      shim and searched the default server ("cannot find session") — fixed in `1039fd9d`.
- [x] **Compact + restore** — PASS 2026-08-30. `ae-next compact` published the archive and the
      same-name child came up with the frozen roster; `ae-next <new> --from <uuid>` recorded
      `parent_archive_id` and the pinned core. The child has no `memo.tsv` — by contract (no
      archive *content* is injected; the parent pointer and the digest instruction are the
      handover, see `docs/reference/commands.md`). Headless invocations (no tty) end with
      `open terminal failed: not a terminal`, rc 1, AFTER the operation succeeded — the final
      attach has no terminal; a scripted canary should use the no-attach path.
- [ ] **Telegram round-trip.** The seeded config has no credentials; configure `ae-next`
      explicitly, one of:
      **A (recommended)** a distinct test bot: its token file under `~/.ae-next`, its
      `allowed_user_ids`, `[telegram] enabled = true`; the live bridge keeps running — the
      guard allows it because the tokens differ.
      **B** exclusive smoke with the live token: `ae telegram stop` → `ae-next telegram start`
      → `say` + a reply → `ae-next telegram stop` → `ae telegram start`. If the old bridge is
      still live, the guard refuses the start — that refusal is the test of the guard.
- [x] **musl DNS/NSS:** the CI Linux leg's `_net-probe` step is green; optionally the same
      binary in `docker run --rm alpine` locally. *PASS 2026-08-30, run 33323387544 at
      `dd38896d`, `ubuntu-24.04`: the static-pie musl `ae 0.2.23` answered
      `_net-probe api.telegram.org` with `ok 2` (two addresses), after the reserved
      `.invalid` negative control exited 1 as the step requires. The lane had been red on
      both platforms since the first dependency landed; three causes were peeled before the
      probe could run: a history-deriving control on a depth-1 checkout (`d42a6db2`), the
      same control inside cargo-mutants' `.git`-less tree copy (`copy_vcs`, `3e44ad32`), and
      a Linux liveness-classification bug — tmux 3.4 escapes the U+001F separator in `-F`
      output, so a present pane read as "hard dead" (`dd38896d`). Scope of the proof: the
      runner's resolver (systemd-resolved stub, `/etc/resolv.conf`) works from a static musl
      binary; a host whose names resolve only through NSS modules (LDAP/SSSD, mDNS) is still
      outside it — the AGENTS.md caveat stands as a host-specific residual, not a blocker.*

## Rollback

Manual, human-run, total: `rm ~/.local/bin/ae-next`; `tmux -L ae-next kill-server`;
`rm -rf ~/.ae-next`. Nothing under `~/.ae` or at `~/.local/bin/ae` was ever changed.
Per-session, `AE_CORE=` (set-empty) forces the bash bodies for one invocation.

## Known edges (accepted)

- `ae-next stop` run from inside a pane of the **old** server consults `$TMUX` for the caller's
  pane tty; the answer is simply not a pane of any `ae-next` session. Harmless.
- The wrapper overrides an inherited `AE_HOME`, `CONFIG_FILE` and `AE_TMUX_SERVER` and
  unsets `AE_CORE_BIN` unconditionally — that is its purpose. `AE_NEXT_HOME`/`AE_NEXT_TMUX_SERVER`
  are the sanctioned overrides; the core has no override at all (install a version instead).
- The ownership guard knows the default home as `$HOME/.ae` literally. An old instance run
  under a relocated `AE_HOME` is not detected; the guard fails closed only on what it can see.
- A cwd `.ae/config` still overrides *other* keys (agents, layout) for `ae-next` exactly as it
  does for `ae` — only the core is coexistence-owned.
