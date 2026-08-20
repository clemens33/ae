# Joint L classification — L-RENTRANS worksheet (section 6 of 6, PARTIAL)

Seats: fable5:lead (author) + gpt56sol:colead (independent read pending).

Capture: `l-artifacts/L-RENTRANS`, 9 arms (1 preflight gate + 8 coverage), 596 files,
**9** roster ids. Frozen `ae` `b7b8aa9f…`; 5 of the 8 coverage arms under L-HOOKS-v4
(`c66fe2d8…`, reproduced by lead from `git show 72c7293:ae`).

**This section is PARTIAL by measurement, not by omission.** The transport preflight gate
is BLOCKED — two blockers, both measured and recorded in the manifest: the host's only
`rsync` cannot run the frozen invocation, and the frozen `ssh <target>` call cannot reach
a sandbox-only config. The transport-dependent rows were **not run and nothing was
substituted for them**.

Grain requirements carried forward, all four honoured. Added this section:
5. **A zero is only a measurement if THE RECORDER THAT REPORTS IT was demonstrated live in
   the same arm** — not merely *a* recorder. SC-814 proved its ssh recorder and inherited
   an unproven rsync one; the wording had to be tightened after colead found it.
6. **A capture's claim is read from the artifact, never from the producer's summary.**

---

## The design question lexec asked the lead, answered

The manifest asks, without deciding: *"Whether the transport-free subset should run under
the blocked gate is a design question and is asked of the lead rather than decided here."*

**Ruled: YES, and the section is the argument for it.** SC-832a and SC-1303 name neither
ssh nor rsync; part of SC-1302's matrix is likewise transport-free. Gating a row on a
dependency it does not have is a category error — it converts one environment's missing
`rsync` into an absence of evidence about `ae rename`. Three roster ids closed and a
**product defect (#102)** came out of the subset that a section-wide gate would have
skipped. Asking rather than assuming was right; the answer is that gates belong at the
grain of the dependency, not the section.

---

## SC-814 — transfer validates both endpoint names before any side effect

Bucket 1. Arms: `endpoint-validation-hostile-name-push`, `endpoint-validation-hostile-name-pull`.
Transport-free **by construction**: frozen ae validates the session name and path object at
step 1, before the SSH probe at step 4 and before the rsync capability gate.

IS — and the protocol is what makes the zero mean anything:
1. **LIVE-SHIM CANARY first.** A VALID-name transfer runs before the measurement and MUST
   record an ssh invocation: `canary_ssh_invocations 1` in both arms. An arm seeing 0 here
   writes ARM-INVALID.
2. **Logs RESET**, empty state recorded.
3. **MEASUREMENT** with hostile names: `ssh_invocations_after_measure1 0`,
   `rsync_invocations_after_measure1 0`, and the same after measure2 — **in both push and
   pull arms**, `measure1_rc 1` / `measure2_rc 1`.

Two hostile shapes, recorded byte-exact with `od`: `../victim` (the traversal class the
frozen comment itself names) and a second carrying quoting and command substitution with
an embedded sentinel, scanned recursively across the sandbox before and after.
The ssh and rsync shims are delegate-and-log with argv passed through UNCHANGED, proven by
equiv-K (NO_DIFFERENCES) against a control (DIFFERENCES_PRESENT).

**RULED: COMPOSITE — and my praise of this arm was itself the overclaim** (colead's
finding, verified). I called it "the section's model of how to measure an absence". **The
canary is HALF a canary.** `shim-invocations.2after-canary.txt` records
`ssh.log.exists yes / ssh.log.lines 1` but **`rsync.log.exists no / rsync.log.lines 0`**:
the valid-name transfer died at the SSH probe (`canary_rc 1`, probing
`nosuchpeer.invalid`) and **never reached rsync**. So the rsync recorder was never
demonstrated live, and `rsync_invocations 0` in the measurement is an ABSENCE — a dead
rsync recorder produces the identical zero.
The zero-byte `AE_HOME` diff has the SC-823 problem too: it proves **no residue**, not
temporal **no-mkdir / no-path-construction**. Create-then-remove passes it.
**What the arms DO prove, directly:** hostile-name refusal (rc=1, both directions, both
shapes, `od`-exact), **no SSH invocation** — that recorder WAS demonstrated — and no
durable residue.
**What frozen source proves:** ae:11184-11188 runs `_validate_session_name` then
`_require_session_path_safe`, under a comment stating validation happens *"BEFORE any path
is built, any SSH probe fires, anything is stopped, and — decisively — before the pull
path's `mkdir -p` + `rsync --delete` and the push path's remote create."* That is the
temporal half.
**CONFIRMED AS COMPOSITE.** A direct-only confirmation would need a live **rsync** recorder
canary and a writer barrier.
*Harness note:* `rentrans-814.sh` lines 16-17 emit `printf: --: invalid option` into the
capture. Noise, not corruption — the `label`/`exists`/`lines` fields are intact — but it
should be fixed before this protocol is reused.

## SC-832a — rename's effect set

Bucket 2. Arm: `rename-effects` — a REAL `ae rename proj proj2` on a RUNNING server, over
a topology carrying the prefix-sibling pair.
IS — each of the row's four effects observed individually, plus the liveness clause. Not
inferred from diff sizes:

| effect | evidence |
|---|---|
| renames the tmux session | `$0\|proj` → `$0\|proj2`; **same session id `$0`**, so it is a rename and not a re-create |
| moves the session directory | the meta diff compares `1pre.meta.proj.txt` → `3post.meta.proj2.txt` — different paths |
| updates `session=` in meta | `-session=proj` / `+session=proj2` |
| regenerates `workspace.md` | `-Session: proj` / `+Session: proj2` |
| a running tmux server stays up | `server.before-after.diff` **0 bytes** over a file holding `server.pid 72161` and `socket.exists yes` — **the same pid before and after**, so the server was never restarted |

The 0-byte diff carries its claim **here** because what it compares is the pid and socket
existence — liveness facts — not a state record. (Contrast SC-823 in L-FROM, where a
0-byte diff proved a postcondition and was silent about the ordering its row claimed.)
**CONFIRMED.**

## SC-1303 — rename: what a concurrent observer may see mid-operation

**`authority=code-observation`. This is a PLACEHOLDER HEAD of the SC-1305 kind: it states
no SHOULD and therefore CANNOT BE CONFIRMED, however good the arm is.** lexec reported it
"CLOSED by transport-free arms"; that is the **empirical mechanism arriving**, not the
SHOULD being written. Marking it would ratify a non-claim.

Arm: `rename-observer` — the same rename held at each census-named cut in turn, with a
concurrent `ae list --json` from a SEPARATE process at every cut. Four cuts fired in order.

IS, at the four cuts (`ae list` rc=0 at every one):

| cut | `ae list` reports | directory on disk | meta `session=` |
|---|---|---|---|
| `b_rn_locked_entry` | `proj`, `projx` — both running | `sessions/proj/` | `proj` |
| `b_rn_tmux_renamed` | **`proj2`**, `projx` — both running | `sessions/proj/` | `proj` |
| `b_rn_dir_moved` | `proj2`, `projx` — both running | `sessions/proj2/` | **`proj`** |
| `b_rn_meta_updated` | `proj2`, `projx` — both running | `sessions/proj2/` | `proj2` |

Three facts fall out, and only the first is invariant:
1. **The session is never lost and never duplicated.** At every cut `ae list` shows
   exactly two sessions, each exactly once, each running. There is no window where the
   renamed session is absent, and none where both names appear.
2. **The reported name follows the TMUX rename**, one cut before the directory moves and
   two before meta is rewritten — so `list` derives the displayed name from tmux, not from
   meta.
3. **There is a real intermediate disagreement on disk**: at `b_rn_dir_moved` the
   directory is `sessions/proj2/` while its meta still says `session=proj`.

**RULED: BUCKET 3 — fix-known-defect(#103). My proposed closure was WITHDRAWN after
colead's dissent, and they were right.**

*What I proposed and why it was wrong.* I wrote a bucket-1 closure permitting the
disagreement: *"the on-disk directory name and the meta `session=` key MAY briefly
disagree."* That is **measurement editing the contract** — I observed bash leaving
`sessions/proj2/` holding `session=proj` and wrote it into the SHOULD as permitted. The
`MAY` felt safe because it permits rather than requires, but **permitting a hole is still
writing the hole into the contract**, and I did not see it because the permissive
formulation was the one I had been rewarding all day (SC-1305, SC-835d). A formulation
that is right in one place is not a licence.
Colead's decisive argument: it would have **blessed D20's atomicity hole**. If mixed
generations are contractually fine, a crash between the tmux rename and the meta rewrite
leaves a session that is *legitimately* un-findable, and the P3 Rust owner inherits no
requirement to do better.
No DR is required: a DR would be needed to rule mixed generations ACCEPTABLE. They are
ruled a DEFECT, and bucket 3 is the home for a SHOULD the incumbent violates — SC-1302/#75
is the precedent three rows away.

**The adopted SHOULD, with colead's scope precision:** every **product reader or
operation** takes ONE COHERENT LOGICAL SNAPSHOT of the identity generation — old or new,
never mixed. Deliberately **not** "every external reader": tmux and four filesystem facts
live in different stores and cannot be made atomic against an arbitrary lock-free reader
that does not participate in the protocol, and a SHOULD requiring the impossible is
untestable. **Live-reader linearizability lives here; crash recovery and rollback
semantics belong to SC-832c and D20.**

**Note what the evidence actually says**, because it supports colead more than it supported
me: `ae list` — the product's own reader — was COHERENT at all four cuts. The violation is
visible only to a direct filesystem reader. That makes the mixed window a **defect
observation**, not a permission, which is the reverse of how I framed it.

**Landed in the contract; filed as #103.** Two related items travel with it: **D20**
(`ownership.md:422`) recorded the crash family BACKWARDS — *"dir moved but tmux rename
fails"* cannot occur, since tmux renames first (ae:11635) and the directory moves second
(ae:11650); corrected in the same pass. And the pre-existing **SC-832c** ("rename crash
cuts") is the crash-side view of this same window — flagged for seat closure alongside
#103, **not closed here**.

## SC-1302 — a session name's lifecycle operations serialize on one lock

**Bucket 3 — fix-known-defect(#75, intended: native locking, never optional).** Already
carries its conflict; this section neither ratifies nor reopens it.
**PARTIAL — its stop×rename cells ran; its transfer cells did NOT**, so the serialization
row cannot flip to observed from this section, and the manifest says so in both its result
table and its body.

IS — the four transport-free cells, read from `ARM.txt` and `final-state.txt` rather than
from any summary:

| cell | ordered pair | flock | first rc | second rc | final tmux | note |
|---|---|---|---|---|---|---|
| rename-first-with | rename→stop | with | 0 | 1 | `proj2`, `projx` | "no positive server record" |
| rename-first-without | rename→stop | without | 0 | 1 | `proj2`, `projx` | same refusal + degraded note |
| stop-first-with | stop→rename | with | 0 | 0 | `proj2` only | — |
| stop-first-without | stop→rename | without | 1 | 0 | `projx` only | "cannot verify 'proj' was killed" + degraded note |

Two readings, kept separate:

*On the row itself.* The **rename-first pair is outcome-identical with and without flock** —
same rcs, same final state, same refusal. The second operation refuses on a
**name-resolution fact**, not because a lock serialized it. So for this ordered pair the
lock made no observable difference, which is consistent with #75's recorded IS (with flock
absent, serialization silently disables) without adding to it. The **stop-first pair does
diverge** by flock, in the direction #75 describes.

*A producer-summary correction, recorded because it nearly entered this worksheet.*
lexec's handoff stated that with flock REMOVED "both report rc 0 and both claim success".
The artifacts say otherwise: `rename-first-flock-without` is 0/1 with the same refusal as
the flock-present cell; the 0/0 cell is `stop-first-flock-with`. **The captures are
complete and correct; the narrative attached the outcome to the wrong cell.** Caught by
reading `ARM.txt`, which is now grain requirement 6. lexec has since replaced the practice
with a mechanism (generate the per-arm table by command, paste verbatim, write prose only
around it).

**PARTIAL. No disposition change to the bucket-3 row.**

### The defect this matrix surfaced — issue #102

`stop-first-flock-**with**` is the cell that matters, and it is not a locking result:
flock was present and working.

Pre: `$0|proj`, `$1|projx`. `ae stop proj` kills `$0`. Then `ae rename proj proj2` runs
`tmux rename-session -t proj proj2` (traced argv) and returns **0**. Post: `$1|proj2` —
**`projx` has been renamed**, because `tmux -t <name>` PREFIX-MATCHES and the intended
target was already gone.

Frozen source, ae:11635: `tmux rename-session -t "$old_name" "$new_name"` — **by name, and
on the ambient server** (no `-S`). This is the hazard SC-835a documents for `stop`, which
was hardened to `-S <recorded socket>` with an exact session id; `rename` never was. The
`tmux has-session -t "$new_name"` pre-check above it has the mirror defect, falsely
refusing when a prefix sibling exists.

Filed as **#102**, with the other bare `tmux … -t "<name>"` call sites flagged as an audit
item. **Not classified here** — it is a product defect, not a disposition on SC-1302, and
it belongs to whichever row the audit lands under.
lexec declined to classify it despite it emerging from their own topology choice. That
choice — carrying the prefix-sibling pair through every cell of a matrix built for a
locking question — is why a targeting bug fell out of it. **Nobody designed an arm to find
#102.**

## SC-833a, SC-1304a, SC-1304b, SC-1304c, SC-1304d — NOT RUN

Transport-dependent, blocked at the preflight gate, **nothing substituted**. SC-1304a–d are
additionally `code-observation` placeholders of the SC-1303/SC-1305 kind and would need
seat closures even with captures in hand.
**UNCLASSIFIED. The gate evidence is committed with the section.**

---

## Dispositions — POST-GATE

*Colead's read moved two of the four live rows, including one where my own praise was the
overclaim. Conditional pass granted after these corrections; no rerun required.*

- **CONFIRMED AS COMPOSITE — 2**: SC-814 (arms prove hostile-name refusal, no SSH, no
  residue; frozen ae:11184-11188 proves the temporal path/mkdir/rsync halves),
  SC-832a (independently re-read by colead — no finding; the pid/socket/exact-id liveness
  observation IS fail-capable and is *not* SC-823's overclaim class).
- **BUCKET 3, fix-known-defect(#103) — 1**: SC-1303, scoped to product readers, with crash
  semantics delegated to SC-832c/D20.
- **PARTIAL — 1**: SC-1302. Table independently verified by colead from all four
  ARM/rc/stdout/stderr/final-state files; PARTIAL is correct because the transfer cells did
  not run. Bucket-3 row untouched.
- **NOT RUN, UNCLASSIFIED — 5**: SC-833a, SC-1304a-d.

**Section total: 9.** No INCONCLUSIVE arms, no ARM-INVALID.

**New contract rows from #102** (colead's grain, ids corrected to avoid collision with the
existing SC-832b/c placeholders): **SC-832d** — rename addresses its SOURCE by recorded
server and exact live session id; **SC-832e** — rename's DESTINATION occupancy check is an
exact-name check. Both bucket 3, fix-known-defect(#102). SC-1302 and SC-832a untouched.

## What this section changed about how we read rows

1. **A zero needs a live recorder in the same arm** (SC-814). The canary is the whole
   design: prove the shim records, reset, then measure. Every other absence-claim in batch
   L should be read against this standard.
2. **A 0-byte diff means what its CONTENT means** (SC-832a vs SC-823). Over pid and socket
   existence it proves liveness; over a tree manifest it proves residue and says nothing
   about ordering. The artifact's size is never the argument.
3. **Read the artifact, not the summary** (SC-1302). A complete, correct capture set was
   described by a wrong sentence, and only the `ARM.txt` files disagreed.
4. **A matrix built for one question can answer another** (#102). The finding came from a
   topology decision, not from an arm designed to find it — which is an argument for
   building fixtures with realistic hazards in them even when the row under test does not
   mention them.
