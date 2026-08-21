# P1 phase 3 — pre-registered falsification gate

Rows: SC-017m, SC-017n, SC-521c, SC-017o, with SC-017f selection parity.
Input: one completed phase-2 classified snapshot plus its inventory-completeness and
logical-loss facts. Output: the human and JSON surfaces an operator or consumer sees.

**Authored by gpt56sol:colead, 2026-08-21, WITHOUT reading `src/` and before phase 3
exists.** Acceptance criteria derived after seeing an implementation are shaped by it;
this file fixes the failure conditions while they can still be independent.

**PASS requires every structural check and required test below.** Each criterion labels
the kind of obligation it observes: `STATE`, `PAIR`, `SET`, `SEQUENCE`, or `ORDER`.
Output-only agreement cannot prove that a renderer presented its input rather than
recomputed it, so the gate separately observes the presentation boundary and its external
reads. `presentation enter` means the first phase-3 operation after classification and
before filtering, sorting, or formatting; a late formatter-only marker is invalid.

The phase-3 handoff in `p1-phase2-gate.md` is absorbed into criteria 12 and 13 here. It
is provenance for these criteria, not a parallel gate to run or maintain separately.

## Status

**NOT RUN — phase 3 does not exist.** The lead holds this gate unread by the builder
until implementation handback.

## Reference classified snapshot

The main fixture has four candidates in each status group. Within every group the names
are deliberately hostile to creation order, locale collation, and natural-number sort:
`AlphaR`, `ZetaR`, `alpha10R`, `alpha9R` for running; the same suffix pattern with `U`
for unknown and `S` for stopped. Per group, assign the four rows these independent facts:

1. attention=true, active=false;
2. attention=false, active=true;
3. attention=true, active=true;
4. attention=false, active=false.

Stable identities remain distinct. Pre-register a finite adversarial input-order set:
reverse C/status order; the fixture's deliberately opposed creation order; a
status-interleaved order; and a fixed worst-case order for each input-sensitive failure
mode named in criterion 9. Natural-number, case-folded, and non-C-locale comparators are
also calibrated on the planted names even though they do not depend on input order. A
fixture manifest records every input identity, status, attention/activity fact,
degradation fact, completeness fact, creation position, and supplied position before
rendering. The expected C-byte order within each group is `Alpha*`, `Zeta*`, `alpha10*`,
`alpha9*`.

## Falsification criteria

1. **[STATE/PAIR] THE REAL LIST ENTRY POINT REACHES THE COMPLETE PIPELINE.** Invoke the
   actual `list` and `ls` surfaces, human and JSON, over the nonempty reference snapshot.
   Observable: both spellings reach phase-3 rendering, return the expected semantic rows,
   and emit no `NO_SESSION_SOURCE`/unwired-session refusal. Opposed control: the accepted
   phase-2 baseline still refuses or has no callable render path. FAIL if only a renderer
   unit test passes, one public spelling stays refused, or the command returns an empty
   success over the nonempty fixture. Exact removal site and internal wiring are **OPEN
   CHOICES**.

2. **[SEQUENCE/SET] PRESENTATION STARTS FROM ONE COMPLETED CLASSIFIED SNAPSHOT.** Record
   named `classify complete` and first `presentation enter` events, with a semantic
   fingerprint of the classified identities and facts at both. The latter marker must
   precede every phase-3 filter, sort, and formatter, not merely the final serializer.
   Observable: classification completes first; every human/JSON presentation input equals
   that completed set; the input snapshot gains, loses, or changes no candidate identity
   or semantic fact after `presentation enter`. FAIL on presentation from a partial input
   before classification completes, different pipeline inputs per surface, a late marker
   that excludes preprocessing, or presentation calling classification. Forming a filtered
   output projection is required and is not mutation of the input. Moving, borrowing,
   copying, or streaming bytes from that completed input are **OPEN CHOICES**; so are
   sorting before versus after filtering and buffered versus iterator-based presentation.

3. **[PAIR/BOUNDARY] A RENDERER PRESENTS FACTS; IT NEVER RE-DERIVES THEM.** Render one
   fixed snapshot in two opposed external worlds after `presentation enter`: tmux reports the
   opposite liveness for planted running/unknown rows, event/attention files report the
   opposite attention/activity facts, durable traversal order changes, and tmux emission
   order changes. Instrument tmux, durable-root/meta, and event-store access from the
   presentation boundary onward. Observable: zero such reads/calls and identical semantic
   selection, status, attention/activity filtering, and order in both worlds. FAIL if
   output stays coincidentally equal but a forbidden read occurs, or if any external fact
   changes it. Before relying on any zero, calibrate each access recorder through the same
   primitive with a neutral leg that reports zero and a deliberately invoked control that
   reports the named access; an uncalibrated recorder makes the arm invalid. Clock reads
   used only for already-ratified presentation fields are outside the access prohibition,
   but an opposed clock control must leave selection, status, attention/activity
   filtering, and order unchanged.

4. **[SET] BASE STATUS VIEWS HAVE THE EXACT ACTIVE/HISTORY DOMAINS.** Against both human
   and JSON surfaces, default and explicit `--running` select every running and unknown
   identity and no stopped identity; `--stopped` selects every stopped identity and no
   running/unknown identity; `--all` selects all three groups. FAIL on omission of unknown
   from either active view, inclusion of stopped in an active view, inclusion of active
   rows in stopped, or a default/`--running` disagreement. The same-dimension last-selector
   rule remains SC-521b's existing gate rather than being redefined here.

5. **[STATE/PAIR] STATUS IS RENDERED LITERALLY AND NEVER RELABELLED.** Plant at least one
   row of each status plus the unknown x degraded pair from phase-2 criterion 13. Human
   output must spell `running`, `unknown`, and `stopped` explicitly; JSON
   `sessions[].status` must carry those exact lowercase strings. Filtering changes only
   membership: a retained unknown remains unknown and a retained running/stopped row
   retains its status. FAIL if unknown is blank, degraded, running, stopped, omitted, or
   represented only by a warning; fail if a filter rewrites any status. Human column
   widths, color, headers, and unrelated SC-017h fields are outside this gate.

6. **[SET/PAIR] ATTENTION FILTERING USES THE POSITIVE FACT ON RUNNING OR UNKNOWN.** For
   default, `--running`, and `--all`, `--needs-attn` retains exactly the attention=true
   running and unknown rows from the reference matrix, in their unchanged status groups.
   Attention=false running/unknown controls are excluded. Stopped rows with attention=true
   are excluded, and `--stopped --needs-attn` plus the reversed flag order are empty.
   FAIL if all unknown rows pass merely because they are unknown, if matching unknown is
   hidden, if stopped passes on a positive attention fact, or if attention relabels status.

7. **[SET/PAIR] ACTIVITY FILTERING USES THE POSITIVE FACT ON RUNNING OR UNKNOWN.** Repeat
   criterion 6 with `--active`: retain exactly active=true running/unknown rows under
   default, `--running`, and `--all`; exclude active=false controls and every stopped row;
   require both argument orders with `--stopped` to be empty. `AE_LIST_ACTIVE_SECS`, event
   chronology, and how the positive input fact was computed belong upstream; phase 3
   consumes the supplied fact.

8. **[SET] CROSS-DIMENSION FILTERS INTERSECT WITHOUT STATUS-DERIVED SHORTCUTS.** Combine
   `--needs-attn --active` over default, `--running`, and `--all`: retain only rows whose
   attention AND activity facts are both true, for running and unknown. Cross either
   flag order with `--stopped`: result is empty. FAIL on union, last-filter-wins, an
   invented usage error, all-unknown admission, or status relabelling. Alias spelling
   parity for the pre-existing domain remains SC-017d/e's obligation; criterion 11 binds
   both command spellings across this phase-specific domain change.

9. **[ORDER/PAIR] THE PRODUCT OWNS C-BYTE NAME ORDER AND STATUS-GROUP ORDER.** For every
   preregistered adversarial input order in the finite reference set, assert the exact
   identity sequence on human and JSON surfaces: raw C-byte name order within a group and
   group order running, unknown, stopped. This does not require all factorial
   permutations. Default/`--running` retain the running-then-unknown prefix;
   `--stopped` retains only stopped; attention/activity filters retain the same relative
   order among selected rows. FAIL on input/creation order, natural-number order,
   case-folding, locale collation, or a status-first order other than the ratified one.
   Relative order of distinct identities with byte-identical session names is an **OPEN
   CHOICE** because SC-017n supplies no tie-breaker.

10. **[PAIR/CONTROL] THE ORDER ARM CAN DISTINGUISH PRODUCT ORDER FROM AMBIENT ORDER.**
    Demonstrate before relying on criterion 9 that the planted input permutation and an
    opposed tmux-emission sequence each differ from the required C/group sequence. Run
    under `LC_ALL=C` and at least one available non-C locale whose control collation is
    demonstrated to differ for the planted names; an arm whose control agrees is
    INCONCLUSIVE, not a pass. Observable output remains byte-order identical across all
    valid arms, with no tmux query after `presentation enter`. Sort implementation, stable-sort
    algorithm, and whether filtering precedes sorting are **OPEN CHOICES**.

11. **[SET/ORDER] HUMAN AND JSON HAVE IDENTICAL SELECTION AND SEMANTIC ORDER.** For every
    status-view and attention/activity combination above through both `list` and `ls`,
    parse human rows and JSON `sessions[]` into stable identity + status sequences.
    Observable: both spellings and the two surfaces agree exactly. FAIL if an alias or
    `--json` ignores a filter, includes an extra row, omits a row, or orders the same
    selected identities differently. JSON object-field order and human presentation bytes
    are not compared.

12. **[PAIR/SET] EVERY HUMAN VIEW EXPOSES INCOMPLETENESS WITHOUT CHANGING FOUND ROWS.**
    Absorb the phase-2 handoff fixtures: use phase-1 criterion 24's separate one-source
    failure and its simultaneous canonical-root + worktrees-root failure. The positively
    owned ambient live candidate is the found row in both, with positive attention and
    activity facts; the incomplete inputs carry respectively one source key and two
    distinct source keys in the test's semantic loss projection. This does not mandate a
    product key or public loss-record schema. Their complete controls make the failed
    sources readable-empty, so all three arms have the same found candidate set. Through
    both `list` and `ls`, run
    every supported human status/filter combination exercised by criteria 4, 6, 7, and 8,
    including the empty stopped/filter intersections. For each view, emitted rows equal
    that filter applied to candidates the inventory actually found; do not compare against
    hidden identities visible only when a failed source becomes readable. Complete
    invocations emit no diagnostic; one-loss and two-loss invocations emit explicit stderr
    diagnostics whose counts equal the cardinalities of the distinct carried source-key
    sets (`1` and `2`). FAIL if only `--all` warns, any filter or alias hides the warning,
    one loss is counted twice, the count is a boolean or hardcoded value, found rows change,
    or a synthetic candidate/status represents the loss. Exact wording, optional
    paths/targets, and exit status are **OPEN CHOICES**.

13. **[PAIR/STATE] EVERY SUCCESSOR JSON DIGEST CARRIES SNAPSHOT COMPLETENESS.** Across
    every status/filter combination through both `list` and `ls`, emit complete and
    incomplete successor controls. Observable in every document: numeric
    `schema_version: 2` exists exactly once; top-level `inventory_complete` exists exactly
    once and is true versus false; and every retained session object preserves its
    phase-2 SC-509/SC-509b fields and values, including degradation, with filtering only
    changing membership. The selected `sessions[]` sequence otherwise agrees. Repeat with
    an empty complete inventory and an empty incomplete inventory so empty rows cannot
    imply completeness. FAIL on a filter-specific version 1, missing/duplicate version or
    completeness field, dropped/forced degradation, or a new loss-shaped session. Frozen/
    version-1 output remains unchanged and does not acquire the successor field. Detailed
    machine loss records, their order, and JSON stderr warning policy are **OPEN CHOICES**
    because SC-017o requires only the boolean on the machine surface.

14. **[PAIR/CONTROL] COMPLETENESS AND STATUS/DEGRADATION ARE INDEPENDENT AT THE LAST
    BOUNDARY.** Hold the unknown x degraded matrix and every selected identity fixed;
    flip only snapshot completeness and its loss facts. Observable: human status,
    selection, and order plus JSON status, degradation, selection, and order remain
    identical while only the human diagnostic and JSON completeness value change. FAIL if
    incomplete forces unknown/degraded, complete clears an existing unknown/degraded fact,
    or warning construction alters rows. This is the render-side complement of phase-2
    criterion 23, not a new classification rule; it does not invent a human degradation
    field.

15. **[SCOPE GUARD] DO NOT MAKE PRESENTATION CHOICES INTO CONTRACT.** Human table layout,
    colors, headers, whitespace, diagnostic wording/path detail/exit status, JSON object
    field order, detailed machine loss records, internal collection/sort implementation,
    and equal-name tie-breaking are **OPEN CHOICES** where no cited row constrains them.
    The gate itself FAILS if a test rejects an otherwise correct phase-3 implementation
    for one of those choices. It also fails if it reopens phase-1 discovery or phase-2
    liveness/schema decisions instead of consuming their completed snapshot.
