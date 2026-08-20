# B0 read-only census — events.jsonl consumers + SC-1208 transport

Provenance: delivered 2026-08-20 by gpt56luna:b0census (read-only worker; no census
artifact was written by the worker — this file is the lead's verbatim persistence of
its report). Frozen tree: 72c729343a0117af2968b66e1c43f89ad25fc0b2, verified via
`git rev-parse 72c7293^{commit}`. Lead verification: five line citations spot-checked
against `git show 72c7293:ae` (build_ae_context:1323, inject_ae_context:1470,
_goal_set_epoch:3237, _ar_facts_row region:4956, ae_emit_event:13208) — all correct.
Input to the B0 designs (b0-design.md); cited by SC-511c's consumer matrix and
SC-1208's transport table.

## A. events.jsonl consumers

Event writer/schema anchor:

- ae:13208-13256, `ae_emit_event` writes ts, actor, action, optional target/ref,
  optional actor_slot/actor_session/target_slot/target_session, optional
  summary/body_file.
- docs/internals/events.md:49-84 documents the STABLE key set — ts/actor/action/
  target/ref/summary plus the four routing keys; `body_file` is emitted but ABSENT
  from the documented set: an EMPIRICAL EXTENSION, never silently promoted
  [lead correction per colead's fold audit — the worker's original sentence claimed
  the doc defines every emitted key including body_file; it does not]. :86-106
  lists actions; :108-128 defines request pairing/latest-state/relevant-event
  behavior.

Runtime consumers in ae:

- Session goal age: ae:3237-3260 (`_goal_set_epoch`) reverse-scans action=goal, reads ts.
- Current agent states: ae:3356-3413 (`_session_states`) reverse-scans
  action=state|done; reads actor and, for state, ref; maps legacy done.
- Alert rollup: ae:3416-3565 (`_agents_alert_reasons`/`_agent_alert_reason`) reads
  actor, target, action, summary. Handles alert, throttled, throttle-cleared,
  alert-cleared, including @session:agent targets.
- Attention/request rollup: ae:3583-3707 (`_session_attn_rollup`) delegates
  states/alerts and scans action=ask|review|reply; reads ref, target, ts for
  unanswered request aging.
- List/next callers: ae:4184-4213 invokes state and attention consumers.
- Activity fallback: ae:3993-4010 (`_session_active_epoch`) consumes events.jsonl
  only as a file (mtime); no JSON fields.
- Archive facts: ae:4350-4377 (`_ar_event_facts`) reads ts from complete JSON lines
  for count/first/last; ae:4905 onward (`_ar_fingerprint`), ae:5175-5227
  (`_ar_validate_tree`), and ae:5356-5389 (`_ar_stage_payload`) copy/validate/
  fingerprint events.jsonl as a file without interpreting event fields.
- Archive request state: ae:4414-4612 (`_ar_request_states`) reads action/ref/ts and,
  for ask|review, actor/target, actor_slot/actor_session, target_slot/target_session,
  summary, body_file; handles ask, review, cancel, reply; validates reply/cancel
  sender identity.
- Archive latest state/digest: ae:4637-4664 (`_ar_latest_state`) reads action=state,
  actor, ref, ts, summary; ae:4782-4895 (`_ar_render_digest`) consumes latest
  state/request rows and prints raw events.jsonl; ae:4937-4940 (`_ar_facts_row`)
  calls the facts/request sensors.
- Compact handover: ae:5833-5851 (`_compact_reply_seen`) accepts action=reply, reads
  ref, actor_slot, actor_session; ae:5854-5868 (`_compact_find_outstanding`) consumes
  archive request rows (status/from/ref/body); callers ae:5939 and ae:6284-6305.
- Stop verification: ae:7190-7197 (`_stop_result_for_op`) selects action=stop-result
  and operation tag; ae:7200-7205 (`_stop_result_ok`) checks summary text for the
  verified-stopped phrase. User-facing references ae:7055,7115,7230-7239.
- State helper: ae:13259-13296 (`ae_latest_state_for`) reads actor/action; state
  reads ref/summary/ts, done reads summary/ts.
- Request helper/reply: ae:13490-13538 (`ae_find_request`) reads ref, action=
  ask|review, actor, target, actor_slot, actor_session, target_slot, target_session.
  Reply caller ae:14343; request listing ae:14422-14455.
- Events pane: generated helper functions ae:14827-14905 parse ts, actor, action,
  target, summary; `tail -n 30 -f` is file-level.
- Watchdog: ae:15107-15124 (`_last_event_age`) reads actor/ts; ae:15126-15177
  (`_latest_relevant_event`) reads actor/target/action/ts/ref/summary, recognizes
  target=@session:agent; ae:15179-15244 (`_agent_quiet_reason`) consumes
  action/ts/ref/actor/summary and quiet state refs done|waiting-user|blocked, with
  legacy done. Generated watchdog calls at ae:16163,16298,16437.
- Telegram daemon: ae:9471-9487 `event_action_allowed` reads action; :9500-9516
  `format_event` reads action/actor/target/summary; :10101-10175 `process_session`
  tails complete JSON lines and filters/forwards; :9690-9713 list uses events file
  mtime; :9827-9854 parses the formatted header for session/actor reply routing (not
  raw JSON fields).
- Resume retention: ae:18046-18077 counts/tails/moves events.jsonl under its lock;
  no JSON fields.

Runtime consumers in contrib/aewatch:

- aewatch:1372-1377 filters action; :1380-1393 formats action/actor/target/summary;
  :1396-1408 forwards; :1463-1508 tails/decodes complete event objects, handles
  inode/truncation/offset; :1608-1620 drains outbound sessions.
- aewatch:2029-2060 reads actor/ts for age; :2063-2098 reads
  actor/target/action/ts/ref/summary for latest relevant event; :2104-2121 maps
  action=done and action=state with quiet refs; :2267-2314 reads
  actor/target/action/summary for alert/throttle/throttle-cleared/alert-cleared;
  run_watchdog_cycle invokes at :2627,2680.
- The bridge's event-only path forwards the decoded event object; body_file is not
  converted into peer text by the bridge reader.

Frozen tests that consume/assert event fields:

- tests/unit:1764-2052 latest-state/parser (actor/action/ref/summary/ts); :2346-2604
  session states/alerts (state, done, send, alert, throttled + clear actions, plus
  actor/target/ref/summary); :6045-6154 relevant/quiet fixtures; :6567-6790 request
  sensors and :9475 routing slot fields; :9967-10054 send/event-body reliability;
  :10885-10886 spawn/failure actions; :11280-11626 archive
  facts/state/request/body_file; :11976-12101 and :12912-12996 compact request/reply;
  :12229 onward retention; :1501 and :850-1301 launch/context (event-adjacent).
- tests/integration:359-452 state/chat; :1094-1105 alert; :1121-1143 next;
  :1245-1281 ask/review/ref/target/slot routing; :1597-1642 live sync/request
  sensor; :1708-1722 retention; :2046-2047 goal; :2358-2359 JSON robustness;
  :3368-3864 stop-request/stop-result/summary/tag; :4131-4134 archive fixtures;
  :4822-5009 archive ask/reply/cancel/ref; :5209-5232 compact handover; :5531 actor
  count.
- tests/aewatch_tests: test_02_harness.py:56-60,116; test_10_multitick.py:65,104;
  test_13_activity_parity.py:84; test_14_nudge_parity.py:98-101;
  test_15_quiet_parity.py:42,49,98,176,181; test_16_alert_parity.py:116-143;
  test_17_throttle_parity.py:111-131; test_18_sweep_parity.py:124,148-208;
  test_19_recover_parity.py:68-78; test_20_telegram_parity.py:114;
  test_23_p3_tick_schema.py:46-119 (wrapped session/event/action);
  test_27_real_ae_boundaries.py:58-91 (JSONL/inode/lock);
  test_36_outbound_format.py:35-86 (action/actor/target/summary/filter); further
  literal events.jsonl consumers in test_37_outbound_state.py,
  test_39_bridge_tick.py, test_44_bridge_transport_live.py.
- tests/e2e/ai/lib.sh:83-118 event-file/JSONL presence;
  tests/e2e/ai/cross-agent/steps.sh:45 reply action; steward guards check
  absence/presence of lifecycle actions.

Frozen documentation consumers/contracts:

- docs/internals/events.md:3-45 names watchdog, requests, events-tail readers;
  :47-144 schema/action/request/latest-state contract.
- docs/internals/architecture.md:324-335 event producers/readers + events pane.
- docs/internals/bridge-protocol.md:13-16,52,65,76-97,123-143 bridge tailing,
  action/filter behavior, additive-key/additive-action compatibility.
- docs/internals/watchdog.md:73-102,145-149; docs/reference/telegram.md:3-13,69,
  167-169; docs/reference/commands.md:323-375; docs/reference/helpers.md:13-41;
  docs/internals/monitor.md:9,34-62; docs/troubleshooting.md:73-77,122;
  docs/lead-handover.md:43,106-115; contrib/aewatch/README.md:7 and
  contrib/aewatch/CONTRACTS.md event sections around :527,590,793.

Minimal producer-derived fixture matrix (observe transitions only; no expected
values):

1. F0 baseline: one valid session/meta roster (main + one worker), a known
   events.jsonl inode, a pane/consumer fixture. Seed only producer-shaped rows if a
   consumer needs a prior state/request.
2. Add: run the real `_cmd_spawn` success path (ae:11845-12040; success emit at
   :12084-12085; `_spawn_emit_event` at :12089-12114). Observables: appended spawn
   row, new roster/meta slot + pane. The failure branch (:12068-12073) is a separate
   producer case for spawn-failed if error handling is in scope.
3. Rename: run `cmd_rename` (ae:11547-11645). Frozen source emits NO action=rename.
   Observe session/tmux/meta/path rename and continuity/movement of events.jsonl.
   For churn-resistant request consumers, seed a producer ask/reply pair
   before/after rename with stable slot/session routing fields and changed display
   names; do not assert projected values.
4. Remove: run `_cmd_retire` (ae:12117-12262; retire emit at :12261-12262).
   Observables: appended retire row, roster/meta removal, pane disappearance. For
   whole-session stop/remove, add the producer stop-request/stop-result sequence
   and directory disappearance.
5. Replay F0→add→rename→remove through `_session_states`/`_agents_alert_reasons`/
   `_session_attn_rollup`, `_ar_request_states`, compact sensors, events-tail,
   aewatch, Telegram formatter, and archive staging. Producer rows and
   filesystem/meta/pane transitions are the only fixture inputs; no hard-coded
   IS/projected outputs.

## B. SC-1208 transport census

Common launch boundary:

- Context assembled by `build_ae_context` ae:1323-1438 from session meta
  (mode/origin/layout), validated roster identity, workdir/session text, hard-coded
  helper/rule text, role/mode blocks, parent archive pointer/counts, and config
  prompt.instructions (local overrides global, then appends). Peer/pane text is NOT
  an input to this function.
- `inject_ae_context` ae:1470-1541; OpenCode file setup `_opencode_context_files`
  ae:1455-1468.
- Every launch writes a shell script via `_emit_launch_script` ae:853-878, then
  `send_agent_cmd` ae:12587-12642 pastes the quoted script PATH into the pane. The
  script executes the final command; the launch command is transported by tmux as a
  path, not as the context body.
- `tmux_paste_submit` ae:407-418 and `ae_submit_pasted_message` ae:14052-14115 load
  message text into a tmux buffer, paste to the pane (Codex bracketed-paste where
  applicable), then Enter with bounded readiness/staged checks.
- Spawn source/peer text: `_cmd_spawn` ae:11845-12040. Normal send/ask/review/reply:
  `helper_send_main` ae:14160-14284, ask :14286-14295, review :14298-14312, reply
  :14315-14404. Full message body stored separately; only a capped summary is event
  material.
- Pane capture is sensing/gating ONLY: `_capture_input_region` ae:13541-13569,
  `_spawn_input_ready`/`_wait_input_ready` ae:11808-11843. `_deliver_launch_prompt`
  ae:12661-12698 durably writes undelivered.launch-<slot>.txt and emits
  launch-delivery-failed on timeout; it does not turn pane text into context.

Per-tool transport table (system/context source | peer text | final boundary |
pane-to-instruction path):

- **Claude Code**: context via `--append-system-prompt` (inject ae:1477-1481);
  `build_launch_command` ae:612-621 prefixes env -u CLAUDECODE -u
  CLAUDE_CODE_SESSION, CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=0; session id/resume
  ae:1033-1110,1112-1136. `initial_prompt_for_cmd` empty (ae:1543-1560) — spawn brief
  and peer messages are separate user-turn pastes. Final boundary: launch script →
  argv; later peer body via tmux load-buffer/paste/Enter. No pane text enters
  --append-system-prompt; the validated spawn identity name can enter the identity
  line (ae:1347-1362), not arbitrary prose.
- **Codex**: context + CRITICAL FIRST TASK _register-sid instruction + optional
  AE_CODEX_LAUNCH_ID/AE_CODEX_SLOT marker built ae:1482-1498, injected as
  `-c developer_instructions=…`. `initial_prompt_for_cmd` returns Go (ae:1550-1552);
  fresh spawn appends the caller prompt to the launch positional user text
  (ae:11964-11969); resume takes no inline prompt — launch text defers to
  `_deliver_launch_prompt` after readiness. No pane text enters
  developer_instructions; _register-sid instruction is ae-generated.
- **Gemini CLI**: context + optional launch marker + "context only / wait" suffix
  built ae:1500-1513, passed as `-i` (initial user-turn material, not a system
  flag); capture/resume ae:1087-1093,1138-1160. Spawn/peer text are separate tmux
  pastes. No pane/peer text enters -i context.
- **Grok Build**: context + context-only/wait suffix passed as positional [PROMPT]
  (inject ae:1515-1523); no --system-prompt-override/--system-prompt (replaces
  grok's own agent prompt); session id/resume ae:1095-1106,1118-1135. Spawn/peer
  text are separate tmux pastes. Only the validated spawn identity reaches the
  identity line.
- **OpenCode**: `_opencode_context_files` ae:1455-1468 writes context markdown at
  meta/opencode.<safe-slot>.md (0600) + JSON config meta/opencode.<slot>.json (0600)
  whose instructions array points at the markdown; inject ae:1525-1537 prefixes
  `env OPENCODE_CONFIG=<config>`; resume ae:1079-1085. `initial_prompt_for_cmd`
  empty ae:1552-1557; no deferred launch-prompt branch (ae:12640-12642). Final
  boundary: launch script → env var → JSON config path → markdown instructions →
  OpenCode; peer text via tmux paste. No pane/peer text enters config or markdown.
  Operator-config instruction-array merge is external OpenCode behavior, not a
  pane/peer path.
- **Unsupported/other command**: ae:1539 and ae:1558 leave context/prompt injection
  unsupported — passthrough, no modeled launch transport. A GAP if SC-1208 requires
  a guarantee for arbitrary commands.

Peer/pane ingress conclusion:

- spawn user_prompt is delivered as a user turn, never concatenated into
  build_ae_context. send/ask/review/reply text goes through tmux user-message
  transport; external/event-only targets stay in events.jsonl/body_file.
- Pane text is read only by readiness/staged/busy/hash sensors
  (ae:11808-11843,13541-13569); it is never appended to system/developer/context
  material.
- A spawn name is copied into roster/meta and can reach the validated identity
  fragment; `_cmd_spawn` validates at ae:11853-11858 and `build_ae_context`
  revalidates both halves at ae:1347-1362 — arbitrary instruction prose cannot
  enter through that route in the frozen source.
- prompt.instructions is operator config input (ae:1434-1437), not peer/pane text.
- Supporting frozen tests: tests/unit:850-1003 (tool injection), :1005-1027
  (initial prompts), :1205-1301 (OpenCode config/env/files + command
  classification), :9622-9722 and :10163-10297 (readiness, Codex fresh/resume
  delivery), identity/context guards around :1501 and :11917-11937.

## Gaps and risks (worker-reported, standing)

- Rename has no producer event action in 72c7293; consumers expecting an
  event-level rename must observe session/meta/tmux/path effects or a surrounding
  producer sequence.
- events.jsonl has append/mtime/file-lifecycle consumers in addition to JSON-field
  consumers; a fixture that only changes parsed fields misses
  inode/retention/archive/Telegram tail behavior.
- Legacy event rows may lack slot/session routing keys; request consumers have
  explicit display-name fallbacks — fixtures need both pre-routing and
  slot/session-qualified producer shapes when compatibility is under test.
- The Grok and unsupported-command launch surfaces are less modeled than
  Claude/Codex/Gemini; OpenCode's instruction-file semantics are partly external
  to ae.
- Pane capture is a sensor boundary, not an instruction boundary; testing must
  preserve that distinction.
