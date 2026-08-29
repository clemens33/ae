# aeorchestrator — orchestrator (meta-agent) config + charter templates

**Optional. NOT core ae.** These are the templates `ae orchestrator --init` scaffolds
into `~/.ae/orchestrator/` so `ae orchestrator` can launch the **orchestrator** — your fleet's
chief of staff: a single ae session that monitors all your other ae sessions, is
your single point of contact to them (relay + report, via Telegram `say`), and —
whenever you've told it your objective — helps you hold it (rituals + idea
parking lot, plus rare, hard-gated proactive nudges when you drift).

Core ae (`ae list`, `ae <name>`, …) does **not** depend on any of this. You only
touch it if you run `ae orchestrator`. `ae hub` is the deprecated alias from before
the rename; an existing `~/.ae/meta-hub/` scaffold keeps working and keeps its
`hub` session name (migrate with `ae end hub && ae orchestrator --init && ae orchestrator`).

## What `ae orchestrator` does

```bash
ae orchestrator --init     # scaffold ~/.ae/orchestrator/{orchestrator.config,CHARTER.md} from these templates
ae orchestrator            # ensure the detached `orchestrator` session is running with that config
ae orchestrator --attach   # switch/attach to the `orchestrator` session when you want to inspect it
```

`ae orchestrator` launches the `orchestrator` session with **full config isolation**: it
uses `~/.ae/orchestrator/orchestrator.config` as the config and neutralizes any
project-local `./.ae/config`, so the global config's `workers` never leak into
the single-agent orchestrator regardless of which directory you run it from. Bare
`ae orchestrator` does not attach or switch the current tmux client; use `--attach`
explicitly for that.

Escape hatch: `ae orchestrator` is a reserved subcommand. If you ever need a normal
session literally named `orchestrator`, `ae --local orchestrator` reaches the generic
start path (the first arg is no longer `orchestrator`).

## The objective is the switch (no modes)

There is no mode to configure or remember. The orchestrator always monitors and
relays; the focus-aide side arms itself the moment you set an objective:

- **No objective set** — monitor + relay only: fleet attention reports and
  human-directed message relay. It asks "what's today about?" once at startup
  (once per day, ignorable — no re-nag).
- **Objective active** (`objective: <text>` over Telegram) — it holds it, parks
  your `idea: …` messages, offers a parked-idea review when you mark the
  objective done/blocked, answers `what next` on demand, and may **proactively
  nudge** you when you drift — but only through hard gates (concrete drift
  signal persisted two sweeps, ≤1 msg/60–90 min, ≤3/day, outside quiet hours,
  suggest-only). Ignore a couple and it self-mutes for the day.
  Silence it anytime with `snooze [min]`, `quiet: HH:MM-HH:MM`, or
  `drop objective`. (Legacy `focus`/`passive` messages still map sensibly.)

State lives in `~/.ae/sessions/orchestrator/{orchestrator-state,ideas.md}` — written only
by the orchestrator, changed only by *your* messages (pane text from other sessions
can never set your objective; see the charter's injection boundary).

## The templates

| File | Role |
|---|---|
| `orchestrator.config` | Standalone single-agent config (`orchestrator = true` marks the meta-agent; the watchdog then runs a sweep cadence instead of the stale-nudge watchdog). |
| `CHARTER.md` | The orchestrator's operating manual — its three jobs, the `say` channel, the `aemonitor` sweep routine, the ae toolbox, the objective-armed focus aide (state files, operator protocol, the two rituals + §8b gated proactive interrupts), and hard guardrails (injection boundary, never-end-a-session, human-directed relay, suggest-never-dispatch). |

### Placeholders substituted on `--init`

- `__CHARTER_PATH__` (in `orchestrator.config`) → the absolute path of the scaffolded
  `CHARTER.md`.
- `__AEMONITOR_PATH__` (in `CHARTER.md`) → the absolute path of the bundled
  [`aemonitor`](../aemonitor/) sweep helper.
- `__HELPERS_DIR__` (in `CHARTER.md`) → the orchestrator session's helper directory,
  `${AE_HOME:-~/.ae}/sessions/orchestrator`. Baked in so the charter's example commands
  are correct under any `AE_HOME` (default `~/.ae/sessions/orchestrator`; an isolated
  e2e run gets `$AE_HOME/sessions/orchestrator`) instead of a hardcoded path to the
  live `~/.ae`.

`--init` **never overwrites** an existing file — it scaffolds only what's missing
and reports `created` / `skipped (exists)` per file, so it's safe to re-run and
safe to hand-edit the results afterward.

## Customizing

After scaffolding, edit `~/.ae/orchestrator/{orchestrator.config,CHARTER.md}` freely —
they're yours. Common tweaks: the model in `orchestrator.config`, or personalizing the
charter's "your operator" references to your name and your reporting preferences.

## Dependencies

The sweep routine uses the optional [`aemonitor`](../aemonitor/) helper (Python 3
stdlib). The Telegram `say` channel uses the `ae telegram` bridge (`jq` + `curl`).
Neither is required for core ae; both are required for the orchestrator to be useful.
