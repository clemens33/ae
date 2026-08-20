# Batch C design v3 — read-side evidence run (62 assignments)

BOUNDARY REOPENED per the standing contract-change rule (slice-1b added SC-405i/j/k
to C; SC-405d reclassified in place, its arm unchanged). Delta: three arms appended
to A7/A9; everything else stands as gated.

Corrected per the seat gate (six blockers folded). NO evidence worker runs before this
document AND its prerequisites are seat-green.

**Binding invariant (gate):** every capture must be independently able to FALSIFY its
assigned frozen-IS claim, using the same layer the product reads, on uncontaminated
inputs, without importing an expected value.

## Prerequisites (hard gates)

1. **B0 designs** (SC-507b, SC-511c, D01–D04 concurrency, SC-1208): **SATISFIED**
   — all eight designs seat-approved (colead, ae-20260820T142705Z-afec1702, on
   b0-design.md v2.2 at c6cbccd). The approved D01–D04 designs are hereby STAGED
   into this batch for the C worker to execute. EXECUTION OWNERSHIP (B0 preflight ruling): B0 owns the DESIGNS for D01–D04
   concurrency; the C worker EXECUTES those approved designs and returns their
   evidence; the B0 worker itself executes only SC-507b/511c/1208. C consumes approved
   DESIGNS; it never redesigns them.
   SC-511c is REMOVED from this batch's arms (B0 owns it).
2. **T-WD producer precursor**: **SATISFIED** — twd-precursor.md v2 approved
   (colead, 2026-08-20, at 4af7be0); the C worker RUNS it as fixture-harvest step
   zero — the only legitimate source for G2's attention bytes (SC-980's
   incumbent-capture task).

**Value-blindness (spawn-authorization blocker, folded):** every worker-facing
clause below is manipulation + barrier + capture ONLY. All expected relations
(membership rules, positive/negative case labels, degraded-flag expectations,
rollup semantics) live in the SEAT CLASSIFICATION ANNEX at the end of this file,
which is NEVER included in any worker brief.

## Instrumentation admissibility

Governed by cluster-plan.md's global rule "Instrumentation admissibility" (the
normative home for every batch); this batch adds no local exceptions.

## Template groups and per-arm sandboxes

Fixture state is harvested ONCE into immutable **template groups** (chmod-protected,
fingerprinted). Every ARM × LANE (bash lane / rust lane) then **clones a fresh
`AE_HOME` from the template fingerprint** — no arm and no lane ever shares a sandbox
(the bash lane cannot contaminate the rust lane; a mutating arm cannot leak into a
read-only one). Mutating arms (A8) receive writable clones; read-only arms run on
protected clones. Where a row needs LIVE state (running/alive/attach/status-inside),
the arm creates a **dedicated tmux server** (own socket dir per the harness-slice
marker discipline) with controlled panes/processes — shells and fixture scripts, never
live models.

Template groups (a GROUP holds one or more session dirs + optional tmux topology):

| Group | Contents |
|---|---|
| G1 healthy | 2-agent session dir, full ratified meta keys, harvested event history with zero attention reasons |
| G2 attention | 6 session dirs, one attention reason each — event bytes from the T-WD precursor (dead/stale/throttled) and real helper runs (waiting-user/blocked via state; unanswered via ask-pair aging) |
| G2b competing | 1 session dir, multiple agents with COMPETING reasons, arrival order REVERSED relative to severity (SC-017g arm input) |
| G3 degraded | session dir with mode-000 meta; sibling with a malformed COMPLETE event line (producer-derived, named byte-diff) |
| G4 quiet | session dir with NO events.jsonl; sibling with zero-byte events.jsonl |
| G5 requests | ONE harvested valid ask/reply mirror pair + FIVE producer-derived mutations (below) |
| G6 stopped | stopped session dirs incl. one with attention-shaped history |
| G7 tolerance | meta with unknown keys; events with unknown keys/action (producer-derived + named additions) |
| G8 tail | events.jsonl without trailing newline + partial trailing record (harvested then truncated, diff recorded) |
| G9 goals | goal events with DISTINCT deterministic timestamps (clock hook, below) |
| G10 identity-pairs | same-display/different-routing-key pair + display-only legacy pair (SC-511b preference proof) |
| G11 escapes | harvested events whose payloads carry each documented escape class — quote, backslash, newline, tab, CR — producer INPUT bytes and emitted bytes both captured (SC-510d) |

**Producer-derivation rule (gate wording):** every fixture byte is PRODUCER-DERIVED:
harvested from real frozen producers (generated helpers state/goal/memo/say/ask/
review/reply; `_register-sid` and `_recover-pending` for recover-ref bytes — both now
in the harvester list; the T-WD precursor for alert bytes; a real `ae` launch for
meta), then mutated ONLY by the arm's NAMED manipulation with a recorded byte diff.
Hand-authoring is forbidden; "extracted" applies to valid lines, "producer-derived +
named mutation diff" to malformed/truncated/future/mismatch fixtures.

## G5 request-pair protocol (gate B3 — the real reply helper refuses a mismatched
responder, so wrong-target replies cannot be driven live)

Harvest ONE valid ask→reply mirror pair from two scratch agents. Derive five
mutations, each a named coherent byte diff (actor/target AND their slot/session
routing fields mutated together):
1. correct mirror (unmutated control)
2. wrong ref
3. same ref, wrong actor
4. same ref, correct actor, wrong target
5. routed-vs-routed mismatch (both sides carry routing keys, keys disagree)
6. mixed routed/display (one side routed, one display-only)
Each is an independent SC-518 conjunct falsifier; 5–6 double as SC-511b
routing-preference arms with G10.

## Time protocol (gate B4)

- A **fixed-clock hook** for every time-sensitive arm: a PATH-first `date` shim in the
  sandbox (bash lane) and an injected clock (rust lane); no arm computes "now" and
  races the real clock.
- G9's goal events carry DISTINCT timestamps via the clock hook between producer
  invocations.
- **SC-524 source-discrimination pair** (bash reads events.jsonl MTIME for activity,
  not event ts — ae@72c7293:3993-4009,4220-4228): two subarms on identical cloned
  inputs per lane — (a) future event ts / ordinary mtime, (b) ordinary event ts /
  future mtime. The seats see the incumbent-source divergence directly instead of a
  fixture where both sources accidentally agree.
- Threshold arms (SC-522, 523a/b): fixed clock; equality vs strictly-past as two
  inputs. Default-value arms run under a SCRUBBED environment (env -i plus the
  documented minimum), never inherited shell state.

## Read-only and barrier proofs (gate B6)

- Read-only rows: a recursive MANIFEST before and after — path, type, mode, symlink
  target, content hash (atime deliberately ignored) across the cloned AE_HOME — plus
  tmux server/session/window/pane/client snapshots. A listing is not a proof; the
  manifest diff is.
- **SC-020b named barrier**: pause after next's session RESOLUTION, kill/remove that
  exact session, resume, capture the non-attach/non-recreate outcome. This arm may
  reuse D04b's B0 hook ONLY if the approved B0 design names this exact cut and
  capture; otherwise it runs its own barrier as specified here.

## Arms (nine groups, unchanged coverage, corrected mechanics)

A1 schema/document (SC-509, 509b, 506, 510a-d, 511a-b, 405k): G1/G3/G7/G8/G11; 510c's
recover-ref via the `_recover-pending`/`_register-sid` harvest; 511c REMOVED (B0);
405k — live tmux topology with one EXTRA runtime pane absent from the roster AND one
roster slot whose pane is absent; capture the rendered agents[] (membership and
per-agent alive) plus the tmux snapshot.
A2 filters (SC-017a-f, 017i, 521a): G1+G2+G6 clones; one invocation per flag/alias;
the two intersection arms; ls alias.
A3 rollup/severity (SC-017g, 017h, 524): G2 + G2b (competing/reversed); the amended
017g additionally gets a SESSION-LEVEL unanswered request aged past threshold
competing against at least one agent-owned reason; capture the session attn field
AND every agents[].reason; the SC-524 source-discrimination pair.
A4 status/next (SC-016a-d, 513a-c, 019, 020a-c): live-tmux arms on dedicated servers;
never-attaches via client-list snapshots; SC-020b per its named barrier.
A5 exits (SC-514): doctor under a CONTROLLED PATH/capability fixture — clean arm and
planted-failure arm both run the frozen script through a known bash; removing the one
planted dependency cannot remove the interpreter or other checklist items.
A6 requests/pairing (SC-518, 522, 523a-b): G5 protocol; scrubbed-env defaults;
fixed-clock thresholds. (SC-212c is H-HELPER's — removed per gate; no incidental
closure.)
A7 meta grammar (SC-405a-g, 405j): G1/G7 + malformed/duplicate-key producer-derived
fixtures (405d/e captures remain observation-only for UNCLASSIFIED rows); G9 for
405f; 405g's two named resolution subarms (tmux @ae_branch_name primary on a live
server; git fallback on a stopped clone); 405j identity arm — FOUR producer-derived
cases sharing ONE display name: (1) full+fresh routing keys; (2) full but
stale/mismatched keys; (3) partial keys — slot-only AND session-only; (4) keyless
legacy event. Run the consumer on each case; capture per-case outputs.
A8 modes (SC-101, 102a-b, 018b): WRITABLE clones per arm; fast-path attach with full
manifest+tmux diff (101); resume regeneration set diff (102a); inside-session
invocation (102b, live server); use-against-existing arm (018b). (SC-100 and SC-018
are DEFERRABLE — removed per gate; the batch boundary is exactly the 62 assigned
after the slice-1b reopening.)
A9 quiet-vs-degraded (SC-519, 520, 405i): G4 clones (quiet both ways) vs G3 clones;
capture the full rendered session JSON plus any retained generation/offset/reason
fields; 405i — a present session dir with META ABSENT (named mutation), distinct
from G4's missing EVENTS and G3's unreadable meta; capture the rendered session
JSON (all flags and identity fields) and file manifests.

**SC-1306a-e artifact mapping (explicit, so D-record evidence cannot leave the SC
rows implicit):** the snapshot-cut rows ride the approved b0-design.md designs —
SC-1306a → D01 (Design 2, list), SC-1306b → D04a (Design 5, status), SC-1306c →
D04b (Design 6, next), SC-1306d → D02 (Design 3, request scan), SC-1306e → D03
(Design 4, events-tail). Their captures are those designs' captures; no separate
arms exist for them.

## Per-row differentiators (gate B2 — the discriminating manipulation per row;
common capture/barrier boilerplate applies to all)

| Row | Discriminator |
|---|---|
| SC-016b | each of >=2 panes filled with >80 UNIQUELY NUMBERED lines; capture the rendered per-pane output (which numbers survive) AND the binary/pane-id labels |
| SC-405a | a producer-derived meta value containing MULTIPLE `=` characters — first-equals split distinguishable from any-equals split |
| SC-405f | goal APPEND ORDER OPPOSED to timestamp order (clock hook writes an older ts after a newer one) — last-record vs max-ts implementations become distinguishable |
| SC-405g | tmux `@ae_branch_name` and the git branch populated with DIFFERENT values before capture — source ownership observable, not just transport |
| SC-510b | producer cases with genuinely EMPTY target/ref/summary at the producer input — omission-vs-empty-string observable in emitted bytes |
| SC-511a | one producer case with KNOWN routing (both slots/sessions resolvable) and one with genuinely OMITTED routing — presence and omission both exercised |

## Date shim contract (gate IMPORTANT)

The PATH-first `date` shim DELEGATES EVERY invocation to the real binary EXCEPT the
exact frozen now-form(s) it substitutes; the real date's path and hash are recorded in
the run manifest — the shim must never become the parsing/formatting behavior under
test. Protected read-only clones are built WITH a prebuilt config so the M2 bootstrap
cannot turn chmod protection into a synthetic failure.

## Lanes, ordering, environment

Per arm: bash lane first on its own clone, rust lane on ITS own clone (same template
fingerprint) — divergence between lanes is REPORTED, never resolved by the probe.
Fixed arm order; single-threaded; TZ=UTC, fixed LANG, scrubbed env per arm; frozen
commit verified by hash; artifacts under batch-c-artifacts/ with a manifest mapping
assignment → row ids → artifact paths → template fingerprints → mutation diffs. No
verdicts anywhere — seats classify; contradictions become bucket-3/DR reopenings;
measurement never rewrites SHOULD; bucket-3/4 rows capture the incumbent baseline
only.

---

## SEAT CLASSIFICATION ANNEX — never included in any worker brief

The expected relations moved out of the worker-facing clauses (spawn-authorization
ruling). Seats apply these to the returned captures:

- **SC-405k (A1)**: agents[] MEMBERSHIP follows the roster while per-agent `alive`
  follows the runtime — the extra runtime pane appears in no roster entry; the
  pane-less roster slot appears with alive=false.
- **SC-017g (A3/G2b)**: the session attn field is the MAX across agent reasons
  plus session-level unresolved-request facts — not first-wins or last-wins under
  G2b's reversed arrival order; the session-level unanswered request participates
  in the max WITHOUT appearing in any agents[].reason.
- **SC-405j (A7)**: case 1 (full+fresh keys) is the positive control; case 2
  (stale/mismatched keys) negative; case 3 (partial keys, both halves) negative —
  partial keys are unassociated, never display-fallback; case 4 (keyless legacy)
  positive via display fallback. A stale-only arm would be insufficient for the
  precised total-decision row — the four cases exist to separate fallback-vs-key
  behavior independently.
- **SC-519/520/405i (A9)**: G4's quiet shapes stay healthy; G3's degraded shapes
  carry the public degraded marker with retained generation/offset/reason; 405i's
  meta-absent dir renders degraded:true with surviving directory identity —
  distinct from both quiet-absence and unreadable-meta, closing the
  missing-vs-unreadable-vs-quiet triangle.
- **SC-016b (differentiators)**: the per-pane capture boundary sits at ~80 tail
  lines — which uniquely numbered lines survive locates it; binary and pane-id
  labels must be present per pane.
