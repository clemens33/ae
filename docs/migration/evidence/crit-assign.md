# CRIT-ASSIGN table — exact batch/arm per outstanding critical id

Lead-authored per the cluster-plan gate; one line per CRITICAL id in
ratification-critical.md. Checker-asserted as the expected/assignment input
(sweep-check arg 4; default once the dormancy delta lands). Batches per
cluster-plan.md as amended: every batch design seat-approved pre-spawn; arms fail
independently; captures are candidate observations until seat acceptance.

CRIT-ASSIGN: D01 | C | read-side fixture cluster: `list [--json]`
CRIT-ASSIGN: D02 | C | read-side fixture cluster: `requests` query (generated helper)
CRIT-ASSIGN: D03 | C | read-side fixture cluster: events queries (`events-tail` helper, dispatcher event reads)
CRIT-ASSIGN: D04a | C | read-side fixture cluster: `status`
CRIT-ASSIGN: D04b | C | read-side fixture cluster: `next`
CRIT-ASSIGN: SC-013 | H-HELPER | steward --help/--detach invocation arm, captured usage/rc
CRIT-ASSIGN: SC-014 | H-HELPER | version output capture arm
CRIT-ASSIGN: D14b | H-HELPER | doctor --refresh launch-artifact half: regenerate and diff launch.<slot>.sh publication path
CRIT-ASSIGN: SC-016a | C | read-side fixture cluster: `ae status [name]` signature
CRIT-ASSIGN: SC-016b | C | read-side fixture cluster: status prints ~80 labeled lines per agent
CRIT-ASSIGN: SC-016c | C | read-side fixture cluster: status defaults to the current session when run inside one
CRIT-ASSIGN: SC-016d | C | read-side fixture cluster: status never attaches
CRIT-ASSIGN: SC-017a | C | read-side fixture cluster: `list` shows running sessions only by default
CRIT-ASSIGN: SC-017b | C | read-side fixture cluster: `--all` shows running sessions, then stopped
CRIT-ASSIGN: SC-017c | C | read-side fixture cluster: `--stopped` shows stopped sessions only
CRIT-ASSIGN: SC-017d | C | read-side fixture cluster: `--needs-attn` filters to attention sessions; aliases accepted
CRIT-ASSIGN: SC-017e | C | read-side fixture cluster: `--active` filters on recent activity
CRIT-ASSIGN: SC-017f | C | read-side fixture cluster: `--json` honours the active filters
CRIT-ASSIGN: SC-017g | C | read-side fixture cluster: attention rollup arm — fixtures per severity, marker equals highest-severity reason
CRIT-ASSIGN: SC-017h | C | read-side fixture cluster: tabular view arm — per-agent health/state/attn columns present
CRIT-ASSIGN: SC-017i | C | read-side fixture cluster: `--running` is the explicit spelling of the default filter
CRIT-ASSIGN: SC-018b | C | use-against-existing-session arm: capture decision surface
CRIT-ASSIGN: SC-019 | C | read-side fixture cluster: `jump` is an alias of `next`
CRIT-ASSIGN: SC-020a | C | read-side fixture cluster: `next --attach` switches inside tmux, attaches outside
CRIT-ASSIGN: SC-020b | C | read-side fixture cluster: `--attach` re-checks the session still exists first
CRIT-ASSIGN: SC-020c | C | read-side fixture cluster: `--attach` no-ops with a message when already current
CRIT-ASSIGN: D24 | B0 | negative-evidence artifact: scoped writer-enumeration absence assertion with demonstrated red arm
CRIT-ASSIGN: D25 | T-WD | watchdog branch harness: watchdog daemon (mode-split; measured, colead 2026-08-20; census-3 audited)
CRIT-ASSIGN: D27 | T-STORE | store/handoff harness: telegram bridge (runtime handoff; corrected per gate finding fe7cfc2e, blocker 6)
CRIT-ASSIGN: D28c | T-CTRL | daemon control harness: telegram status (read)
CRIT-ASSIGN: D30a | F-CONTRIB | contrib/code audit: static template consumption + provenance
CRIT-ASSIGN: D30b | F-CONTRIB | contrib/code audit: aemonitor state-writer scope enumeration, asserted outside ae state
CRIT-ASSIGN: D30c | T-STORE | store/handoff harness: aewatch internals
CRIT-ASSIGN: SC-101 | C | read-side fixture cluster: the running-session fast path's mutation exclusion
CRIT-ASSIGN: SC-102a | C | read-side fixture cluster: resume of a stopped session
CRIT-ASSIGN: SC-102b | C | read-side fixture cluster: invocation from inside a session
CRIT-ASSIGN: SC-200 | H-DELIVERY | delivery rig: delivery-model evolution
CRIT-ASSIGN: SC-201 | H-DELIVERY | delivery rig: text is never pasted into a shell
CRIT-ASSIGN: SC-202 | H-DELIVERY | delivery rig: a human's unsent input is never clobbered
CRIT-ASSIGN: SC-204 | H-DELIVERY | delivery rig: no durable outbox (until DR-004)
CRIT-ASSIGN: SC-209a | H-DELIVERY | delivery rig: requests and replies are addressed by slot + session
CRIT-ASSIGN: SC-209b | H-DELIVERY | delivery rig: reply verifies the sender's live slot against the stored slot
CRIT-ASSIGN: SC-209c | H-DELIVERY | delivery rig: the display name is never trusted for routing
CRIT-ASSIGN: SC-209d | H-DELIVERY | delivery rig: routing survives display-name churn
CRIT-ASSIGN: SC-210 | H-DELIVERY | delivery rig: the unprotected-delivery degradation retires
CRIT-ASSIGN: SC-211a | H-HELPER | helper surface run: `state` refusal/malformed modes
CRIT-ASSIGN: SC-211b | H-HELPER | helper surface run: `goal` refusal/malformed modes
CRIT-ASSIGN: SC-211c | H-HELPER | helper surface run: `memo` refusal/malformed modes
CRIT-ASSIGN: SC-211d | H-HELPER | helper surface run: `requests` refusal/malformed modes
CRIT-ASSIGN: SC-211e | H-HELPER | helper surface run: `peek` out-of-bounds and refusal modes
CRIT-ASSIGN: SC-211f | H-HELPER | helper surface run: `agents` failure modes
CRIT-ASSIGN: SC-211g | H-HELPER | helper surface run: `focus` refusal modes
CRIT-ASSIGN: SC-211h | H-HELPER | helper surface run: `interrupt` refusal modes
CRIT-ASSIGN: SC-211i | H-HELPER | helper surface run: `spawn` non-name argument errors
CRIT-ASSIGN: SC-211j | H-HELPER | helper surface run: `retire` refusal modes
CRIT-ASSIGN: SC-211l | H-HELPER | helper surface run: `say` refusal/failure modes
CRIT-ASSIGN: SC-211n | H-HELPER | helper surface run: `events-tail` query surface
CRIT-ASSIGN: SC-211o | H-HELPER | helper surface run: Codex session identity is registered positively and slot-bound
CRIT-ASSIGN: SC-211p | H-HELPER | helper surface run: `_lib` name resolution grammar
CRIT-ASSIGN: SC-212c | H-HELPER | helper surface run: requests mine/inbox/all signature
CRIT-ASSIGN: SC-300c | F-CONFIG | config grammar probe: unknown sections and unconsumed keys are ignored, not errors
CRIT-ASSIGN: SC-307 | F-CONFIG | config grammar probe: malformed-line behavior
CRIT-ASSIGN: SC-400a | F-FORMAT | format round-trip probe: the bash-era session layout remains READABLE
CRIT-ASSIGN: SC-400b | F-FORMAT | format round-trip probe: the event store's written layout changes under DR-001
CRIT-ASSIGN: SC-400c | F-FORMAT | format round-trip probe: generated-logic helpers retire from the written layout at P2
CRIT-ASSIGN: SC-401a | F-FORMAT | format round-trip probe: the archive payload is the five-part set
CRIT-ASSIGN: SC-401b | F-FORMAT | format round-trip probe: an archive materializes ONE canonical event stream
CRIT-ASSIGN: SC-402 | F-FORMAT | format round-trip probe: working directories stay clean
CRIT-ASSIGN: SC-403 | F-FORMAT | format round-trip probe: record framing round-trips every field faithfully
CRIT-ASSIGN: SC-404 | F-CONFIG | config grammar probe: AE_HOME derivation arm — default roots + each override exception exercised
CRIT-ASSIGN: SC-500 | L-COMPACT | compact tree: compact stdout byte format
CRIT-ASSIGN: SC-501 | L-COMPACT | compact tree: compact stderr carries everything else
CRIT-ASSIGN: SC-502 | L-COMPACT | compact tree: `Recovery:` prints BEFORE the relaunch
CRIT-ASSIGN: SC-503a | L-COMPACT | compact tree: a typed `n` is an answer
CRIT-ASSIGN: SC-503b | L-COMPACT | compact tree: end-of-input is not an answer
CRIT-ASSIGN: SC-504b | L-COMPACT | compact tree: no altered SIGPIPE disposition leaks into the child
CRIT-ASSIGN: SC-505a | F-IDENTITY | multi-boundary identity arm: session-name grammar echo at a general boundary
CRIT-ASSIGN: SC-505b | F-IDENTITY | multi-boundary identity arm: agent-name grammar echo at spawn/roster boundaries
CRIT-ASSIGN: SC-506 | C | corrupt-session degradation arm: one broken fixture session, document still closes
CRIT-ASSIGN: SC-507a | L-COMPACT | compact tree: `archive preview` stdout is exactly the digest
CRIT-ASSIGN: SC-507b | B0 | fingerprint-barrier design: mutate each moving file between fingerprint A/render/fingerprint B via fault hook
CRIT-ASSIGN: SC-507c | L-COMPACT | compact tree: `archive preview` diagnostics go to stderr
CRIT-ASSIGN: SC-507d | L-COMPACT | compact tree: `archive preview` is read-only by construction
CRIT-ASSIGN: SC-508 | L-COMPACT | compact tree: residual undocumented exit codes
CRIT-ASSIGN: SC-509 | C | read-side fixture cluster: list --json versioned object schema against fixture sessions
CRIT-ASSIGN: SC-510a | C | read-side fixture cluster: event required keys
CRIT-ASSIGN: SC-510b | C | read-side fixture cluster: optional keys are omitted when empty
CRIT-ASSIGN: SC-510c | C | read-side fixture cluster: `ref` polysemy follows the action table
CRIT-ASSIGN: SC-510d | C | read-side fixture cluster: string values are JSON-escaped
CRIT-ASSIGN: SC-511a | C | read-side fixture cluster: messaging events carry optional routing-key fields
CRIT-ASSIGN: SC-511b | C | read-side fixture cluster: readers prefer slot+session over display name
CRIT-ASSIGN: SC-511c | B0 | frozen-consumer fixture design: add/remove/rename keys against extracted consumers, explicit expected compatibility outcomes
CRIT-ASSIGN: SC-512 | L-COMPACT | compact tree: compact stdout truth claim
CRIT-ASSIGN: SC-513a | C | read-side fixture cluster: `next` exits non-zero when nothing needs attention
CRIT-ASSIGN: SC-513b | C | read-side fixture cluster: `next` exits non-zero on an unknown argument
CRIT-ASSIGN: SC-513c | C | read-side fixture cluster: `next` is read-only by default
CRIT-ASSIGN: SC-514 | C | read-side fixture cluster: `doctor` exit contract
CRIT-ASSIGN: SC-515a | L-STOP | stop matrix tree: `stop all` folds per-target result records into its exit
CRIT-ASSIGN: SC-515b | L-STOP | stop matrix tree: result-wait timeout is not a failure
CRIT-ASSIGN: SC-515c | L-STOP | stop matrix tree: an unowned ae-tagged session is named, not stopped
CRIT-ASSIGN: SC-516 | L-END | end/archive tree: `end` fails non-zero when the archive cannot be written
CRIT-ASSIGN: SC-517a | L-COMPACT | compact tree: compact's exit status is the launch's
CRIT-ASSIGN: SC-517b | L-COMPACT | compact tree: terminal case: attach, exit on detach
CRIT-ASSIGN: SC-517c | L-COMPACT | compact tree: non-terminal case: launch failure reports as plain `ae <name>`
CRIT-ASSIGN: SC-600 | F-TMUX | tmux effects probe: user text reaching a tmux format string is literalized or option-routed
CRIT-ASSIGN: SC-601 | F-TMUX | tmux effects probe: send-keys never receives user text as key names
CRIT-ASSIGN: SC-602 | F-TMUX | @ae_slot stamp arm: read pane option after launch/refresh
CRIT-ASSIGN: SC-603 | F-TMUX | layout application arm: configured layout vs resulting pane arrangement
CRIT-ASSIGN: SC-604 | F-TMUX | window naming arm: capture titles after launch/spawn
CRIT-ASSIGN: SC-704a | F-ADAPTER | adapter fixture probe: injected ae context never replaces a vendor's own agent prompt
CRIT-ASSIGN: SC-704b | F-ADAPTER | adapter fixture probe: capture binds only a positively-owned signal
CRIT-ASSIGN: SC-704c | F-ADAPTER | adapter fixture probe: resume requires exact ownership
CRIT-ASSIGN: SC-704d | F-ADAPTER | adapter fixture probe: heuristic fallbacks retire
CRIT-ASSIGN: SC-704e | F-ADAPTER | adapter fixture probe: rerun truth is explicit
CRIT-ASSIGN: SC-705 | F-ADAPTER | adapter fixture probe: executable-identification arm — env-prefixed and suffixed commands classified by real binary
CRIT-ASSIGN: SC-706 | F-ADAPTER | adapter fixture probe: a fact built upstream is transported, never re-parsed
CRIT-ASSIGN: SC-800 | L-END | end/archive tree: archive publication claims by `mkdir`
CRIT-ASSIGN: SC-801 | L-END | end/archive tree: staging is private by construction
CRIT-ASSIGN: SC-802 | L-END | end/archive tree: the final archive appears complete or not at all
CRIT-ASSIGN: SC-803 | L-END | end/archive tree: a standing claim is refused and named, never cleaned
CRIT-ASSIGN: SC-804a | L-PURGE | purge/validator tree: validator: exact path whitelist
CRIT-ASSIGN: SC-804b | L-PURGE | purge/validator tree: validator: no symlink or special file
CRIT-ASSIGN: SC-804c | L-PURGE | purge/validator tree: validator: directories 0700
CRIT-ASSIGN: SC-804d | L-PURGE | purge/validator tree: validator: no executable bit for user, group, OR other
CRIT-ASSIGN: SC-804e | L-PURGE | purge/validator tree: validator: `meta` and `digest.md` must agree
CRIT-ASSIGN: SC-804f | L-PURGE | purge/validator tree: validator: files 0600
CRIT-ASSIGN: SC-805 | L-PURGE | purge/validator tree: an archive is inert data
CRIT-ASSIGN: SC-806a | L-END | end/archive tree: archive identity is the session UUID, never the mutable name
CRIT-ASSIGN: SC-806b | L-END | end/archive tree: canonical lowercase key; legacy uppercase normalized
CRIT-ASSIGN: SC-807 | L-END | end/archive tree: the lifecycle lock is released before the relaunch
CRIT-ASSIGN: SC-808 | L-END | end/archive tree: the child re-proves the exact parent archive before publishing its state
CRIT-ASSIGN: SC-809 | L-FROM | from/lineage tree: lineage never inferred from a name
CRIT-ASSIGN: SC-810a | L-PURGE | purge/validator tree: `--purge-history` writes no archive
CRIT-ASSIGN: SC-810b | L-PURGE | purge/validator tree: `--purge-history` deletes any existing archive for the source UUID
CRIT-ASSIGN: SC-811a | L-END | end/archive tree: `launch.<slot>.sh` re-run: first run creates, later runs resume
CRIT-ASSIGN: SC-811b | L-END | end/archive tree: ae clears the marker whenever it rewrites the script
CRIT-ASSIGN: SC-812 | L-END | end/archive tree: the resume decision happens BEFORE exec
CRIT-ASSIGN: SC-813 | F-IDENTITY | multi-boundary identity arm: launch entry + default-name + rename target + transfer both directions, one hostile name each
CRIT-ASSIGN: SC-814 | L-RENTRANS | rename/transfer tree: endpoint name validation before any side effect
CRIT-ASSIGN: SC-815a | L-STOP | stop matrix tree: fleet acts on the confirmed set only
CRIT-ASSIGN: SC-815b | L-STOP | stop matrix tree: fleet entries carry session identity, name-reuse leaves newcomer
CRIT-ASSIGN: SC-815c | L-STOP | stop matrix tree: concurrent-fleet arm — two ops, each consumes only its own results
CRIT-ASSIGN: SC-815d | L-STOP | stop matrix tree: op-id representation in fleet events
CRIT-ASSIGN: SC-816 | L-END | end/archive tree: an unverifiable session is still a target
CRIT-ASSIGN: SC-817 | L-END | end/archive tree: end's transaction order: stop, git-outcome-fixed, capture, cleanup
CRIT-ASSIGN: SC-818b | L-PURGE | purge/validator tree: purge acquires the same `.publishing.<uuid>` claim
CRIT-ASSIGN: SC-818c | L-PURGE | purge/validator tree: purge validates the tree as an ae archive before deleting
CRIT-ASSIGN: SC-818d | L-PURGE | purge/validator tree: purge requires a NONEMPTY exact source-identity match
CRIT-ASSIGN: SC-818e | L-PURGE | purge/validator tree: purge refuses to delete a parent a live `--from` lineage points at
CRIT-ASSIGN: SC-819 | L-PURGE | purge/validator tree: an unidentifiable session is refused BEFORE anything is stopped
CRIT-ASSIGN: SC-820a | L-END | end/archive tree: confirmed-plan freeze + re-proof under lock, mismatch refuses
CRIT-ASSIGN: SC-821a | L-END | end/archive tree: end-all acts on the confirmed target set only
CRIT-ASSIGN: SC-821b | L-END | end/archive tree: prompt-ran is its own fact, empty set ends nothing
CRIT-ASSIGN: SC-822 | L-FROM | from/lineage tree: `--from` is valid only for a session that does not exist in any form
CRIT-ASSIGN: SC-823 | L-FROM | from/lineage tree: the parent is proved before anything is created
CRIT-ASSIGN: SC-824a | L-FROM | from/lineage tree: proof facts are recorded as proved, never re-read
CRIT-ASSIGN: SC-824b | L-FROM | from/lineage tree: an archive mid-publication or mid-purge is refused outright
CRIT-ASSIGN: SC-825a | L-FROM | from/lineage tree: the child records lineage durably
CRIT-ASSIGN: SC-825b | L-FROM | from/lineage tree: the parent path is derived, never stored
CRIT-ASSIGN: SC-825c | L-FROM | from/lineage tree: a deleted parent warns and continues on resume
CRIT-ASSIGN: SC-826 | L-FROM | from/lineage tree: a pre-id session gets one minted at end, recorded on both sides
CRIT-ASSIGN: SC-827 | L-COMPACT | compact tree: compact freezes ONE authorization tuple
CRIT-ASSIGN: SC-828 | L-COMPACT | compact tree: two revalidations, positioned by what they protect
CRIT-ASSIGN: SC-829a | L-COMPACT | compact tree: handover completion is two facts
CRIT-ASSIGN: SC-829b | L-COMPACT | compact tree: a re-run reuses the outstanding request and its baseline
CRIT-ASSIGN: SC-830 | L-END | end/archive tree: `--digest-only` is the one explicit degradation
CRIT-ASSIGN: SC-831 | L-END | end/archive tree: a timed-out handover stops nothing
CRIT-ASSIGN: SC-832a | L-RENTRANS | rename/transfer tree: rename's effect set
CRIT-ASSIGN: SC-833a | L-RENTRANS | rename/transfer tree: transfer moves a stopped session both directions
CRIT-ASSIGN: SC-834a | T-WD | watchdog branch harness: watchdog-driven _recover-pending invocation arm
CRIT-ASSIGN: SC-835a | L-STOP | stop matrix tree: stop addresses the recorded server and the exact session id
CRIT-ASSIGN: SC-835b | L-STOP | stop matrix tree: stop reports stopped only after verifying the session is gone
CRIT-ASSIGN: SC-835c | L-STOP | stop matrix tree: an unverifiable kill fails loudly and changes nothing
CRIT-ASSIGN: SC-835d | L-STOP | stop matrix tree: stop never deletes anything
CRIT-ASSIGN: SC-835e | L-STOP | stop matrix tree: self-stop confirms with the recoverability warning
CRIT-ASSIGN: SC-835f | L-STOP | stop matrix tree: `-y` skips the self-stop confirmation
CRIT-ASSIGN: SC-835g | L-STOP | stop matrix tree: self-stop executes via a short-lived out-of-pane supervisor
CRIT-ASSIGN: SC-835h | L-STOP | stop matrix tree: the self-stop outcome is a durable `stop-result` event
CRIT-ASSIGN: SC-836 | L-COMPACT | compact tree: `purge_agent_history` refuses compact unless `--keep-history`
CRIT-ASSIGN: SC-837 | L-COMPACT | compact tree: `compact -f` proceeds without asking
CRIT-ASSIGN: SC-838a | L-END | end/archive tree: end history policy precedence is CLI > session config > keep
CRIT-ASSIGN: SC-838b | L-END | end/archive tree: `end all` resolves and lists both decisions per session
CRIT-ASSIGN: SC-839a | L-STOP | stop matrix tree: `--self` waives exactly one check
CRIT-ASSIGN: SC-839b | L-STOP | stop matrix tree: `--pane` accepts only a shape-checked tmux pane id
CRIT-ASSIGN: SC-839c | L-STOP | stop matrix tree: the stop identity checks are C1–C5
CRIT-ASSIGN: SC-839d | L-STOP | stop matrix tree: a stop refusal names the failed check
CRIT-ASSIGN: SC-839e | L-STOP | stop matrix tree: the no-name form keeps tmux-controlled text out of shell programs
CRIT-ASSIGN: SC-900 | T-WD | watchdog branch harness: event-log container lifecycle
CRIT-ASSIGN: SC-901 | T-WD | watchdog branch harness: daemon topology
CRIT-ASSIGN: SC-913 | T-WD | watchdog branch harness: b3 fix-known-defect(#45) — every daemon nudge uses the ONE verified
CRIT-ASSIGN: SC-920 | T-WD | watchdog branch harness: b3 fix-known-defect(#51) — human-origin evidence inside quiet stabilization
CRIT-ASSIGN: SC-921 | T-WD | watchdog branch harness: b3 fix-known-defect(#73) — monitor panes are never agents and never enter
CRIT-ASSIGN: SC-926 | T-WD | watchdog branch harness: b3 fix-known-defect(#88-A) — control success only when durable intent and
CRIT-ASSIGN: SC-927 | T-WD | watchdog branch harness: b3 fix-known-defect(#88-B) — status is read-only; cleanup belongs to an
CRIT-ASSIGN: SC-928 | T-WD | watchdog branch harness: b3 fix-known-defect(#88-C) — an event-append error is surfaced and
CRIT-ASSIGN: SC-929 | T-WD | watchdog branch harness: b4 DR-002 — the restart outcome (gate ruling, testable): after a
CRIT-ASSIGN: SC-939a | T-CTRL | daemon control harness: b1 — sweep delivery is at-least-once: event-write failure after paste may
CRIT-ASSIGN: SC-941 | T-CTRL | daemon control harness: b2 — outbound include allow-list default; exclude applies after include
CRIT-ASSIGN: SC-942 | T-CTRL | daemon control harness: b2 — `chat` action gives the two-way loop; include-without-chat disables
CRIT-ASSIGN: SC-943 | T-AUTH | fake-updates auth harness: b1 — inbound exists only with nonempty `allowed_user_ids`; empty =
CRIT-ASSIGN: SC-944a | T-AUTH | fake-updates auth harness: b1 — inbound trust predicate: numeric allowlisted `from.id`; failure
CRIT-ASSIGN: SC-944b | T-AUTH | fake-updates auth harness: b1 — inbound trust predicate: exact configured `chat.id`; failure
CRIT-ASSIGN: SC-944c | T-AUTH | fake-updates auth harness: b1 — inbound trust predicate: private chat only; failure silently drops
CRIT-ASSIGN: SC-945 | T-AUTH | fake-updates auth harness: b2 — routing precedence: command > reply > compact > override/steward
CRIT-ASSIGN: SC-946 | T-AUTH | fake-updates auth harness: b1 — every inbound route passes the same session/agent revalidation
CRIT-ASSIGN: SC-947 | T-AUTH | fake-updates auth harness: b1 — only running sessions are addressable
CRIT-ASSIGN: SC-948 | T-AUTH | fake-updates auth harness: b2 — session resolves by exact name or unique session_id prefix
CRIT-ASSIGN: SC-949 | T-AUTH | fake-updates auth harness: b1 — agents resolve only within that session; pane-id, cross-session, and
CRIT-ASSIGN: SC-950 | T-AUTH | fake-updates auth harness: b2 — sender identity is `telegram:<id>`; replies route back outbound
CRIT-ASSIGN: SC-951 | T-AUTH | fake-updates auth harness: b1 — inbound update offset persists BEFORE dispatch: at-most-once side
CRIT-ASSIGN: SC-952 | T-AUTH | fake-updates auth harness: b2 — command-menu registration is best-effort (log and ignore)
CRIT-ASSIGN: SC-953 | T-STORE | store/handoff harness: b2 — start is idempotent
CRIT-ASSIGN: SC-954 | T-STORE | store/handoff harness: b2 — stop succeeds when already stopped
CRIT-ASSIGN: SC-955 | T-STORE | store/handoff harness: b2 — status reports persisted intent, runtime, deps, token validity
CRIT-ASSIGN: SC-956 | T-STORE | store/handoff harness: b1 — autostart failure warns one line and never blocks session launch
CRIT-ASSIGN: SC-957 | T-STORE | store/handoff harness: b1 — supervision honors durable disabled state; can never revive after an
CRIT-ASSIGN: SC-958 | T-STORE | store/handoff harness: b4 DR-003 — outbound delivery is at-least-once: cursor persistence is part
CRIT-ASSIGN: SC-959 | T-STORE | store/handoff harness: first-seen session starts at EOF (outbound cursor)
CRIT-ASSIGN: SC-960 | T-AUTH | fake-updates auth harness: b1 — the persisted getUpdates offset prevents inbound redispatch on
CRIT-ASSIGN: SC-961 | T-AUTH | fake-updates auth harness: b1 — token file is owner-only 0600; wrong perms refuse start with a
CRIT-ASSIGN: SC-962 | T-AUTH | fake-updates auth harness: b1 — the token never enters argv; logs redact it
CRIT-ASSIGN: SC-963 | T-STORE | store/handoff harness: b3 fix-known-defect(#83) — explicit start preserves exactly-one-sender:
CRIT-ASSIGN: SC-964 | T-STORE | store/handoff harness: b3 fix-known-defect(#84) — takeover is serialized and proves every
CRIT-ASSIGN: SC-965 | T-STORE | store/handoff harness: b3 fix-known-defect(#85) — destructive tmux targets resolve exact
CRIT-ASSIGN: SC-966 | T-STORE | store/handoff harness: b3 fix-known-defect(#86-E) — `/use clear` succeeds only after durable
CRIT-ASSIGN: SC-967 | T-STORE | store/handoff harness: b3 fix-known-defect(#87) — one effective-config authority for every
CRIT-ASSIGN: SC-968 | T-STORE | store/handoff harness: b3 fix-known-defect(#88-G) — lifecycle ownership acquired before any
CRIT-ASSIGN: SC-969 | T-STORE | store/handoff harness: b3 fix-known-defect(#87-H) — setup publishes token/config with atomic
CRIT-ASSIGN: SC-970 | T-STORE | store/handoff harness: b2 — setup persists enabled, token_file, chat_id, seeded allowlist (byte
CRIT-ASSIGN: SC-971 | T-STORE | store/handoff harness: b2 — start persists `enabled=true`; stop persists `enabled=false`
CRIT-ASSIGN: SC-972 | H-DELIVERY | delivery rig: external actor prefix grammar arm
CRIT-ASSIGN: SC-973a | H-DELIVERY | delivery rig: event-only sink literal-target arm (telegram:/discord:/ae:compact:)
CRIT-ASSIGN: SC-973b | H-DELIVERY | delivery rig: unknown external prefix loud-refusal arm
CRIT-ASSIGN: SC-974a | H-DELIVERY | delivery rig: AE_SENDER_OVERRIDE actor arm (send/ask/review)
CRIT-ASSIGN: SC-974b | H-DELIVERY | delivery rig: reply --as display-only arm
CRIT-ASSIGN: SC-975a | T-STORE | store/handoff harness: b1 — bridge readers tolerate a missing event file
CRIT-ASSIGN: SC-975b | T-STORE | store/handoff harness: b1 — malformed/unterminated trailing data is buffered until a complete
CRIT-ASSIGN: SC-976a | T-STORE | store/handoff harness: b4 DR-001 — the reader cursor is generation-aware (generation + offset
CRIT-ASSIGN: SC-976b | T-STORE | store/handoff harness: b2 — event logs are tailed/back-scanned bounded, never whole-loaded
CRIT-ASSIGN: SC-977 | T-STORE | store/handoff harness: b1 — bridges bind the stable session_id across resume/rename/transfer
CRIT-ASSIGN: SC-978a | T-STORE | store/handoff harness: b2 — bridges ignore unknown fields/actions
CRIT-ASSIGN: SC-978b | T-STORE | store/handoff harness: b2 — renames/removals/semantic changes of existing fields are BREAKING
CRIT-ASSIGN: SC-979a | T-STORE | store/handoff harness: b1 — telegram sends use plain-text paths (no parse-mode injection)
CRIT-ASSIGN: SC-979b | T-STORE | store/handoff harness: b1 — jq programs stay fixed strings; user data enters via stdin only
CRIT-ASSIGN: SC-1005 | F-INSTALL | installer/doctor probe: installer failure modes
CRIT-ASSIGN: SC-1006 | F-INSTALL | installer/doctor probe: the installed artifact is versioned and atomic
CRIT-ASSIGN: SC-1101a | F-PLATFORM | flock-absent PATH shim arm: core commands degrade loudly, never command-not-found death
CRIT-ASSIGN: SC-1102 | F-PLATFORM | platform probe: session/archive UUIDs are canonical lowercase
CRIT-ASSIGN: SC-1103 | F-PLATFORM | platform probe: socket-path limit arm — over-limit path fails loud pre-creation
CRIT-ASSIGN: SC-1200 | F-IDENTITY | identity boundary probe: agent names are allowlisted, not screened
CRIT-ASSIGN: SC-1201 | F-IDENTITY | identity boundary probe: the spawn boundary treats a peer name as hostile
CRIT-ASSIGN: SC-1202 | F-IDENTITY | identity boundary probe: the operator roster boundary fails the launch before product mutation
CRIT-ASSIGN: SC-1203 | F-IDENTITY | identity boundary probe: enforcement follows provenance, not the variable
CRIT-ASSIGN: SC-1204 | F-IDENTITY | identity boundary probe: the interpolation boundary re-validates and fails quiet
CRIT-ASSIGN: SC-1205a | F-IDENTITY | identity boundary probe: every derived name is grammar-valid and unique after derivation
CRIT-ASSIGN: SC-1205b | F-IDENTITY | identity boundary probe: dedup shape: truncate to fit, suffix from `-2`
CRIT-ASSIGN: SC-1206 | F-IDENTITY | identity boundary probe: a leading underscore is a legal alias but never an agent name
CRIT-ASSIGN: SC-1207a | F-IDENTITY | identity boundary probe: prompt identity facets are unambiguous
CRIT-ASSIGN: SC-1207b | F-IDENTITY | identity boundary probe: meta serializes agents as `alias:name:provider-session-id`
CRIT-ASSIGN: SC-1208 | B0 | argv/context capture design: constructed injection boundary vs delivered user-input artifact
CRIT-ASSIGN: SC-1209 | F-IDENTITY | identity boundary probe: envelope authority arm — nested/pasted envelopes treated as data, unenveloped input as human
CRIT-ASSIGN: SC-1301 | H-HELPER | meta-writer fault arm: fault hooks on each of the three writers, reader observes complete-or-old only
CRIT-ASSIGN: SC-1302 | L-RENTRANS | cross-lifecycle concurrency arm: concurrent stop/rename/transfer on one name, serialization observed
CRIT-ASSIGN: SC-1303 | L-RENTRANS | rename/transfer tree: rename: what a concurrent observer may see mid-operation
CRIT-ASSIGN: SC-1304a | L-RENTRANS | rename/transfer tree: push mid-op arm — source present post-stop, no destination write yet
CRIT-ASSIGN: SC-1304b | L-RENTRANS | rename/transfer tree: push destination-partial arm — crash-cut leaves partial remote state
CRIT-ASSIGN: SC-1304c | L-RENTRANS | rename/transfer tree: pull mid-op arm — remote source present post-stop
CRIT-ASSIGN: SC-1304d | L-RENTRANS | rename/transfer tree: pull destination-partial arm — crash-cut leaves partial local state
CRIT-ASSIGN: SC-1305 | L-COMPACT | compact tree: mid-operation observability cuts
CRIT-ASSIGN: SC-1306a | C | read-side fixture cluster: `list` snapshot cut under concurrent writes
CRIT-ASSIGN: SC-1306b | C | read-side fixture cluster: `status` snapshot cut
CRIT-ASSIGN: SC-1306c | C | read-side fixture cluster: `next` snapshot cut
CRIT-ASSIGN: SC-1306d | C | read-side fixture cluster: `requests` snapshot cut
CRIT-ASSIGN: SC-1306e | C | read-side fixture cluster: `events-tail` snapshot cut vs concurrent append/trim
CRIT-ASSIGN: SC-1409a | H-ENV | env sweep: non-numeric values in numeric watchdog/loop tunables
CRIT-ASSIGN: SC-1409b | H-ENV | env sweep: malformed telegram include/exclude lists
CRIT-ASSIGN: SC-1409c | H-ENV | env sweep: malformed `allowed_user_ids`
CRIT-ASSIGN: SC-1410a | H-ENV | env sweep: `AE_HOME`
CRIT-ASSIGN: SC-1410b | H-ENV | env sweep: `CONFIG_FILE`/`AE_LOCAL_CONFIG` precedence
CRIT-ASSIGN: SC-1410c | H-ENV | env sweep: `AE_TMUX_SERVER`
CRIT-ASSIGN: SC-1410d | H-ENV | env sweep: `AE_NO_AUTOSTART`
CRIT-ASSIGN: SC-1410e | H-ENV | env sweep: `AE_END_SERVER`
CRIT-ASSIGN: SC-1410f | H-ENV | env sweep: `AE_HUB_DIR`
CRIT-ASSIGN: SC-1410g | H-ENV | env sweep: `AE_STEWARD_DIR`
CRIT-ASSIGN: SC-1410h | H-ENV | env sweep: `AE_EVENTS_KEEP`
CRIT-ASSIGN: SC-1410i | H-ENV | env sweep: `AE_SEND_DEFER_SEC`
CRIT-ASSIGN: SC-1410j | H-ENV | env sweep: `AE_ATTN_REQUEST_SECS`
CRIT-ASSIGN: SC-1410k | H-ENV | env sweep: `AE_LIST_ACTIVE_SECS`
CRIT-ASSIGN: SC-1410l | H-ENV | env sweep: `AE_COMPACT_HANDOVER_SECS`
CRIT-ASSIGN: SC-1411a | H-ENV | env sweep: `AE_CODEX_LAUNCH_ID`/`AE_CODEX_SLOT`
CRIT-ASSIGN: SC-1411b | H-ENV | env sweep: `AE_GEMINI_LAUNCH_ID`/`AE_GEMINI_SLOT`
CRIT-ASSIGN: SC-1411c | H-ENV | env sweep: `AE_OPENCODE_LAUNCH_ID`
CRIT-ASSIGN: SC-1412a | H-ENV | env sweep: `AE_RESOLVED_*`
CRIT-ASSIGN: SC-1412b | H-ENV | env sweep: `AE_SESSION`
CRIT-ASSIGN: SC-1412c | H-ENV | env sweep: `AE_META`
CRIT-ASSIGN: SC-1412d | H-ENV | env sweep: `AE_DIR`
CRIT-ASSIGN: SC-1412e | H-ENV | env sweep: `AE_MODE`
CRIT-ASSIGN: SC-1412f | H-ENV | env sweep: `AE_ORIGIN`
CRIT-ASSIGN: SC-1412g | H-ENV | env sweep: `AE_PATH`/`AE_PATH_BIN`
CRIT-ASSIGN: SC-509b | C | read-side fixture cluster: degraded:true additive on read/parse loss, absent on normal entries
CRIT-ASSIGN: SC-518 | C | read-side fixture cluster: full mirror-match closure arm (ref + actor/target both ways, mixed matches nothing)
CRIT-ASSIGN: SC-519 | C | read-side fixture cluster: absent vs empty vs unreadable event-log arms
CRIT-ASSIGN: SC-520 | C | read-side fixture cluster: malformed-complete-line skip observable arm
CRIT-ASSIGN: SC-521 | C | read-side fixture cluster: filter intersection arms (stopped+needs-attn, stopped+active, all+either)
CRIT-ASSIGN: SC-522 | C | read-side fixture cluster: threshold equality-vs-past boundary arm
CRIT-ASSIGN: SC-523a | C | read-side fixture cluster: unanswered-threshold default confirmation (1800s)
CRIT-ASSIGN: SC-523b | C | read-side fixture cluster: activity-window default confirmation (300s)
CRIT-ASSIGN: SC-524 | C | read-side fixture cluster: future-timestamp counts-active arm
CRIT-ASSIGN: SC-405a | C | read-side fixture cluster: meta first-equals/single-line parse arm
CRIT-ASSIGN: SC-405b | C | read-side fixture cluster: session-context key arms (mode/origin/work_dir/goal)
CRIT-ASSIGN: SC-405c | C | read-side fixture cluster: roster key arms (agent.slot, agent_bin.slot)
CRIT-ASSIGN: SC-405d | C | read-side fixture cluster: unknown-meta-key probe arm
CRIT-ASSIGN: SC-405e | C | read-side fixture cluster: malformed/duplicate-meta-key probe arm
CRIT-ASSIGN: SC-405f | C | read-side fixture cluster: goal_set_epoch derived-from-latest-goal-event arm
CRIT-ASSIGN: SC-405g | C | read-side fixture cluster: branch resolution subarms — running tmux @ae_branch_name primary; git fallback when absent/stopped
CRIT-ASSIGN: SC-980 | T-WD | watchdog branch harness: incumbent alert action/summary byte capture (legacy adapter IS only, never SHOULD)
CRIT-ASSIGN: SC-405i | C | read-side fixture cluster: missing-meta-degrades arm (present dir, no meta; distinct from SC-519 quiet)
CRIT-ASSIGN: SC-405j | C | read-side fixture cluster: stale-session routed event stays unassociated (renamed-session fixture, loud false-negative capture)
CRIT-ASSIGN: SC-405k | C | read-side fixture cluster: runtime-only slot never invents an agent (roster-vs-runtime divergence fixture)

