# aeorchestrator — orchestrator (meta-agent) config + charter templates

**Optional. NOT core ae.** These are the templates you install into
`~/.ae/orchestrator/` to get the **orchestrator** — your fleet's chief of staff: a
single ae session that monitors all your other ae sessions, is your single point of
contact to them (relay + report, via Telegram `say`), and — whenever you've told it
your objective — helps you hold it (rituals + idea parking lot, plus rare,
hard-gated proactive nudges when you drift).

Core ae (`ae list`, `ae <name>`, …) does **not** depend on any of this, and a machine
that never installed the scaffold is never touched by it.

## Installing it: copy two files, substitute three paths

There is no `ae orchestrator` command any more. The subcommand was a bash trampoline
that scaffolded these templates and then re-entered the normal launch path; it was cut
from the glue along with the rest of ae's bash, and `ae orchestrator` now refuses with a
message rather than doing something surprising. Installing is a copy:

```bash
mkdir -p ~/.ae/orchestrator
cp contrib/aeorchestrator/orchestrator.config ~/.ae/orchestrator/
cp contrib/aeorchestrator/CHARTER.md          ~/.ae/orchestrator/
```

Then replace every `__PLACEHOLDER__` in both files with a real absolute path — nothing
substitutes them for you, and one left in place lands **verbatim in the agent's system
prompt**:

| Placeholder | In | Replace with |
|---|---|---|
| `__CHARTER_PATH__` | `orchestrator.config` | `~/.ae/orchestrator/CHARTER.md`, absolute |
| `__HELPERS_DIR__` | `CHARTER.md` | the orchestrator session's helper dir, `${AE_HOME:-~/.ae}/sessions/orchestrator` |
| `__AEMONITOR_PATH__` | `CHARTER.md` | wherever you installed [`aemonitor`](../aemonitor/), absolute |

`__HELPERS_DIR__` is a placeholder rather than a hardcoded `~/.ae/...` on purpose: an
isolated run (tests, e2e) sets its own `AE_HOME`, and a charter that named the live
`~/.ae` would point that run's orchestrator at your real fleet.

## How it starts: the scaffold's existence is the switch

Once `~/.ae/orchestrator/orchestrator.config` is on disk, **any** `ae <session>` launch
brings the orchestrator up as a detached **companion** session named `orchestrator`. ae
starts it itself — it is a launch like any other, with that file as its config and any
project-local `./.ae/config` neutralized, so your global roster's `workers` never leak
into the single-agent orchestrator whatever directory you launched from.

- It is **opt-in by that file existing**. No config key, no flag.
- `AE_NO_AUTOSTART=1 ae <session>` starts neither companion (this one or the watchdog).
- It never starts a second copy: a launch whose own session is named `orchestrator` (or
  `hub`) is the companion, and does not recurse.
- A launch will not start one it cannot rule out — if tmux does not answer, ae says so
  and skips rather than risking a duplicate.

To look at it, treat it as the ordinary session it is: `ae orchestrator` is not a
command, but the session is reachable with the session-switching you already use
(`ae next --attach`, or tmux directly). To stop it for good, remove the scaffold.

`~/.ae/meta-hub/hub.config` is the pre-rename layout. It still works and keeps its `hub`
session name, so its baked charter paths and resume state stay consistent. To move to the
canonical one: `ae end hub`, then install as above.

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

## Customizing

Edit `~/.ae/orchestrator/{orchestrator.config,CHARTER.md}` freely — they're yours, and ae
only ever reads them. Common tweaks: the model in `orchestrator.config`, or personalizing
the charter's "your operator" references to your name and your reporting preferences.

## Dependencies

The sweep routine uses the optional [`aemonitor`](../aemonitor/) helper (Python 3
stdlib). The Telegram `say` channel uses the `ae telegram` bridge, which is the ae core
binary and needs no `jq` or `curl`. Neither is required for core ae; both are required
for the orchestrator to be useful.
