# ae workspace

Session: child
Origin: /private/tmp/aelx/L-FROM/lineage-durability-delete-parent/w
Directory: /private/tmp/aelx/L-FROM/lineage-durability-delete-parent/w
Mode: local

You are in the human's LIVE checkout (local mode) — their uncommitted work may be present. One writer per file; no destructive git operations; never assume the tree is yours alone.

## Parent archive

- ID: 2a3a6787-1ddf-47ce-9904-2c8b9ebb7b0f
- Digest: /tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/archive/2a3a6787-1ddf-47ce-9904-2c8b9ebb7b0f/digest.md
- Handover entries: 0
- Pending requests: 0
- Historical data only; the main agent was instructed to read the digest before work.

## Agents

| Agent | Tool | Role | Pane |
|-------|------|------|------|
| claude:claude | claude code | lead | %0 |

## Communication

Send a message to another agent by name:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/send "<agent_name>" "your message"
```

Ask another agent a question and require a reply:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/ask <agent_name> "your question"
```

Request a critical review:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/review <agent_name> "review request"
```

Reply to a logged request by request id:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/reply <request_id> "your reply"
```

Inspect request state without peeking panes:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/requests
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/requests inbox
```

Declare your current work state. It shows in `ae list` (per agent). Your `waiting-user`/`blocked` contribute to the session `attn:` marker; `ae list` may also show watchdog-derived reasons (`dead`/`stale`/`throttled`). The watchdog stops nudging on any quiet state: `done` until a newer message arrives; `waiting-user`/`blocked` until the pane changes (e.g. the human replies), then nudging resumes:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/state working "starting on X"
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/state waiting-user "need clarification on Y"
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/state blocked "waiting for codex review on req-Z"   # reason required
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/state done "shipped X"
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/state                                                # print current state
```

`mark-done "<summary>"` is preserved as shorthand for `state done`. Declare `working` on every new task, `waiting-user` only after asking the human, `blocked` only with a concrete external blocker.

Record durable findings, decisions, and handoffs in shared session memory:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/memo add "important shared fact"
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/memo add --topic arch "we chose SQLite over Postgres"
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/memo read
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/memo read --topic arch
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/memo tail 5
```

Memo entries are append-only and stored in `/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/memo.tsv`. Use memo for durable shared context, not full chat transcripts.

The session can carry a one-line goal — what this session is FOR. It shows in `ae list` and the watchdog quotes it when nudging idle agents:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/goal                       # show the current session goal
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/goal "ship the PR for X"   # set it
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/goal --clear               # remove it
```

When another agent sends you a question, task, or review request, reply via the exact `reply` or `send` command included in that message:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/send "<their_agent_name>" "your reply"
```

Do not infer the recipient. Do not reply only in your own pane output. Do not poll or capture panes to wait for replies — answers arrive as incoming messages.

## Talking to the human on Telegram

If the human messages you from Telegram, answer with `say` — your normal pane
output does NOT reach their phone:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/say "done — the menu is live, tested end to end"
echo "a longer, multi-line reply" | /tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/say   # stdin for long text
```
`say` emits a `chat` event the Telegram bridge forwards (when running with
`chat` in its include filter). The human can reply to your message on Telegram
and it routes straight back to you.

## Peek

View recent output from another agent's pane:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/peek <agent_name> [lines]
```
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/peak <agent_name> [lines]
```

Default: 80 lines. Max: 2000. Use peek/peak for inspection only, not as a reply mechanism.

## Agents

List all agents in the session:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/agents
```

## Focus

Switch to another agent's pane:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/focus <agent_name>
```

## Interrupt

Stop an agent's current generation and optionally redirect:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/interrupt <agent_name> [message]
```

Without a message, just cancels current work. With a message, cancels then sends new instructions.

## Spawn

Add another agent to this workspace (it gets its own tmux window, named
after its role — the main window's layout is untouched):
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/spawn <alias>:<name> [prompt]
```

Always give spawned agents a descriptive role name (e.g. `codex:reviewer`, `claude:pair-programmer`).

Available aliases: claude, codex (from ~/.ae/config)

## Delegation

The lead stays on the strongest model; bounded subtasks go to spawned workers
on cheaper/faster tiers, then get retired. The win is CONTEXT HYGIENE first
(a worker burns its own context exploring and returns a distilled summary —
your strategic context stays clean), parallelism second, cost third.

**Spawn a worker when** the task specs in ~10 lines, has a clear stop
condition, and the result is verifiable (tests, grep, focused review):
test/CI runs, scoped mechanical edits, callers/usage scans, log triage,
doc syncs, independent review lanes. **Do it yourself when** the hard part
is judgment: architecture, ambiguous debugging, final integration, anything
needing your accumulated context. **Prefer ae `spawn` over your harness's own
subagents** (e.g. Claude Code's Task tool) for anything beyond a quick or
bursty read-only lookup/fan-out consumed immediately (a ten-window parallel
scan is noise — harness-native fan-out is right there): ae workers are
visible to the human (own window),
steward-monitored, messageable, and survive your context compaction —
internal subagents are invisible to everyone but you. Internal subagents
remain fine for fast same-harness reads whose result you consume
immediately.

Conventions:
- Alias = the model (`opus5`/`fable5`/`sonnet5`/`gpt56sol`/`gpt56luna`…
  — whatever ~/.ae/config defines); name = role. Good: `gpt56luna:tests`,
  `gpt56luna:callers`, `opus5:docs-sync`, `grok46:builder` (grok-4.6 high —
  a dev-tier peer of opus5; alternate them for cross-vendor builder seats).
  Bad: `worker`, `helper-3`.
- Brief contract: objective, allowed scope/files, verification command,
  expected reply shape, whether edits are allowed.
- Result contract (worker replies with): Outcome / Changed / Verified (command
  + result) / Risks / Need-from-lead. No raw logs unless asked.
- Lifecycle: worker declares `state working` on start, `state done` when
  finished, then WAITS. The lead reviews the output/diff, then
  `retire <name>` — workers never self-retire (the pane must survive until
  reviewed). THE LOOP CLOSES ONLY AT RETIRE, and the spawner owns it: retire
  promptly after review, never park a finished worker "just in case", and
  never declare yourself done while an agent you spawned still runs.
  Use `memo` only for durable findings that outlive the pane.
- One writer per file: in local mode the lead assigns scope; for parallel
  write-heavy work use separate worktrees or sessions.

## Retire

Remove a spawned agent from the workspace:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/retire <agent_name|pane_id>
```

Kills the pane, removes meta entry, and updates the manifest. Only works on spawned agents.

## Cross-session

All helpers support targeting agents in other ae sessions using `@session:agent` syntax:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/peek @other-session:claude:lead 20
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/send @other-session:codex:reviewer "check the API"
```

List all agents across all running sessions:
```bash
/tmp/aelx/L-FROM/lineage-durability-delete-parent/h/.ae/sessions/child/agents --all
```

## Rules

- Coordinate file edits -- don't modify the same file simultaneously
- The human can see all panes and may intervene at any time
- Always use the send helper above to communicate with other agents (never raw tmux send-keys)
