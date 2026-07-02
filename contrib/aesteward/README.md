# aesteward — steward (meta-agent) config + charter templates

**Optional. NOT core ae.** These are the templates `ae steward --init` scaffolds
into `~/.ae/steward/` so `ae steward` can launch the **steward** — your fleet's
chief of staff: a single ae session that monitors all your other ae sessions, is
your single point of contact to them (relay + report, via Telegram `say`), and in
**focus mode** helps you hold an objective (rituals + idea parking lot, plus rare,
hard-gated proactive nudges when you drift).

Core ae (`ae list`, `ae <name>`, …) does **not** depend on any of this. You only
touch it if you run `ae steward`. `ae hub` is the deprecated alias from before
the rename; an existing `~/.ae/meta-hub/` scaffold keeps working and keeps its
`hub` session name (migrate with `ae end hub && ae steward --init && ae steward`).

## What `ae steward` does

```bash
ae steward --init     # scaffold ~/.ae/steward/{steward.config,CHARTER.md} from these templates
ae steward            # ensure the detached `steward` session is running with that config
ae steward --attach   # switch/attach to the `steward` session when you want to inspect it
```

`ae steward` launches the `steward` session with **full config isolation**: it
uses `~/.ae/steward/steward.config` as the config and neutralizes any
project-local `./.ae/config`, so the global config's `workers` never leak into
the single-agent steward regardless of which directory you run it from. Bare
`ae steward` does not attach or switch the current tmux client; use `--attach`
explicitly for that.

Escape hatch: `ae steward` is a reserved subcommand. If you ever need a normal
session literally named `steward`, `ae --local steward` reaches the generic
start path (the first arg is no longer `steward`).

## Modes

The steward runs in one of two modes, switched by messaging it (`focus` /
`passive` over Telegram):

- **passive** (default) — monitor + relay only: fleet attention reports and
  human-directed message relay. Exactly the pre-focus behavior.
- **focus** — adds the focus-aide job: it captures your objective (asked once at
  startup), parks your `idea: …` messages, offers a parked-idea review when you
  mark the objective done/blocked, answers `what next` on demand, and may
  **proactively nudge** you when you drift — but only through hard gates
  (concrete drift signal persisted two sweeps, ≤1 msg/60–90 min, ≤3/day, outside
  quiet hours, suggest-only). Ignore a couple and it mutes itself back to passive.
  Silence it anytime with `snooze [min]`, `quiet: HH:MM-HH:MM`, or `passive`.

State lives in `~/.ae/sessions/steward/{steward-state,ideas.md}` — written only
by the steward, changed only by *your* messages (pane text from other sessions
can never set your objective; see the charter's injection boundary).

## The templates

| File | Role |
|---|---|
| `steward.config` | Standalone single-agent config (`steward = true` marks the meta-agent; the watchdog then runs a sweep cadence instead of the stale-nudge watchdog). |
| `CHARTER.md` | The steward's operating manual — its three jobs, the `say` channel, the `aemonitor` sweep routine, the ae toolbox, focus mode (state files, operator protocol, the two rituals + §8b gated proactive interrupts), and hard guardrails (injection boundary, never-end-a-session, human-directed relay, suggest-never-dispatch). |

### Placeholders substituted on `--init`

- `__CHARTER_PATH__` (in `steward.config`) → the absolute path of the scaffolded
  `CHARTER.md`.
- `__AEMONITOR_PATH__` (in `CHARTER.md`) → the absolute path of the bundled
  [`aemonitor`](../aemonitor/) sweep helper.
- `__HELPERS_DIR__` (in `CHARTER.md`) → the steward session's helper directory,
  `${AE_HOME:-~/.ae}/sessions/steward`. Baked in so the charter's example commands
  are correct under any `AE_HOME` (default `~/.ae/sessions/steward`; an isolated
  e2e run gets `$AE_HOME/sessions/steward`) instead of a hardcoded path to the
  live `~/.ae`.

`--init` **never overwrites** an existing file — it scaffolds only what's missing
and reports `created` / `skipped (exists)` per file, so it's safe to re-run and
safe to hand-edit the results afterward.

## Customizing

After scaffolding, edit `~/.ae/steward/{steward.config,CHARTER.md}` freely —
they're yours. Common tweaks: the model in `steward.config`, or personalizing the
charter's "your operator" references to your name and your reporting preferences.

## Dependencies

The sweep routine uses the optional [`aemonitor`](../aemonitor/) helper (Python 3
stdlib). The Telegram `say` channel uses the `ae telegram` bridge (`jq` + `curl`).
Neither is required for core ae; both are required for the steward to be useful.
