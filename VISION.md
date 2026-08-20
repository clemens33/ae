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
- **One file to install, no runtime to bring.** `curl | bash`, one artifact, no package
  manager, no interpreter, no service to keep alive.
- **Your repo stays clean.** Session state lives in `~/.ae/sessions/`, archives in
  `~/.ae/archive/`. Working directories are never touched.
- **Optional stays optional.** Companions (telegram, steward, monitor) may declare their
  own dependencies; core ae keeps working on a machine without them.
- **Coordination is a protocol, not a framework.** Agents call small CLI helpers they
  already know how to use. No SDK, no plugin system, no custom wire format.

## Where it is going

**One typed core, a thin bash remainder.** ae is being rewritten as a single Rust binary,
package and binary named `ae` from day one. Bash keeps only what it is best at: pane-side
artifacts (`launch.<slot>.sh`, interactive shims) and the tmux mechanics around
new-session/attach. Rust calls the `tmux` CLI directly — bash was always the glue, never
the API.

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

**Daemons fold in.** Watchdog and telegram graduate into the binary once their Python
behavior is stable; the Python contrib becomes reference and incubator only. Analytics
sidecars stay Python contrib indefinitely.

**Install does not change shape.** Cross-compiled darwin-arm64 and linux-amd64 (static
musl), one file, same contract as today.

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
  its revert.

| Phase | What flips |
|---|---|
| **P0** | Semantic contract, golden corpus, state/lock model; Cargo package, lanes, CI, cross-compile |
| **P1** | Read side: `list --json`, requests, events queries; snapshot parity against the corpus |
| **P2** | Write domains: events emission, request tracking, state/goal/memo — helpers become thin shims |
| **P3** | Lifecycle: claims, archives, compact orchestration; bash keeps tmux new-session/attach |
| **P4** | Daemons: watchdog + telegram graduate from Python contrib |
| **P5** | Entry flip: the installer points `ae` at the binary; the single-file / pure-bash doctrine retires and the bash-hazards checklist shrinks to the pane glue that survives |

## Status

Branch `rust-rewrite`. Bash ae is **frozen at `72c7293`** — no feature work, no rescue
tooling for pre-freeze sessions. Until a domain flips, the bash behavior documented in
README and `docs/` is the behavior you get. The single-file / pure-bash rules in AGENTS.md
govern that frozen glue until P5 and retire with it. The bash hazards do not retire — they
shrink to whatever pane-side bash survives the flip, and stay binding for it.
