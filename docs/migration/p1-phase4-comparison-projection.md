# P1 phase 4 — fixed comparison projection

This file defines the comparison relation used by `p1-phase4-gate.md`. It is a fixed
input to a parity run. A runner may choose its parser implementation, but it may not
choose another projection or classify an unparsed byte as an open choice.

The invocation class comes only from the pinned P1 row in `INVOCATIONS.tsv`. A row is a
digest invocation iff its `normalised_argv` token sequence contains the exact token
`--json`; it is a human-list invocation iff `surface` equals `ae list` or `ae ls` and the
token is absent; every other P1 row is opaque. The current fixed partition is 401 digest,
458 human list/ls, and 206 opaque rows. Consumer names and suffixes never select a class.

## `rc`

Project the scalar process exit status as an integer and compare it exactly unless the
fixed open-choice register excludes that scalar for the invocation. No stdout or stderr
fact is part of the `rc` projection.

## JSON stdout

For a P1 digest invocation, stdout must contain exactly one RFC 8259 JSON value followed
only by JSON whitespace. Reject duplicate object-member names before projection. Project:

- strings as decoded Unicode scalar sequences;
- numbers as their exact mathematical decimal value;
- booleans and null by type and value;
- arrays as ordered element sequences; and
- objects as both a unique name-to-value mapping and a separately addressable member-order
  sequence.

JSON supplies one number type rather than separate integer and float types. Exact
mathematical-value projection deliberately makes `2`, `2.0`, and `2e0` equal; string,
boolean, and null values remain different types.

Insignificant JSON whitespace and equivalent string escaping do not enter the projection.
Object member order does enter it and is removed only by
`OC-P3-JSON-FIELD-ORDER`. Member presence remains in the mapping unless an exact
member-presence locus is removed by `OC-P2-SEMANTIC-FALSE-PRESENCE` or an enumerated
`OC-P3-MACHINE-LOSS-RECORDS` pointer. A member value remains unless the exact top-level
`generated_at` value locus is removed by `OC-P3-GENERATED-AT`; that removal never removes
the member name, presence, JSON string type, or the requirement that its value is one valid
SC-510a timestamp. No other register row removes a value from a baseline member outside
an enumerated `OC-P3-MACHINE-LOSS-RECORDS` subtree. Array membership and order always
remain.

For a non-digest invocation whose successor stdout parses as JSON, criterion 9's two
forbidden top-level fields are projected from the same duplicate-rejecting parse. The
rest of that stdout stays in the opaque projection below; parsing it does not convert the
invocation into a digest surface.

## Human `list`/`ls` stdout

For a non-JSON P1 `list` or `ls` invocation, project two layers:

1. the ordered semantic session and agent rows required by SC-017m/n, SC-017h/r, and
   phase-3 criteria 4–11; and
2. the original byte stream partitioned into the closed roles below.

The semantic row projection retains stable identity, every displayed contract field and
value, row membership, and row order. Locate the agent-health cell and map its literal
token to three-valued semantic health only through the agent-health presentation manifest
pinned by criterion 1 and calibrated by criterion 8. Neither neighboring state/reason
text nor the successor capture may select the cell or define the mapping during
comparison. A parser that cannot recover a required semantic row or value fails the
comparison.

The byte roles are closed:

- `header`: initial column-label lines before the first semantic row. When a view has no
  semantic row, recover the baseline and successor header spans independently. On each
  side, a claimed span must be byte-identical to the header span recovered on that same
  side from a paired nonempty P1 invocation in the same case with the same command,
  surface, flags, and presentation settings. A successor span never calibrates the
  baseline or vice versa; without the same-side, same-case pair that zero-row view has no
  `header` span;
- `layout`: ANSI SGR bytes, column separators, padding, and line-ending whitespace around
  already recovered semantic fields;
- `semantic-field`: the byte span carrying a recovered field value; and
- `residual`: every other stdout byte, including summaries, footers, counts, and trailing
  markers.

The assignment is total and disjoint: every stdout byte belongs to exactly one role.
Every excluded span is minimal for the atom it represents; a `layout` span contains no
semantic-field byte, an agent-health token span is exactly that recovered cell's literal
span, and a header span contains only its calibrated initial lines. An overlap, double
assignment, non-minimal excluded span, or unassigned byte fails comparison.

`OC-P3-HUMAN-LAYOUT` may remove only `header` and `layout`. It never removes a
`semantic-field` or `residual` byte. `OC-P3-AGENT-HEALTH-TOKEN` may remove only the
literal span of an already recovered agent-health cell; the semantic health value remains.
`OC-P3-EQUAL-NAME-TIE` may remove only the relative-order relation for the exact tied
identities. Every remaining semantic fact and byte role compares exactly after applying
directional obligations.

## Opaque stdout

For every other P1 invocation, project stdout as its exact byte sequence. No human-layout
or JSON-output exclusion applies merely because the bytes resemble another surface.

## Stderr

Project stderr as an ordered byte sequence plus any registered semantic diagnostic span.
Every byte not inside an admitted registered span is `residual` and compares exactly.

- `OC-P3-HUMAN-DIAGNOSTIC` may identify a span only on its fixed scope and must still
  project diagnostic presence plus the exact distinct-source loss count.
- `OC-P3-JSON-WARNING` may identify a span only after the opposed completeness calibration
  required by the register and criterion 8. The same calibration must show the span absent
  in the fixed-complete arm.

A parse failure, overlapping spans, or an unassigned byte fails comparison. Wording and
path-detail exclusions do not remove unrelated stderr.

## No mutation

Project `no-mutation` as the relation between the per-invocation verified initial scratch
fingerprint and its post-run fingerprint, plus the independently enforced zero-write fact
for the frozen corpus. It passes only when the scratch relation matches the recorded
no-mutation expectation and the corpus write count is zero. It is never inferred from a
successful process exit.
