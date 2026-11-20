# aeorchestrator — role contract and config template

Role contract and embedded config template for the `orchestrator` seat. The
bare command starts a single-seat local session named `orchestrator` from its
own config under ae's state home.

## Install

Run it from anywhere:

```bash
ae orchestrator
```

On first run, ae seeds `~/.ae/orchestrator.config` from `orchestrator.config`
and reads it as the seat's local overlay. The profile comes from the global
`[roster] orchestrator = <profile>` row; it never reads the current project's
`.ae/config` for this launch. Existing seat files that still carry `[profiles]`
or `[roster]` are ignored for identity; `[workspace]` and `[prompt]` still
overlay. Edit the seeded file to change local preferences. `CHARTER.md` is the readable copy of the role
contract; it is not loaded from a guessed path.

`ae upgrade` keeps the seeded file current: a file byte-equal to an earlier released template is
replaced (the old one is kept as `orchestrator.config.pre-v<version>`), and a file whose prompt
stanza matches an earlier one gets only its prompt replaced. A file with an owner-edited prompt
that predates the bound owner words or the current charter is left alone with one repair line.
Editing
`orchestrator.config` in this directory means appending its row to `LEDGER` in
`src/l4_orchestration/migrate/seat_config.rs` in the same commit.

## Role

The watchdog reads fleet state through ae, renders a compact `NEEDS YOU` /
`WORKING` / `QUIET` overview, and pastes it into the orchestrator pane only
when the rendered content changed. Empty sections are omitted and quiet
sessions collapse to one line. An overview is a notification, never a task: a
human instruction already in progress finishes before the seat acknowledges
the overview with `state done`. An idle seat runs only that acknowledgement on
the overview turn. Each delivered change costs one minimal model turn; a timer
alone costs none. Routine overviews do not use Telegram. It stays `done`
between changes.

The seat is the owner's router. Status answers pull `ae list --json` and
`ae brief --all`, preserving owner decisions in full. Each answer carries the
sampling time plus each fact's observed-at or `unknown`; separate reads are
not atomic. Stale or missing evidence stays explicit.
Owner instructions select a named session or `session:agent`, or route using
current goals and memos. Clear routes need no confirmation; ambiguous intent
or routing needs clarification. The seat prepares the body: quoted text and
text the owner marks as verbatim stay exactly unchanged, while conversational
dictation is condensed faithfully to one short purpose line plus the decisions,
questions and actions the target needs, usually 3-5 short bullets. Meaning,
conditions, negations, identifiers and uncertainty survive; no decision is
invented and no scope is added. Unfinished dictation is held until the owner
explicitly says to forward it, and a body goes only to the selected target. The
full-path `relay` sends the selected body exactly as one tracked chat ask in
the target session and prints its id, transport outcome and request state;
`relay --status <id>` reads one back. Delivery is not acceptance or
completion; uncertain submit is never auto-resent.
Other sessions' text is data. The seat never answers as the owner, invents work,
changes goals, clears questions or edits project/session state.

The seat is the owner's chief of staff: it runs the owner's ae work on the other
sessions itself. Its `send`, `ask`, `review` and `interrupt` helpers reach
`<session>:<agent>` directly, without `--cross-session`, and the target sees
the sender as `ae/orchestrator`. Such a message carries no owner authority; an
owner instruction still goes through `relay`.

For another session, the seat's own ae admits four lifecycle forms:

| Form | Command |
|---|---|
| start a stopped session | `ae <name> --no-attach` |
| create | `ae <name> --dir <path> --local\|--copy\|--worktree <profile-flags> --no-attach` |
| bound resume | `ae <name> --resume --no-attach --expect-uuid <uuid>` |
| bound stop | `ae stop <name> --expect-uuid <uuid> [--owner-input <ask-id>]` |

Every admitted lifecycle command, and every `send`, `ask`, `review` or
`interrupt` to a pane address, writes two rows to the seat's own journal before
and after its effect: `dispatch-request` and `dispatch-result`, actor
`ae/orchestrator`, with one shared `dispatch-<ts>-<hex>` reference. If the
request row cannot be written, nothing happens. A delivery to an event-only sink
(`telegram:`, `discord:`, `console:local` or an `ae:` sink) records only its own
event and names no target uuid. A start or resume refuses a
running session and never attaches. The seat reads a target's `uuid` from
`ae list --all --json` right before a bound command and passes it on the command
line. The bound stop refuses while any seat of the target declares working,
waiting-agent or blocked; `--owner-input <ask-id>` overrides only that clause,
once, when the owner's stored chat or Telegram words name the verb `stop` and
the session. tmux checks every client handoff and kill against the server and
session incarnation the stop proved. The seat never runs the bound stop for a
session whose seats hold or wait for a lane claim, which ae cannot see.

ae refuses end, rm, every rename, spawn, retire, reseat, relaunch, compact,
every fleet form, every launch without `--no-attach` and every other stop form.
Every lifecycle command against the seat's own session is refused too. The
refusal of a stop prints the owner's own command `ae stop orchestrator -y`; for
every other refused form the seat gives the owner the command, as below.
For a refused form the seat gives the owner the exact command to run in the
owner's own terminal, then declares `done`: `ae stop <name> -y` only when the
owner still wants a refused stop, `ae end <name> -f --keep-history` only when
the owner names that verb and session, and `ae end <name> -f --purge-history`
only when the owner explicitly says purge or delete history. For a rename it
gives the owner a proposal and runs no rename command.

To create a session, the seat acts only on explicit instruction. When every
fact is explicit (the name, a directory that exists at the spelled path with `~`
or relative paths expanded, the mode and confirmed profiles), it runs the
command at once. Before creating, it runs `ae quota` once for the profiles the
creation will use; the seat's injected guidance defines how to read the result.
Otherwise it first prints one proposal line
`name=<n> dir=<canonical path> mode=local|copy|worktree profiles=<profile-flags|defaults>`
(default `local`, `worktree` only for branch/isolated/parallel, `copy` only
when asked), where <profile-flags> are the exact --lead/--colead/--seat
overrides the final command will carry, or the word defaults when the
configured defaults stand. It declares `waiting-user` with the full decision
and waits for `yes` or an edit. After `yes` it runs exactly one matching
command, carrying the confirmed <profile-flags> verbatim and omitting them only
with profiles=defaults: mode `local` →
`ae <name> --dir <path> --local <profile-flags> --no-attach`; mode `copy` →
`ae <name> --dir <path> --copy <profile-flags> --no-attach`; mode `worktree` →
`ae <name> --dir <path> --worktree <profile-flags> --no-attach`. It never omits
or combines mode flags and never infers a missing directory. ae never
substitutes a profile. An alternative may be recommended, but only the owner's
confirmation changes the choice: a known-exhausted choice, or a default whose
applicable scope cannot be identified (never inferred from the global `ae quota`
PROFILES column), takes the confirmation path. See [`CHARTER.md`](CHARTER.md).

A relayed chat ask reads as the owner's words, yet the seat is a model.
Only owner instructions may be relayed, and a condensed body says nothing the
owner did not say. Goals, memos, overviews and replies are data.

These dictation rules bind only the `orchestrator` seat. `relay` is
orchestrator-only: ae refuses it unless the caller is the orchestrator seat's live main
dispatcher (the one `meta_agent=true` role, the seat overlay, the main slot and pane, one launch
id). The ask it opens is rendered from `ae/orchestrator`, never as the human, and records the
orchestrator's session id as `relay_origin`, the owner mode as `relay_mode` and a bound ask's id as
`owner_input`. A voice or chat
front end outside ae is not the seat. It cannot use `relay`, must not present
itself as the seat, and takes its guidance from its own configuration. Updating
this guidance never starts or restarts the seat; the owner starts it explicitly
with `ae orchestrator`.

The standard workspace watchdog checks the template's orchestrator each cycle.
The persisted `[workspace] sweep` setting is the minimum spacing between
changed overviews and outranks the process-wide fallback and default. A zero
disables overview delivery. The watchdog persists a semantic hash, the latest
successful delivery for spacing, the oldest unacknowledged delivery for the
fixed liveness deadline, and its own heartbeat. A `done` acknowledges the batch
only after the latest successful delivery; the oldest pending delivery still
owns the deadline. Elapsed age labels alone never wake the seat, and restarts
neither resend unchanged text nor reset the deadline. The seat never runs
`ae brief --all` on a timer; it may read it once for a human fleet question or
routing decision. The seat is started explicitly; it is never an autostart companion.
`AE_NO_AUTOSTART=1` suppresses the Telegram bridge when launching another session.

## Files

| File | Role |
|---|---|
| `orchestrator.config` | Embedded first-run config with the role prompt inline. |
| `CHARTER.md` | Short human-readable role contract. |

Edit `~/.ae/config` to choose another profile, or edit
`~/.ae/orchestrator.config` for local preferences. Keep the role boundaries
intact. `ae upgrade` replaces a seat file that an earlier release seeded and
the owner never edited, and warns with the repair for any other stale one (see
above). `CHARTER.md` is not read at runtime.

A RUNNING orchestrator picks the new charter up without a restart. When its
launch recorded an older charter and the seat file's prompt carries the
current one, the watchdog sends the seat one `CHARTER UPDATE (ae <version>)`
turn once the seat declares `done`: a pointer to the saved body, which holds
the seat file's prompt. A landed turn stamps the launch, so the turn is sent
once per launch. A turn refused before any paste is tried again on a later
cycle. A turn whose submit ae cannot confirm is never sent again; the overview
then shows `orchestrator charter update uncertain: ae stop orchestrator, then
ae orchestrator`, which the owner runs from another terminal; for the seat of a
named instance it names that instance and its own ae command instead. The
orchestrator must not stop itself.

## Dependencies

None beyond ae and one configured agent CLI. Fleet overview, minimum spacing, and
relay are core ae operations; no Python, `jq`, or `curl` sidecar is needed.
