# aeorchestrator — role contract and config template

Optional templates for the `orchestrator` seat. Core ae does not require these
files. The bare command starts an ordinary local session named `orchestrator`;
the config below gives that seat its roster and role instructions.

## Install

From the project directory where the seat should run:

```bash
mkdir -p .ae
cp contrib/aeorchestrator/orchestrator.config .ae/config
ae orchestrator
```

`ae orchestrator` starts or reattaches the local `orchestrator` session. If no
local config exists, ae still starts the seat and prints this copy hint. The
config is read as the normal project-local overlay, so it must contain its own
`[profiles]`, `[roster]`, `[workspace]`, and `[prompt]` entries. `CHARTER.md` is
the readable copy of the role contract; it is not loaded from a guessed path.

To run the seat without the template, use `ae orchestrator --local`; it then
uses the current project config like any other local launch.

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
| `orchestrator.config` | Copy-ready local config with the role prompt inline. |
| `CHARTER.md` | Short human-readable role contract. |

Edit the copied config to choose another profile or add local preferences. Keep
the role boundaries intact.

## Dependencies

None beyond ae and one configured agent CLI. Fleet sweep and Telegram `say` are
core ae operations; no Python, `jq`, or `curl` sidecar is needed.
