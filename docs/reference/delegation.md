# Delegation — leads, tiers, and spawned workers

The era of running the strongest model for everything is over. The pattern
that works: **the lead (and the steward) stay on a frontier model; bounded
subtasks go to spawned workers on cheaper/faster tiers; workers get retired
when reviewed.**

Honest economics up front: multi-agent work can cost *more* tokens in total —
Anthropic's own numbers ([multi-agent research system](https://www.anthropic.com/engineering/multi-agent-research-system))
put a lead+workers setup at ~15× the tokens of a single chat while beating
single-agent quality by ~90% on their internal research eval. What you are buying is
**context hygiene** (a worker burns tens of thousands of tokens exploring and
returns a 1–2K distilled summary — the lead's strategic context stays clean)
and **parallelism**. The cheap tiers keep that affordable; they are the tuning
knob, not the point.

## The tiers

ae needs no mechanism for this — an alias *is* a tier (see
[Configuration](../getting-started/config.md#model-tiers-recommended-aliases)
for the recommended `fast` / `standard` / `optimal` / `best` + `codex` /
`codexfast` block). Role guidance:

| Role | Tier |
|---|---|
| Session lead, steward | `optimal` / `best` |
| Cross-model review | `codex` (different model family = different blind spots) |
| Chores, test runners, CI/CD, scouts | `fast` / `codexfast` |
| Scoped implementation | `standard` |

## When to spawn a worker

Delegate when the task **specs in ~10 lines, has a clear stop condition, and
the result is verifiable** by tests, grep, or a focused review:

- test/CI runs ("run `just test-unit`; report failures only")
- read-heavy scans ("find every caller of X; reply file:line list")
- scoped mechanical edits (renames, doc-table updates, fixture refreshes)
- log/bug triage, reproduction, exact-failure collection
- independent review lanes (security, test-gaps, API compatibility)

Keep it yourself when the hard part is judgment: architecture, ambiguous
debugging, final integration, user-facing decisions — or when briefing the
worker would require half your conversation (the hygiene gain is gone).

Use your harness's **in-process subagents** (e.g. Claude Code's Task tool)
for ephemeral same-harness read/research; use ae `spawn` when you want a
different model/harness, a visible pane the human can peek and message, an
independent lifetime, or durable events.

## The contracts

**Naming**: alias = tier, name = role. `fast:tests`, `codexfast:callers`,
`standard:docs-sync` — never `worker`, `helper-3`.

**Brief** (what the lead sends): objective, allowed scope/files, verification
command, expected reply shape, whether edits are allowed.

**Result** (what the worker replies): `Outcome / Changed / Verified (command +
result) / Risks / Need from lead`. No raw logs unless asked.

**Lifecycle**: worker declares `state working` on start, `state done` when
finished, then waits. The lead reviews the output/diff, then `retire <name>`.
Workers never self-retire — the pane must survive until reviewed. `memo` is
for durable findings that outlive the pane, never chat transcripts.

**File ownership**: one writer per file. In `--local` mode the lead assigns
scope; for parallel write-heavy work use separate worktrees or sessions.

## Example round-trip

```bash
# lead:
~/.ae/sessions/myfeat/spawn fast:tests "Objective: run 'just test'; report
failures only, format: Outcome/Verified/Risks. Read-only — no edits."
# worker (when done): declares 'state done', replies with the summary
# lead: reviews, then
~/.ae/sessions/myfeat/retire tests
```

## What ae deliberately does NOT do

No model router or auto-selection (judgment call), no per-spawn model flags
(aliases already encode tiers), no cost tracking, no auto-retire (destroys
inspectability), no task queue (`events.jsonl`/`requests`/`memo` are enough),
and the steward never dispatches workers on its own — it suggests, you decide.
