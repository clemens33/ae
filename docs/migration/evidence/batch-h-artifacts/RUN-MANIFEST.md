# Batch H — run manifest

PRE-REGISTRATION. Every arm script is committed and hash-registered HERE BEFORE its first
run. Any amendment to a script is durable and REOPENS the arms that ran under the previous
hash. Adaptive capture — choosing what to record after seeing a reading — is therefore a
diff rather than a promise.

## Exposure record (required by seat ruling)

The author of the design and census (opus5:cexec) saw answer-labelled source outcomes in
design drafts v1-v3 before they were moved to the seat annex: `interrupt`'s silent
non-refusal path, `goal`'s arity guard, the `%*` branch's unconditional return,
`_register-sid`'s selection order, and `say`'s exact stdout. Recorded so a seat reading
these captures knows what the author knew. The arms are built from
`batch-h-input-list.md`, which is generated from the census by a columnar drop and gated
by a checker with per-arm neutral/mutated calibration.

## Scope

D14b is HELD and EXCLUDED pending the ownership-record split; no probe of it runs.
SC-1301 runs last, under the hook and barrier machinery its own section describes.

## Pre-registered scripts

| script | sha256 | registered (UTC) |
|---|---|---|
| `_harness/hlib.sh` | `0f838f7e89619358650f4ee99da31776f5351ef219c32f85c9b1546a0a8779e4` | 2026-08-20T21:51:16Z |
| `_harness/hfix.sh` | `51e401d354e20cd8332fb019d3a76122a669df482a87f9054cb68818d7a5c4b5` | 2026-08-20T21:51:16Z |
| `_harness/make-h-hook-patch.py` | `b9b62f8ddf2a1db0fbadbb45e2b42a51573cb3976bc66990dec021e48da485be` | 2026-08-21 (registered BEFORE its first run) |
| `_harness/arm-h1301.sh` | `880047d483fe32a5bf05fb4f71a08f4a6a2de4842e8e37b26a073694a4aa815c` | 2026-08-21 (registered BEFORE its first run) |
| `_harness/arm-h8.sh` | `a0382a9dc48123dd10a1034e85723fcbc3d06fb69d51f229331ebbe86061df71` | 2026-08-21 (registered BEFORE its first run) |
| `_harness/arm-h1h2.sh` | `4c2c8cb2d71d8153a9efe102572596546780b8381fed2885d30ed0f13d0da720` | 2026-08-21 (registered BEFORE its first run) |
| `_harness/arm-h7l.sh` | `dffe8d0a7a2101555a776887c66efa66ad7673994e2eb73f31163c344206da86` | 2026-08-21 (registered BEFORE its first run) |
| `_harness/pty-run.py` | `04bb88cc25a4312a76e6cde62563aa5c18966a1ac17d3e6c9b06f02d7842c5c3` | 2026-08-21 (registered BEFORE its first run) |
| `_harness/arm-h3.sh` | `453421717eaf23a711d303a65e63016e95ba1d4ea99c093e4ab0ede98d721ce1` | 2026-08-21 (registered BEFORE its first run) |
| `_harness/arm-h5.sh` | `0977a5fdf505de288518175c882fe78837249fe473fe41f8d12ed6eb8110e400` | 2026-08-21 (registered BEFORE its first run) |
| `_harness/arm-h4.sh` | `5944b88c92acd4752addffabe9705ab087ff6c430bed0ea1d0bf6d8040693009` | 2026-08-20T21:51:16Z |

### A3 — `_harness/derive-h4-record.py` registered after A-H4's run

- new sha256: `c7c96f2a2da3c67703b11a4fe6f8dd101b70146d2f7a7556fdb996b11f15f15f`
- what it is: a REPORTING script that reads A-H4's committed captures and emits
  `resolution-record.txt`. It records no new observation and cannot change one.
- why it is registered anyway: pre-registration is about the capture program, and a
  reporting script that runs after the captures exist is outside it — but it is committed
  and hashed here so a seat reading the record can regenerate it and get the same bytes.

### A5 — `_harness/finalize-h-arm.sh` registered after A-H4's run

- new sha256: `3e6bdba0ccc314ebeb2700900d1c17b956623b8e07db5f9032a5a573ae6898de`
- what it is: a post-capture indexer. It writes the content-bound case index (case dir ->
  ledger sha256, ledger lines, file count) and records no observation.

### A6 — constructed inputs in A-H5, declared before the run

`_register-sid` reads Codex session `.jsonl` files and the `launch_id`/`launch_time` meta
keys. There is no offline producer for either — this batch runs with no live models and no
network, and the fake agent is not a codex-kind tool, so ae never writes those keys.

The candidate files and meta lines are therefore written by the CONTROLLER, and each case
records the exact bytes it planted in `planted-inputs.txt` with their hashes. They are
INPUT DATA the surface reads, not helper bytes: every helper byte still comes from a real
frozen launch. Declared here rather than discovered later, because "producer-derived" is a
claim this arm cannot make about its candidates and should not appear to.

### A7 — `_harness/derive-h5-record.py` registered after A-H5's run

- new sha256: `f71e06aea76925ad10e036ad1830afb4a618324d10486e28be1e44602a7e09d4`
- post-capture reporting only; reads the committed captures and records no observation.

### A11 — A-H3's scope and its per-group sandboxes, declared before the run

- SC-211l (`say`) is NOT in A-H3. It runs under its own containment section, and putting
  it here would mean invoking a surface whose effect can leave the machine before the
  containment that governs it exists.
- Each helper group gets its OWN launched sandbox. A-H4 taught this the expensive way: a
  manipulation made for one case destroyed another case's precondition. Cases inside a
  group run in the listed order and some of them mutate — `spawn` adds an agent, `retire`
  removes one, `state`/`goal`/`memo` write — so the order is part of each group's fixture
  and is recorded in every ledger.
- Four cases need a controller manipulation the input class itself names: an unreadable
  `meta` (mode 000), an emptied `meta`, a malformed `config`, and a pane respawned as a
  plain shell. Each is applied immediately before its case and reverted immediately after
  where the group continues.

### A13 — `_harness/derive-h3-record.py` registered after A-H3's run

- new sha256: `91e8872e7876a1aa0be1344da184057aa8f7bf43f2210e19138cf883bc50eb15`
- post-capture reporting only; reads the committed captures and records no observation.

### A14 — SC-211l's containment, declared before the run

Layer 1 is STRUCTURAL and load-bearing: the bridge takes its root from `AE_HOME`
(telegram-daemon:10-11) and this fixture's `AE_HOME` is randomly named and created AFTER
the system-root census. Reach is inherited across fork — the live daemon forks a child per
poll cycle — so a child cannot reach what its parent cannot, and the census enumerates
long-lived ROOTS rather than trying to catch transient children.

Layer 2 is the census, corroborating. It does not match on argv, because a census whose own
command line contains its search string counts itself. It classifies by REACH (each
process's own `AE_HOME`) and excludes the arm's own processes by a token they carry, which
no foreign process can hold. Both directions are demonstrated per case: an in-range control
the census MUST report, and a token-carrying process it MUST exclude. A case whose census
cannot report its own control is INCONCLUSIVE, not contained.

Layer 3 is PATH-first refusing `curl`/`wget` stubs, and its scope is stated rather than
overstated: they contain only what the ARM spawns. An already-running bridge never
inherited this PATH and is NOT contained by them. The stub is fired deliberately in each
case and its log must carry the attempt — a recorder nobody has seen fire is not evidence
of silence.

`pty-run.py` exists because `say` branches on whether stdin is a terminal (ae:14473): a
redirected empty stdin and a real TTY are different inputs and only a pty presents the
second.

### A16 — `_harness/derive-h7l-record.py` registered after A-H7L's run

- new sha256: `42fba8e4e625e6c8b5633b0b096c6fbe3d56d39081b67983f56852e0fd0ca93e`
- post-capture reporting only.

### A17 — `_harness/derive-h1h2-record.py` registered after A-H1/A-H2's run

- new sha256: `bc2c105a87f01788d6c83ac66fb4b98c7e6d98c810f087fc9ca9bc4025933cfb`
- post-capture reporting only.

### A18 — A-H8's guards enumerated before it was built

The preventive rule applied for the first time. Between an invocation and the fact each
case is about there sit: an unconditional banner (ae:14889-14894), which a barrier must not
mistake for content; the file-existence wait (ae:14897-14899); the replay cut at
`tail -n 30` (ae:14902); and `helper_events_tail_format_event` (ae:14862-14884), which
RETURNS EARLY on an empty line and on a line not starting with `{`.

That last one is the guard that would have cost a run: a planted line failing the formatter
produces no output, which looks exactly like the replay cut dropping it. Every planted event
is well-formed JSON carrying the fields the formatter reads, except the case that is
deliberately about a partial line.

Termination is a controller action after a named barrier — this surface never exits on its
own, so a bound expiring here is an instrument artifact and is recorded as one, never as a
product rc.

### A20 — `_harness/derive-h8-record.py` registered after A-H8's run

- new sha256: `24206b9c24c8667269f42c3c595dbf7e2e58f86b89f85296f00f68c8fdc8c618`
- post-capture reporting only.

### A21 — SC-1301's instrumented copy, declared before any hooked capture

ONE hook-only patch over an exact 72c7293 copy, three hook points, one per writer, because
the three writers do not share a boundary:

- `AH_META_TEMP_COMPLETE` — `ae_meta_set`, between the temp write and the rename, the
  atomic writer's only window;
- `AH_SPAWN_BETWEEN_APPENDS` — `_cmd_spawn`, between two of its OWN appends, a real
  partial-generation window the frozen writer produces;
- `AH_CAPTURE_APPEND_DONE` — `start_capture_session_id`, after its single append. There is
  no mid-write window there, so this point exists only to sequence the controller and
  NEVER to claim a writer tear.

`_ae_hook` is guard-first: with `AE_HOOK` unset it returns 0 having done nothing. It rides
the `_lib` `declare -f` emission list beside `ae_meta_set`, because the helper-side writer
is emitted into every generated helper and would otherwise call a function it does not
have — the D02 discovery from batch C, applied before it cost a run.

The arm refuses to take a hooked capture until TWO things hold: the inactive hook is
byte-equivalent to the unmodified control on the same scenario, AND the comparator reports
a KNOWN difference (a copy with an altered version string) as different. A comparator that
has never reported red cannot certify equivalence.

## Frozen source

`72c7293`, verified by hash at every case open and recorded in each case's ledger.

## Amendments

### A1 — `_harness/hlib.sh`, before any arm completed

- previous sha256: `a655b4394a65bf7f429cd003fd5b6c4316bad876f42b62d4c2bd5ec1bbe4d320`
- new sha256: `40ace86d757f5e0f27734472a25d6f4658875140720b85499bce2b6f90f0cb5f`
- what changed: `canary()`'s three locals were declared in ONE `local` statement whose
  third value interpolated the first two. Bash expands every word of such a statement
  before any assignment takes effect, so `when` was still unset when `label` was built and
  the run aborted under `set -u`. Split into separate statements.
- why it is not adaptive capture: the arm never produced a reading. A-H4's first run died
  inside the PRE canary of its first case — the instrument refusing to start, which is the
  canary doing its job — and its partial output was deleted rather than published.
- arms reopened: A-H4 (it had not completed; nothing ran under the previous hash).
- note: this is the exact class AGENTS.md documents for `export HOME=... AE_HOME=...`. A
  documented hazard does not prevent itself.

### A2 — `_harness/hfix.sh` and `_harness/arm-h4.sh`, after a run whose fixture was wrong

- `hfix.sh`: `73c287312f99824b00450e24da687bb756e5c3a376fcd5d0192ed5dd05a3feda` -> `1227077c8691e481c0f5074b847e113acf4a963787ec875e238c7fe7dd2e61a0`
- `arm-h4.sh`: `6f6208234ea3d46c11c4d9493554d05008650f1b3752f7361515f5299d49b91b` -> `5944b88c92acd4752addffabe9705ab087ff6c430bed0ea1d0bf6d8040693009`
- what changed: the H5 fixture did not carry the collisions three of its cases name, and
  the run was discarded rather than published. Specifically:
  1. `workers = cx:lead` was RENAMED to `cx:lead-2` by the frozen launcher's own worker
     dedup, so the bare-name-AMBIGUOUS case had no collision — it measured a unique name.
     The collision is now created through the real `spawn` helper, the product path that
     can make one.
  2. the second session reused the first's config and came up with an identical roster, so
     the cross-session case that names "session exists, agent present" resolved nothing and
     measured the same thing as the "agent absent" case beside it. The second session now
     gets its own roster via `h_reconfig`.
  3. the dead-pane manipulation killed `zz:only` — which is also the fixture for
     "alias-only unique". One case's manipulation destroyed another case's precondition. A
     fourth alias (`qq:spare`) exists solely to be killed.
- why it is not adaptive capture: nothing from the first run is published. The defect is in
  the FIXTURE's ability to present the input class, not in what any reading said, and it was
  found by reading the roster the arm itself captured — which is why the arm now writes
  `fixture-validity.txt` naming what the roster and the server actually carry, before any
  case runs.
- arms reopened: A-H4, entirely.

### A4 — `_harness/hlib.sh`, event name made batch-distinct

- `40ace86d757f5e0f27734472a25d6f4658875140720b85499bce2b6f90f0cb5f` -> `b2cf84dd1990c01fdbfc9512c00834d10543d4745ed6a2dc2ad6c134ea27455d`
- what changed: `surface_state()` emitted `source-state-captured`, the same event batch C
  uses to declare it captured `meta-state.txt`. The shared name made batch C's schema rule
  apply to batch H's cases, which capture `surface-state.txt` instead — so the gate
  demanded an artifact this batch does not produce. Renamed to `surface-state-captured`,
  and the schema kinds now fork by batch layout rather than one batch's rule being
  weakened to fit the other's captures.
- why it is not adaptive capture: the event name is a LEDGER LABEL, not a reading. No
  capture changes. A-H4 is re-run anyway so its committed ledgers match the script that is
  registered against them — a ledger naming an event its script no longer emits is exactly
  the drift the hash registration exists to prevent.
- arms reopened: A-H4.

### A8 — `_harness/arm-h5.sh`, the cwd cases could not consult their own fact

- `cc6cfbd8abe690024bae907a7c87f28ce12c97a61212119d000cd461786735fb` -> `6f1d7e8123717aba8d66867080d7d6282251830a41717c6a8fa375aa611c2380`
- what changed: `h5-c09` and `h5-c10` planted a candidate whose recorded cwd matched or
  differed, but the token pass selected it first, so `best_id` was non-empty and the cwd
  fallback at ae:14794-14812 — guarded by `if [ -z "$best_id" ]` at ae:14793 — never ran.
  Both cases reported identically because the cwd fact was NEVER CONSULTED, not because it
  does not matter. They now set a launch-id token that no candidate carries, so the token
  pass selects nothing and the fallback is what decides.
- who found it: the worker flagged the identical readings as a capture and named the
  fixture as one of two hypotheses rather than shipping the pair; the seat confirmed the
  guard in frozen source and ruled the rebuild.
- the mtime pair is NOT rebuilt: `h5-c07` and `h5-c08` already discriminate, and c07
  reading like a single-candidate case follows from `-gt` at ae:14784 — equal mtimes means
  first-wins, exactly as a lone candidate does. Same output, different mechanism; the
  discriminating comparison is c07 against c08.
- arms reopened: A-H5, whole. The other twelve cases are re-run under the amended script so
  every committed ledger matches the hash registered against it, and their readings are
  compared with the previous run rather than assumed to reproduce.

### A9 — `_harness/arm-h5.sh`, the invocation stood in the wrong directory

- `6f1d7e8123717aba8d66867080d7d6282251830a41717c6a8fa375aa611c2380` -> `3055a8c98121a2c940e103a9e4d854d945d7375c08e59fd6df8fa70519984889`
- what changed: with the fallback now reachable (A8), both cwd cases reported rc 1 and no
  artifact — still identical, in the other direction. The helper takes `TARGET_CWD` from
  `$PWD` (ae:14753) and the fallback compares a candidate's recorded cwd against it
  (ae:14807); the arm invoked it from wherever the controller happened to stand, so the
  match case could not match BY CONSTRUCTION. Invocations now run from the session's work
  dir, which is where an agent's pane stands, and the cwd is recorded in each ledger.
- the same pair has now failed to consult its own fact twice, for two different reasons.
  The first was caught by a seat reading the guard; the second by the rebuild reporting
  identical-again readings rather than the opposed pair the fix predicted — which is why a
  rebuilt case is re-read rather than assumed fixed.
- arms reopened: A-H5, whole.

### A10 — `_harness/arm-h5.sh`, two pairs instead of one rebuilt pair (seat disposition)

- `3055a8c98121a2c940e103a9e4d854d945d7375c08e59fd6df8fa70519984889` -> `8796a1023a4dd211732cb8e0e95e183543908861defa1552d1de2622dba00921`
- what changed: both seats reached the c09/c10 non-discrimination independently and the
  reviewing seat's disposition was adopted over the lead's own. Rather than REBUILDING the
  cwd cases, the original token-carrying pair is RETAINED as TOKEN-PRECEDENCE CONTROLS —
  they record that while the token path selects, the cwd fallback at ae:14794-14812 is
  never reached, which is what ae:14793 encodes and which nothing else in the batch
  captures — and a new pair (`h5-c15`, `h5-c16`) carries a token no candidate holds so the
  fallback IS reached and cwd decides. Every other byte and time is held constant across
  each pair.
- why it is better than the rebuild it replaces: rebuilding would have deleted evidence for
  a true fact in order to test a different one.

### A9 note — the cwd fix changed a SECOND case's reading, and the reason is mechanical

`h5-c04-token-mismatch` read rc 1 with no artifact before A9 and rc 0 with an artifact
after it. Nothing about that case's own fixture changed. Its token matches no candidate, so
the token pass selects nothing and the fallback runs; before A9 the invocation stood in the
controller's directory so the fallback's cwd compare (ae:14807) could not match, and after
A9 it stands in the session work dir, where the planted candidate's recorded cwd does
match. The reading is a joint property of the case's fixture and the invocation's cwd, and
it moved when the cwd did. Recorded rather than smoothed: it is the reason every case is
re-read after an amendment instead of only the ones the amendment names.

### A12 — the subshell broke the chronology record, in two arms

- `hlib.sh`: `b2cf84dd1990c01fdbfc9512c00834d10543d4745ed6a2dc2ad6c134ea27455d` -> `0f838f7e89619358650f4ee99da31776f5351ef219c32f85c9b1546a0a8779e4`
- `arm-h5.sh`: `8796a1023a4dd211732cb8e0e95e183543908861defa1552d1de2622dba00921` -> `0977a5fdf505de288518175c882fe78837249fe473fe41f8d12ed6eb8110e400`
- `arm-h3.sh`: `6c5c7fc06eadff0360cb7411a4e26bbf584d75df199d51e3612cf9cc06e7277b` -> `453421717eaf23a711d303a65e63016e95ba1d4ea99c093e4ab0ede98d721ce1`
- what changed: A9 changed the invocation directory with `( cd dir && measured ... )`.
  `LED_SEQ` is a shell variable; the subshell advanced it and the parent resumed at its own
  stale value, so every ledger written that way REPEATS identities it has already used. All
  sixteen A-H5 ledgers and all seventy-two A-H3 ledgers are chronologically impossible
  records with correct checksums. `run_in` changes directory without a subshell.
- **the ledgers are NOT renumbered.** A chronology record cannot be repaired by editing it —
  renumbering manufactures the ordering the file exists to attest. Both arms are re-run
  under the new hashes and the old captures are deleted, not corrected.
- `arm-h3.sh` had the same defect and had not yet been reported: found by applying the
  seat's finding to the other arm that shares the pattern rather than waiting for it to be
  raised again.
- ALSO in `arm-h5.sh`: `h5-c15`/`h5-c16` varied the candidate's marker field as well as its
  cwd — one variable violated inside the pair built to demonstrate one-variable discipline.
  The marker is identical now.
- the gate could not see any of this: it checks presence and hash, and a file that repeats
  an identity passes both. `gate/check-ledger-chronology.py` now checks uniqueness and
  monotonicity, with a neutral leg reporting caught=NO and four mutated legs reporting YES,
  including the exact subshell shape. Batch C's 177 ledgers pass it unchanged.
- arms reopened: A-H5 and A-H3, both whole.

### A15 — the census cannot classify most processes on this platform, and now says so

- `03b0d8b6a964c8f570595a0b8a76f68450b572aa25e3192d70a434c8f305b825` -> `dffe8d0a7a2101555a776887c66efa66ad7673994e2eb73f31163c344206da86`
- what the first run showed: every case returned INCONCLUSIVE because the census did not
  report its own in-range control. Two causes, both measured:
  1. `bash -c 'sleep 25'` EXECS `sleep`, and macOS then exposes no environment for it — the
     control was unreportable for a reason unrelated to the census. A trailing `:` prevents
     the exec optimisation.
  2. macOS exposes a process's environment to `ps e` for only a SUBSET of even one's own
     processes: 1 of 40 sampled here. A process whose environment cannot be read CANNOT BE
     CLASSIFIED BY REACH.
- what changed: the census reports three classes — IN-RANGE, out-of-range, and
  UNKNOWN-REACH — with counts, and an unreadable process is UNKNOWN, never quietly counted
  as out of range. The containment claim is bounded accordingly: layer 2 cannot enumerate
  all watchers on this platform, so it corroborates rather than carries, layer 1
  (a randomly named `AE_HOME` created after the system census, with reach inherited across
  fork) carries the claim, and layer 3 covers only what the arm spawns.
- the guard worked: every case aborted rather than reporting a contained run on a census
  that could not see its own control.
- arms reopened: A-H7L, which had produced no reading.

### A19 — the replay cohorts held one unterminated line, not 29/30/31 events

- `288e0013679c4cc6c74c0e33f3ba422b775c7d9a2d139b3d98f60333b047a58a` -> `a0382a9dc48123dd10a1034e85723fcbc3d06fb69d51f229331ebbe86061df71`
- what changed: `ev_line` printed its JSON WITHOUT a trailing newline, so every planted
  event concatenated onto the previous one and each cohort file held ONE unterminated line
  regardless of how many events were written (`wc -l` = 0 on a 29-event cohort).
  `tail -n 30` had nothing to cut and the formatter rendered the first field it found, so
  all three cohorts reported one rendered line and no barrier was ever seen.
- the shape is the one this arm's guard enumeration was written to prevent, arriving from
  the other side: the fact under test was foreclosed by the INPUT CONSTRUCTION rather than
  by a product guard. Enumerating the guards between the manipulation and the observation
  does not cover whether the manipulation produced the input it claims — that is its own
  check, and the arm now records `planted_lines` beside `planted_events` so the two can
  disagree visibly.
- arms reopened: A-H8, whole. Nothing from the first run is published.

### A22 — SC-1301's equivalence comparison, twice wrong before it could certify anything

- `7183ec11273bc311c145cb32ebeacc51e5b806b4d01f3ad18dccf74cddf273be` -> `7794562234bac7a4e1027c3ecea61935d7eea0d84c53c42ff14515a5e7882d1d`
- what the first run showed: the arm ABORTED before any hooked capture, which is the guard
  working. Two causes:
  1. each equivalence run named its session after its own tag, so every path differed and
     the comparison reported the LABELLING rather than the binary. Same session name in
     every run now.
  2. equivalence as byte-identity is IMPOSSIBLE with this patch: the hooked copy emits
     `_ae_hook` into every generated `_lib`, so that file legitimately differs. Equivalence
     is now the ENUMERATED form — identical everywhere except files whose ONLY difference is
     the hook's own bytes — and the arm PROVES the difference set is exactly that by
     diffing each differing file and failing on any non-hook line, rather than asserting it.
- the can-fail control is unchanged and still required: a copy with an altered version
  string must be reported as different, or the comparator has never shown it can report red.
- arms reopened: A-H1301, which had produced no hooked capture.

### A23 — the hooked binary has to BUILD the fixture, not merely be invoked against it

- `hfix.sh`: `1227077c8691e481c0f5074b847e113acf4a963787ec875e238c7fe7dd2e61a0` -> `51e401d354e20cd8332fb019d3a76122a669df482a87f9054cb68818d7a5c4b5`
- `arm-h1301.sh`: `7794562234bac7a4e1027c3ecea61935d7eea0d84c53c42ff14515a5e7882d1d` -> `880047d483fe32a5bf05fb4f71a08f4a6a2de4842e8e37b26a073694a4aa815c`
- what the run showed: cut 1's barrier reported `mark_seen=no` after waiting 20s, and the
  case then recorded a reader seeing the COMPLETED write — a state at no boundary at all.
  Cut 2 showed both of `_cmd_spawn`'s appends present, which is a complete generation
  rather than a partial one.
- cause: the fixtures were BUILT by the frozen copy, so their generated helpers carry a
  `_lib` with no `_ae_hook` in it. Invoking the hooked binary afterwards cannot help — the
  helper-side writer that owns the cut is the one the fixture already has. `H_AE` now names
  the binary that builds the fixture and the three cut fixtures are built by the hooked copy.
- and the case no longer proceeds past an unarmed barrier: it records OUTCOME-INCONCLUSIVE
  and stops, because everything captured after an unarmed barrier is a completed write
  wearing a barrier's label. The first run published exactly that and it read as a finding.
- arms reopened: A-H1301, whole.
