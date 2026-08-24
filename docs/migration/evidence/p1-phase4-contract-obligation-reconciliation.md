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
`redproof-contract-obligation-reconciliation.py`.

## Method

This check never invokes or imports `obligations.py`, and does not parse its
authority prose. It independently:

1. selects the 1,065 `phase=P1` rows from `INVOCATIONS.tsv` and joins the
   corpus case by the consumer-path dirname;
2. reads command shape from `normalised_argv`, raw `tmux.before.txt`, raw
   `case.txt`, and captured list output/JSON; and
3. constructs the exact `(case, consumer, obligation_id, stream, locus)` set
   required by the contract reading, then compares it in both directions with
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

## Result

The independent set has **1,378** unique loci. The table has the same 1,378
unique loci, with exactly these IDs:

| Contract row | Obligations | Loci |
| --- | ---: | ---: |
| SC-017l | 134 | failed-server `--all` status changes |
| SC-017m | 150 | default-view and selector-missing unknown membership |
| SC-017o | 573 | completeness JSON and incomplete human diagnostic |
| SC-017r | 78 | human unknown agent-health presentation |
| SC-509d | 401 | every P1 JSON digest version bump |
| SC-509e | 42 | nullable agent liveness on applicable digests |

Forward direction: every independently selected P1 directional locus is in
`OBLIGATIONS.tsv`. Reverse direction: every table locus is independently
selected, and every table ID is one of the six IDs above. An omitted locus and
an added orphan ID both make the verifier fail.

## P1 applicability and zero-locus dispositions

The companion TSV records every P1-applicable output row and each source
carrier that reaches one. `directional corpus locus` rows map to the table.
`retained corpus locus` rows have real P1 output loci but no directional table
entry: phase-4 criterion 8 compares their non-open residue exactly.
`input carrier` rows have no separate output grain of their own; their visible
effects are owned by the directional obligation IDs listed in the TSV. Their
zero count means no *additional* output key, not an unaccounted coverage gap.
Every `directional gap` has its named, pinned successor criterion in the TSV;
zero is never silently treated as coverage.

The retained list/ls rows SC-017a-i, SC-021, SC-506, and SC-1306a; the
requests/events snapshot rows SC-518, SC-1306d, and SC-1306e; and the SC-509
shape are explicitly present in the TSV. They have corpus output loci and
phase-4 criterion 8 carries their exact retained facts, so they are not silent
zeroes. SC-017j/k, SC-017n, SC-400d, and SC-521c are the directional
zero-locus gaps. Their successor gates test respectively discovery/
classification, presentation order, the second durable root, and unknown-plus-
filter combinations. SC-508 is not listed: it remains an unclassified code-
observation row and therefore is not a ratified output obligation. SC-517b/c
and SC-703/704 have no P1 corpus surface.

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
