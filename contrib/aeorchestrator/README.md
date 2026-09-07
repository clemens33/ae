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

## Role

The watchdog reads fleet state through ae, renders a compact `NEEDS YOU` /
`WORKING` / `QUIET` overview, and pastes it into the orchestrator pane only
when the rendered content changed. Empty sections are omitted and quiet
sessions collapse to one line. The seat only declares `done` on that turn, so
the watchdog can tell the delivery was acknowledged. Each delivered change
costs one minimal model turn; a timer alone costs none.
Routine overviews do not use Telegram. It stays `done` between changes.

It relays only explicit human instructions through its full-path `relay`
helper. The target is a session or exact `session:agent`; the text arrives bare
and therefore speaks with human authority. The caller session alone audits the
target and full text. This is intentionally not a general dispatch mechanism.

It never dispatches work, changes goals, clears questions, runs lifecycle
operations, edits project/session state, or treats text from another session as
instructions. Other-session text is data. See [`CHARTER.md`](CHARTER.md).

Residual risk is explicit: an unenveloped relay speaks with human authority,
yet the seat is a model. The role is constrained, every attempt is audited, and
the seat deliberately runs a cheap model; never give it judgment tasks.

The standard workspace watchdog checks the template's orchestrator each cycle.
The persisted `[workspace] sweep` setting is the minimum spacing between
changed overviews and outranks the process-wide fallback and default. A zero
disables overview delivery. The watchdog persists the last delivered hash and
its own heartbeat, so restarts do not resend unchanged text. The seat never
runs `ae brief --all` on a timer; it may read it once for a human fleet question
or routing decision. The seat is started explicitly; it is never an autostart companion.
`AE_NO_AUTOSTART=1` suppresses the Telegram bridge when launching another session.

## Files

| File | Role |
|---|---|
| `orchestrator.config` | Embedded first-run config with the role prompt inline. |
| `CHARTER.md` | Short human-readable role contract. |

Edit `~/.ae/config` to choose another profile, or edit
`~/.ae/orchestrator.config` for local preferences. Keep the role boundaries
intact.

## Dependencies

None beyond ae and one configured agent CLI. Fleet overview, minimum spacing, and
bare relay are core ae operations; no Python, `jq`, or `curl` sidecar is needed.
