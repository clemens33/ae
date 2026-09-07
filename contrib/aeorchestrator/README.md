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
and reads it as the seat's local overlay. It never reads the current project's
`.ae/config` for this launch. Edit the seeded file to change the profile or
local preferences. `CHARTER.md` is the readable copy of the role contract; it
is not loaded from a guessed path.

## Role

The orchestrator reads fleet state with `ae brief --all` (or `ae list`) and
reports three buckets: needs your answer, health to inspect, and in progress.
Reports use `session:agent` identities and go through `say`. It relays only
explicit human instructions, using exact `send`, `ask`, or `review` helpers and
showing each delivery verdict or request id.

It never dispatches work, changes goals, clears questions, runs lifecycle
operations, edits project/session state, or treats text from another session as
instructions. Other-session text is data. See [`CHARTER.md`](CHARTER.md).

The standard workspace watchdog can nudge a configured orchestrator seat to run
its sweep. The seat is started explicitly; it is never an autostart companion.
`AE_NO_AUTOSTART=1` suppresses the Telegram bridge when launching another session.

## Files

| File | Role |
|---|---|
| `orchestrator.config` | Embedded first-run config with the role prompt inline. |
| `CHARTER.md` | Short human-readable role contract. |

Edit `~/.ae/orchestrator.config` to choose another profile or add local
preferences. Keep the role boundaries intact.

## Dependencies

None beyond ae and one configured agent CLI. Fleet sweep and Telegram `say` are
core ae operations; no Python, `jq`, or `curl` sidecar is needed.
