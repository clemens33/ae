# Phase-4 contract-to-obligation reconciliation

Independent reconciliation required by phase-4 gate criterion 3. It is a
contract reading and raw-corpus check by `gpt56terra:c3recon`, not a projection
of the obligation generator.

Fixed contract: `docs/migration/semantic-contract.md`, git blob
`896d08ea3ac753095c04af17dfba92cd9d15fb38`. The verifier rejects any other
contract bytes. The governing criterion is phase-4 gate blob
`3a63f7416ccda870a503ac5e11fb2f53ccbea2a1`, criterion 3. Corpus inputs are
`INVOCATIONS.tsv`, the frozen Batch C case artifacts, and `OBLIGATIONS.tsv`.
The contract byte pin is also the contract-to-inventory completeness control:
a newly ratified or reclassified row changes the pin and forces this inventory
to be re-derived before its fixed ID set can be accepted.

Companion inventory:
`p1-phase4-contract-obligation-loci.tsv`. Verifier:
`verify-contract-obligation-reconciliation.py`. Isolated red proof:
`redproof-contract-obligation-reconciliation.py`. The verifier checks every
inventory key and its exact `(p1_disposition, obligation_ids, corpus_loci)`
triple, so a gap cannot be silently reclassified and an inventory link cannot
drift without turning RED. Every directional-gap successor is also exact-pinned.
`independent_raw_basis` is an explanatory audit index; its support is the raw
selector implementation and both-direction set comparison, not a separately
pinned prose assertion.

## Method

This check never invokes or imports `obligations.py`, and does not parse its
authority prose. It independently:

1. selects the 1,065 `phase=P1` rows from `INVOCATIONS.tsv` and joins the
   corpus case by the consumer-path dirname;
2. reads command shape from `normalised_argv`, raw `tmux.before.txt`, raw
   `case.txt`, and captured list output/JSON; and
3. constructs the exact `(case, consumer, obligation_id, stream, locus)` set
   for ordinary directional fields, and the stricter `(case, consumer,
   obligation_id, stream, session-and-agent-qualified locus, from, to,
   predicate)` set for SC-509c, then compares each in both directions with
   `OBLIGATIONS.tsv`.

The raw selectors are intentionally different in shape from a contract-table
generator: JSON is discovered from argv, failed transport only from an explicit
`error connecting` captured tmux transcript (a missing transcript establishes
neither failure nor reachability), selected human agents from captured rendered
rows, and agent JSON from parsed captured JSON. The human-agent extractor is
bounded to this frozen renderer: a two-space row whose first token is an
`alias:name` identity; it is not asserted as a general output grammar. The
selector-missing arm is likewise asserted only by concrete raw `UNREADABLE
/meta` or `FILE ABSENT` template evidence; a missing template/metadata file
cannot manufacture a locus. The selectors neither import nor shell out to the
table generator.

SC-509b is selected only when raw fixture metadata identifies that exact
session's `meta` as `UNREADABLE` or `FILE ABSENT`, while captured JSON still
lacks `degraded`. Twenty raw P1 JSON inputs carry that loss evidence; fourteen
render it as `sessions[].degraded` fields. SC-509c is selected only when captured JSON has
`reason: null` at `sessions[name].agents[ref].reason` and raw producer bytes
prove that agent's contribution: either an exact self-declared `state` event
(`actor == ref`, event `ref ==` captured state), or a target-named watchdog
alert. Alert text maps exactly to `dead`, `stale`, or `throttled`; a later
event from the alert target clears it, and the latest surviving alert wins. The
two raw carrier populations are disjoint at the full session-and-agent address.
This selector deliberately accepts only `action=alert` and those exact three
captured summaries. The table-side selector has a broader action/prefix shape;
the frozen corpus has four target-named `action=throttled` events with the
unmapped summary `upstream throttling detected — pausing nudges`, and no
non-exact alert summary. That difference does not affect these bytes; any
future difference appears as a reverse-direction failure.

## Result

The independent result has **1,614** directional relations. The table has the
same 1,614 relations, with exactly these IDs:

| Contract row | Relations | Raw basis |
| --- | ---: | ---: |
| SC-017l | 134 | failed-server `--all` status changes |
| SC-017m | 150 | default-view and selector-missing unknown membership |
| SC-017o | 573 | completeness JSON and incomplete human diagnostic |
| SC-017r | 78 | human unknown agent-health presentation |
| SC-509b | 14 | 20 fixture-proven loss sessions coalesced by JSON field |
| SC-509c | 222 | 128 self-declared state + 94 current target-named alert carriers |
| SC-509d | 401 | every P1 JSON digest version bump |
| SC-509e | 42 | nullable agent liveness on applicable digests |

Forward direction: every independently selected P1 directional locus is in
`OBLIGATIONS.tsv`. Reverse direction: every table locus is independently
selected, and every table ID is one of the eight IDs above. SC-509c compares
at its full session-and-agent key, so neither distinct-agent collapse nor
same-output-field multiplicity can manufacture agreement. An omitted locus
and an added orphan ID both make the verifier fail.

## P1 applicability and zero-locus dispositions

The companion TSV records every P1-applicable output row and each source
carrier that reaches one. `directional corpus locus` rows map to the table.
`retained corpus locus` rows have real P1 output loci but no directional table
entry: phase-4 criterion 8 compares their non-open residue exactly.
`input carrier` rows have no separate output grain of their own; their visible
effects are owned by the directional obligation IDs listed in the TSV. Their
zero count means no *additional* output key, not an unaccounted coverage gap.
`partial corpus locus` records the captured portion and names its successor for
the unobserved portion. Every `directional gap` has its named, pinned successor
criterion in the TSV; zero is never silently treated as coverage.

The retained list/ls rows SC-017a-i, SC-021, SC-506, and SC-1306a; the
requests/events snapshot rows SC-518, SC-1306d, and SC-1306e; and the SC-509
shape are explicitly present in the TSV. They have corpus output loci and
phase-4 criterion 8 compares their non-open residue exactly, so they are not
silent zeroes. SC-017j/k, SC-017n, SC-400d, SC-521c, SC-017p, and SC-017s are
the directional zero-locus gaps. Their successor gates test respectively
discovery/classification, presentation order, the second durable root,
unknown-plus-filter combinations, and the product-valid pane association
matrix. SC-017q is partial: its captured unknown output is carried by
SC-017r/SC-509e, but the association matrix remains pinned to criterion 12.
SC-508 is not listed: it remains an unclassified code-observation row and
therefore is not a ratified output obligation. SC-517b/c and SC-703/704 have
no P1 corpus surface.

## SC-509b/c source and exclusion reading

The SC-509b row is distinct from phase-2 criterion 13's open choice about
emitting `false`: that criterion does not name this locus. The contract's
fixed `degraded: true` requirement is selected from raw actual loss only.

The SC-509c verification independently finds the committed 222-row population
with no forward or reverse differences: 128 self-declared-state relations and
94 current target-alert relations. The `twda1` dead-agent alert is included.
It separately confirms no agent address carries both carrier kinds.

`SC-509C-UNPROVED.tsv` reports 184 no-carrier-found exclusions. Raw inspection
now carries the same session-and-agent address as the table. The verifier
checks its header, 184-row population, ruled-grain uniqueness, and disjointness
from the independently selected carrier set. An exclusion claiming an address
with a raw carrier is RED; absence is reported, never laundered into coverage.

## SC-509 `generated_at` (ruled requirement)

`generated_at` is an **underdetermined VALUE locus carried by SC-509**. SC-509
requires the field in every JSON digest; it never fixes a particular timestamp
value. It therefore has 401 retained JSON field loci but zero directional table
loci. This is not an orphan and not permission to drop or rename the field:
phase-4 criterion 8 retains required facts while excluding only the registered
value locus, and criterion 18 replays it. The value treatment is independently
confirmed by phase-2 criterion 17's timestamp normalization and phase-3
criterion 3's opposed-clock non-effect control. The pinned successor criteria
are recorded in the TSV.

## Reproduction

```sh
python3 docs/migration/evidence/verify-contract-obligation-reconciliation.py
python3 docs/migration/evidence/redproof-contract-obligation-reconciliation.py
```

Both commands are read-only with respect to tracked files. The red proof makes
each seed only in a new temporary directory, verifies the mutation landed
there before inspecting the verifier result, and deletes that directory on
exit.
