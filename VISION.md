# Vision

What ae is, and where it is going. The binding rules live in [AGENTS.md](AGENTS.md) and the
user docs in [docs/](docs/index.md). This file is the *why* those two hang together.

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
- **One command to install after prerequisites.** A checksum-verified three-member bundle
  needs no Rust runtime, run-time package manager, or service to keep alive. Bash >= 4,
  tmux, and git remain required and are not bundled.
- **Your repo stays clean.** Session state lives in `~/.ae/sessions/`, archives in
  `~/.ae/archive/`. Working directories are never touched.
- **Optional stays optional.** Companions (telegram, orchestrator, monitor) may declare their
  own dependencies; core ae keeps working on a machine without them.
- **Coordination is a protocol, not a framework.** Agents call small CLI helpers they
  already know how to use. No SDK, no plugin system, no custom wire format.

## The shape today

**One typed core, a thin Bash remainder.** The public `ae` command is a small wrapper that
validates a matched, immutable pair — wrapper and Rust core — out of one versioned
directory, then execs the core once with the facts it cannot see for itself. The core owns
state, lifecycle, the daemons and every command. Bash keeps only what it is best at: that
wrapper and the pane-side artifacts the core writes (`launch.<slot>.sh`, interactive shims).
Rust calls the `tmux` CLI directly — bash was always the glue, never the API.

**Why the core is Rust.** Agents author most of this code, and a restrictive compiler is a
free review lane that never gets tired: a `Result` must be consumed, so the silent-abort
class dies; newtypes encode provenance, so fresh-vs-restored is a type rather than a
convention; and the empty-key / unset-subscript abort class becomes unrepresentable. Go
gives the same static binary, but with agent authors its simplicity edge evaporates and the
type system's bug-class elimination wins. Python stays wrong for the core: interpreter boot
on every helper call, against a hot path where helpers are called constantly. The reasoning
that decided it is recorded under "Revisit triggers" in AGENTS.md.

**Daemons are core-owned.** The watchdog and the Telegram bridge run inside the core, and
their panes are four-line shims that exec it. Python contrib is reference and incubator only, and
analytics sidecars stay Python contrib indefinitely.

**Install preserves the user-level contract.** After Bash >= 4, tmux, and git are present,
one command installs a checksum-verified three-member bundle (`ae`, `ae-core`,
`install`). It needs no Rust runtime, run-time package manager, or service; macOS is built
natively on `macos-15`, while Linux amd64 release bundles are static musl. Building from source
instead compiles a native binary for the current machine.

## Where it is going

- **The Bash remainder stays minimal.** It is policy-frozen — no new Bash features — and a
  pane-side block retires as soon as the core can own it outright. The bash-hazards checklist
  in AGENTS.md governs what is left.
- **Upgrading carries running sessions across.** `ae upgrade` already installs a release and
  repoints the wrapper, but a running session is only reported and left pinned until someone
  stops and resumes it by hand. The next step is doing that part too: inventory what is
  running, switch, and bring exactly that set back — without ending or archiving anything.
- **Ownership grain is the mutation domain, never the file.** One logical operation — one
  command, one transaction — is owned wholly by one side. Per-file single-writer is not
  enough: it still permits a split transaction across events, meta, requests, claims and
  tmux with divergent lock order.
- **Every capability earns its place.** The seven above are the product. A new one has to
  displace something, or save time on every single use.
