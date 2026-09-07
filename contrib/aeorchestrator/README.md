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

The orchestrator reads fleet state with `ae brief --all` and prints a compact
`NEEDS YOU` / `WORKING` / `QUIET` overview in its own pane. Empty sections are
omitted and quiet sessions collapse to one line. Routine sweeps do not use
Telegram. It stays `done` between sweeps.

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

The standard workspace watchdog nudges the template's orchestrator every 120
seconds. Each overview first runs `ae _monitor sweep` for its own session with
`--no-notify`, refreshing the completion heartbeat without sending changed
lines through `say`. The persisted `[workspace] sweep` setting outranks the
process-wide fallback and default. The seat is started explicitly; it is never an autostart companion.
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

None beyond ae and one configured agent CLI. Fleet overview, sweep cadence, and
bare relay are core ae operations; no Python, `jq`, or `curl` sidecar is needed.
