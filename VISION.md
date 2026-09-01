# Vision

What ae is, and where it is going. The binding rules live in [AGENTS.md](AGENTS.md), the
user docs in [docs/](docs/index.md), the migration plan in
[epic #79](https://github.com/clemens33/ae/issues/79). This file is the *why* those three
hang together.

## What ae is

A thin coordination layer over tmux for multi-agent work. Several agent CLIs run side by
side; ae makes them a fleet instead of a pile of tabs. Seven capabilities, and deliberately
nothing else:

| | |
|---|---|
| **Identity** | every agent is told which agent it is (`alias:name`, slot) and who else is in the workspace |
| **Messaging** | agents reach each other by name — `send`, `interrupt`, cross-session `@session:agent` |
| **Requests** | `ask` / `review` carry a request id and an exact reply command, so a question has a findable answer |
| **State** | each agent declares `working` / `waiting-user` / `blocked` / `done`; the session surfaces who needs a human |
| **Memory** | `memo`, the event log, and the archive — shared, durable, restart-surviving |
| **Lifecycle** | start, stop, resume, transfer, end, archive, compact; lineage is explicit (`--from <uuid>`), never inferred |
| **Human bridge** | Telegram out and back in — the human is a participant, not a spectator |

Panes, splits, scrollback and attach are tmux's job and stay there. What ae is *not* is
listed in AGENTS.md and has not changed.

## What does not change

- **Simplicity is the feature.** The whole tool must stay understandable in one sitting.
- **Daily productivity over feature completeness.** If it does not save time on every use,
  it does not belong. Resisting features is the work.
- **One command to install after prerequisites.** A checksum-verified four-member bundle
  needs no Rust runtime, run-time package manager, or service to keep alive. Bash >= 4,
  tmux, and git remain required and are not bundled.
- **Your repo stays clean.** Session state lives in `~/.ae/sessions/`, archives in
  `~/.ae/archive/`. Working directories are never touched.
- **Optional stays optional.** Companions (telegram, orchestrator, monitor) may declare their
  own dependencies; core ae keeps working on a machine without them.
- **Coordination is a protocol, not a framework.** Agents call small CLI helpers they
  already know how to use. No SDK, no plugin system, no custom wire format.

## Where it is going

**One typed core, a thin bash remainder.** `main` now carries the Rust binary, package, and
public command named `ae`. Bash keeps only what it is best at: pane-side artifacts
(`launch.<slot>.sh`, interactive shims) and the tmux mechanics around new-session/attach.
Rust calls the `tmux` CLI directly — bash was always the glue, never the API.

**Why now.** The revisit triggers in AGENTS.md fired, in order:

- *Trigger 1 — the bash tax recurred.* `set -e` silent aborts, IFS/TSV framing, empty-array
  subscripts kept shipping **after** the hazards checklist existed.
- *Trigger 2 — state outgrew bash.* The event ledger, request tracking, claims and
  freshness fingerprints all want typed, concurrent-safe state with real transactions.
- *Trigger 3 — the daemon half outgrew the wrapper half.* Watchdog and telegram are
  long-lived processes wearing a shell script.

**Why Rust specifically.** Agents author most of this code, and a restrictive compiler is a
free review lane that never gets tired: a `Result` must be consumed (the silent-abort class
dies), newtypes encode provenance (fresh-vs-restored becomes a type, not a convention),
serde removes hand-rolled framing, and bash's empty-key / unset-subscript abort class
becomes unrepresentable.
Go gives the same static binary, but with agent authors its simplicity edge evaporates and
the type system's bug-class elimination wins. Python stays wrong for the core: interpreter
boot on every helper call, against a hot path where helpers are called constantly.

**Daemons are Rust-owned.** Watchdog and Telegram run in the core; Bash keeps start/stop/tick
pane glue, while the old watchdog loop is only a rollback path. Python contrib is reference
and incubator only. Analytics sidecars stay Python contrib indefinitely.

**Install preserves the user-level contract.** After Bash >= 4, tmux, and git are present,
one command installs a checksum-verified four-member bundle (`ae`, `ae-core`, `ae-glue`,
`install`). It needs no Rust runtime, run-time package manager, or service; macOS is built
natively on `macos-15`, while Linux amd64 release bundles are static musl. A checkout install
instead compiles a native binary for its current machine.

## How we get there

Strangler fig, with single-owner vertical cutovers.

- **Ownership grain is the mutation domain, never the file.** One logical operation — one
  command, one transaction — is owned wholly by bash or wholly by Rust. Per-file
  single-writer is not enough: it still permits a split transaction across
  events/meta/requests/claims/tmux with divergent lock order.
- **Bash is evidence, not the oracle.** A semantic contract — drafted now, in force only
  once both seats record its ratification — will classify today's behavior into intended /
  known-defect / deliberately-changed. Differential parity without it would freeze
  accidental bash behavior into the typed design.
- **Every cutover is reversible until its phase gate passes.** The bash implementation of a
  domain is deleted only after the Rust owner survives a full gate; the flip commit names
  its revert. **P5 status exception:** the owner explicitly authorized entry despite
  unclosed formal P1/P4 parity evidence. Runtime and product gates passed; formal parity
  closure is not claimed, and its gate record remains local WIP.

| Phase | What flips |
|---|---|
| **P0** | Semantic contract, golden corpus, state/lock model; Cargo package, lanes, CI, native-macOS and static-musl-Linux release lanes |
| **P1** | Read side: `list --json`, requests, events queries; snapshot parity against the corpus |
| **P2** | Write domains: events emission, request tracking, state/goal/memo — helpers become thin shims |
| **P3** | Lifecycle: claims, archives, compact orchestration; bash keeps tmux new-session/attach |
| **P4** | Daemons: watchdog + telegram graduate from Python contrib |
| **P5** | **Entered 2026-08-31.** The public wrapper validates the immutable Rust core and policy-frozen, shrinking Bash pane glue, including the P5 sibling-binding routing fix; the single-file / pure-Bash doctrine is retired and the bash-hazards checklist now governs only surviving pane glue. |

The P5 entry flip makes the `ae-next` coexistence surface unreachable via the public entry;
the residual glue block is scheduled for its own retirement slice. The public `ae` wrapper
validates its matched wrapper/core/glue triple before execution; coreless Bash mode is
unreachable through that entry. The historical coexistence decision remains in
[docs/migration/coexistence.md](docs/migration/coexistence.md); the byte-exact local
`ae-legacy` P5 cutover anchor stays outside the public bundle until pre-flip live sessions
stop or resume.

## Status

P5 entered on 2026-08-31; `main` now carries the post-strangler product, while
`rust-rewrite` is historical development-branch context. The tracked Bash `ae` is
policy-frozen (no new Bash features), shrinking surviving pane glue with the P5
sibling-binding routing fix, shipped as immutable `ae-glue`. The original pre-rewrite script
is preserved locally byte-exact at `72c7293` as the `ae-legacy` P5 cutover anchor outside the
public bundle, removable once pre-flip live sessions stop or resume. The single-file /
pure-Bash rules are historical. Bash hazards remain binding for the surviving pane-side glue.
