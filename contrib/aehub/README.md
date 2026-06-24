# aehub — meta-agent (hub) config + charter templates

**Optional. NOT core ae.** These are the templates `ae hub --init` scaffolds into
`~/.ae/meta-hub/` so `ae hub` can launch the **meta-agent hub** — a single ae
session that monitors all your other ae sessions and is your single point of
contact to them (relay + report, via Telegram `say`).

Core ae (`ae list`, `ae <name>`, …) does **not** depend on any of this. You only
touch it if you run `ae hub`.

## What `ae hub` does

```bash
ae hub --init     # scaffold ~/.ae/meta-hub/{hub.config,CHARTER.md} from these templates
ae hub            # start (or resume) the `hub` session with that standalone config
```

`ae hub` launches the `hub` session with **full config isolation**: it uses
`~/.ae/meta-hub/hub.config` as the config and neutralizes any project-local
`./.ae/config`, so the global config's `workers` never leak into the single-agent
hub regardless of which directory you run it from.

Escape hatch: `ae hub` is a reserved subcommand. If you ever need a normal
(non-meta) session literally named `hub`, `ae --local hub` reaches the generic
start path (the first arg is no longer `hub`).

## The templates

| File | Role |
|---|---|
| `hub.config` | Standalone single-agent config (`hub = true` marks the meta-agent; the loop then runs a sweep cadence instead of the stale-nudge watchdog). |
| `CHARTER.md` | The meta-agent's operating manual — its two jobs, the `say` channel, the `aemonitor` sweep routine, the ae toolbox, and hard guardrails (injection boundary, never-end-a-session, human-directed relay only). |

### Placeholders substituted on `--init`

- `__CHARTER_PATH__` (in `hub.config`) → the absolute path of the scaffolded
  `CHARTER.md`.
- `__AEMONITOR_PATH__` (in `CHARTER.md`) → the absolute path of the bundled
  [`aemonitor`](../aemonitor/) sweep helper.

`--init` **never overwrites** an existing file — it scaffolds only what's missing
and reports `created` / `skipped (exists)` per file, so it's safe to re-run and
safe to hand-edit the results afterward.

## Customizing

After scaffolding, edit `~/.ae/meta-hub/{hub.config,CHARTER.md}` freely — they're
yours. Common tweaks: the model in `hub.config`, or personalizing the charter's
"your operator" references to your name and your reporting preferences.

## Dependencies

The sweep routine uses the optional [`aemonitor`](../aemonitor/) helper (Python 3
stdlib). The Telegram `say` channel uses the `ae telegram` bridge (`jq` + `curl`).
Neither is required for core ae; both are required for the hub to be useful.
