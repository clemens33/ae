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
| **State** | each agent declares `working` / `waiting-user` / `waiting-agent` / `blocked` / `done`; the session surfaces who needs a human |
| **Memory** | `memo`, the event log, and the archive — shared, durable, restart-surviving |
| **Lifecycle** | start, stop, resume, transfer, end, archive, reboot; lineage is explicit (`--from <uuid>`), never inferred |
| **Human surface** | the status bar, `ae list` / `ae next` / `ae brief`, the `ae chat` lane and the `ae app` terminal UI, and Telegram out and back in — the human is a participant, not a spectator |

Panes, splits, scrollback and attach are tmux's job and stay there. What ae is *not* is
listed in AGENTS.md and has not changed.

## What does not change

- **Simplicity is the feature.** The whole tool must stay understandable in one sitting.
- **Daily productivity over feature completeness.** If it does not save time on every use,
  it does not belong. Resisting features is the work.
- **One command to install after prerequisites.** A checksum-verified three-member bundle
  needs no Rust runtime, run-time package manager, or service to keep alive. tmux (3.4 or
  newer) and git remain required and are not bundled.
- **Your repo stays clean.** Session state lives in `~/.ae/sessions/`, archives in
  `~/.ae/archive/`. Working directories are never touched.
- **Optional stays optional.** Companions (telegram, orchestrator, monitor) may declare their
  own dependencies; core ae keeps working on a machine without them.
- **Coordination is a protocol, not a framework.** Agents call small CLI helpers they
  already know how to use. No SDK, no plugin system, no custom wire format.

## The shape today

**One typed core, no wrapper.** The public `ae` command is a symlink straight into a
versioned, read-only install directory — `~/.local/bin/ae` → `~/.ae/versions/<V>/ae-core`.
There is no pair to validate any more: the core decides INSTALLED vs CHECKOUT from where
its own binary resolves to, not from a second file agreeing with it. The core owns state,
lifecycle, the daemons and every command; a session's helpers are symlinks to it and a
pane's command is a core entry, so nothing generated is a script any more. Rust calls the
`tmux` CLI directly — bash was always the glue, never the API, and the one bash file left
in the product is the installer itself.

**Why the core is Rust.** Agents author most of this code, and a restrictive compiler is a
free review lane that never gets tired: a `Result` must be consumed, so the silent-abort
class dies; newtypes encode provenance, so fresh-vs-restored is a type rather than a
convention; and the empty-key / unset-subscript abort class becomes unrepresentable. Go
gives the same static binary, but with agent authors its simplicity edge evaporates and the
type system's bug-class elimination wins. Python stays wrong for the core: interpreter boot
on every helper call, against a hot path where helpers are called constantly. The reasoning
that decided it is recorded under "Revisit triggers" in docs/history.md.

**Daemons are core-owned.** The watchdog and the Telegram bridge run inside the core, and
their panes run it directly through a link named for the job. There is no Python in the
product and no sidecar process: `contrib/` holds configuration and charter templates only.

**Install preserves the user-level contract.** After tmux (3.4 or newer) and git are present,
one command installs a checksum-verified three-member bundle (`ae-core`, `install`,
`SHA256SUMS`), publishes it read-only under `~/.ae/versions/<V>/`, and points
`~/.local/bin/ae` straight at that version's `ae-core` — the public command is the
bundle's core member, not a separate file. It needs no Rust runtime, run-time package
manager, or service; macOS is built natively on `macos-15`, while Linux amd64 release
bundles are static musl. Building from source instead compiles a native binary for the
current machine.

## Where it is going

- **The Bash remainder stays minimal.** It is policy-frozen — no new Bash features — and a
  pane-side block retires as soon as the core can own it outright. What still binds the
  installer and the ad-hoc shell is the GNU/BSD table in AGENTS.md plus the export-expansion
  rule beside it; the full hazards checklist is docs/history.md §14.
- **Upgrading carries running sessions across — shipped.** `ae upgrade` installs a release,
  then migrates, repoints and relinks every placeable session before it moves
  `~/.local/bin/ae` to the new `ae-core`: helper links re-render, and the watchdog and the
  Telegram bridge restart on the new core. A stopped session it cannot place is reported and
  skipped, unreferenced versions are pruned, and a running `ae app` follows the link itself
  once idle. Agent panes are never restarted — they run the agent tool, not ae — and nothing
  is ended or archived.
- **Ownership grain is the mutation domain, never the file.** One logical operation — one
  command, one transaction — is owned wholly by one side. Per-file single-writer is not
  enough: it still permits a split transaction across events, meta, requests, claims and
  tmux with divergent lock order.
- **Every capability earns its place.** The seven above are the product. A new one has to
  displace something, or save time on every single use.
