# Phase-4 Run 2 — preflight stop record

## Status

Stopped at preflight before any successor invocation. This is an auditable stop,
not a zero-result run: the criterion-14 binding rule makes every materialisable
CLI row a `FIXTURE-ABORT` before it may invoke the pre-slice successor.

## Fixed identities at the stop

| input | committed identity |
|---|---|
| product successor | `acb4f540e9d7fb0d5a70880f7aec883ffccb36bd` |
| corpus root SHA-256 | `802c882bca64453e33efce5351e43b5954ddecc3daed6c2b0b6c8833487b4e12` |
| contract / invocation / obligation | `896d08ea3ac753095c04af17dfba92cd9d15fb38` / `035c5fab48cf04229daa9285457922d90563fabe` / `44e06c29cc078e6933298139d204413966419d81` |
| accepted P1 / P2 / P3 gates | `8e3c9ec0b031f4947260d4e0327bad562a10fdcd` / `29db943aa85319534301332052105ba16df03b4d` / `8cccbe44787d4ea6007ad9cf9d1cc83a3d03936c` |
| register / comparison projection / health manifest | `2da4fb86933a6b8edee15fd61596d6f53fa6c550` / `c15087aa57a4f24e4ca773df6cafb60097492454` / `6927a58b30d0583def63fe491248b695b1b6f754` |
| published fingerprints / verifier / red proof | `ad3dbb5d02df7d6879ff4536002496b1492862de` / `54dcc46251c5ea128e556b42cf309e123622869c` / `1f604a4b3e75d00847e271d0188e46286fe1cdd2` |
| C3 / verifier / red proof | `6bf2e7f86c82ba15eb8479cff3b139ce708f15bd` / `4cc3eac1d4624062937bc86e65a57c889d6c5a30` / `12d9af2ad01bf0ca73c9257a82afacd73397869c` |
| C8 / verifier / red proof / occurrence table | `a2232860608455e87cde22b3f37faf61084cc3c0` / `c78a1e84802551f167d49443f9ce08bd1cc90336` / `6d66cd98cdeea9aeca6a2bb37e3f1ea63f90d19e` / `29c80d6bcd40b27d726157791abb6919655fd479` |
| phase-4 gate | `f31ece2ac40ed47077ab07f559ad8ab5ad97f6b0` |

`verify-corpus.py`, `verify-invocations.py`, `verify-obligations.py`,
`verify-contract-obligation-reconciliation.py`,
`verify-open-choice-reconciliation.py`, and
`verify-published-fingerprints.py` all returned zero against these committed
bytes during runner preparation. No product source was changed or invoked.

## C14 no-mutation-manifest census

`PREFLIGHT-C14-MANIFEST-SHAPES.tsv` covers every one of the 70 members of the
committed published-fingerprint artifact. For each member, the census:

1. reads its `entry_count` (tracked leaf count) from the published artifact;
2. finds every corpus `case.txt` whose `template=GROUP/MEMBER` resolves to that
   published member and has a sibling `manifest.before.tsv`;
3. counts the tab-delimited manifest records without filtering; and
4. classifies a member as `PUBLISHED-SHAPED` only when every such count equals
   the published count, `STORE-SHAPED` when every count is greater, or
   `MIXED-SHAPE` otherwise. A member with no recorded manifest is named rather
   than treated as a match.

Result: **61 STORE-SHAPED, 0 PUBLISHED-SHAPED, 0 MIXED-SHAPE, and 9 with no
recorded manifest.** The 61 store-shaped members have 162 recorded manifests.
The same census restricted to the P1 execution population finds 143
materialisable case directories and **0/143** whose recorded pre-manifest
equals its materialised published member under the recorded state-manifest
grammar.

G1/healthy is an exemplar, not a special case: its published artifact records
6 tracked leaves (9 entries when the state grammar includes its directories),
while each of its 9 recorded pre-manifests contains 42 entries. The discrepancy
is generated helper/store state absent from the published projection.

This is the clone-fingerprint referent defect's sibling: the historical
no-mutation expectation is store-shaped, while criterion 14 correctly
materialises the published projection. It is a test/fixture defect, never a
product failure. The historical values are inapplicable—not 143 replacement
hashes waiting to be authored. The repaired expectation is a complete,
post-permission scratch manifest recorded before each invocation and compared
exactly after it. A case-specific fixed transformation not represented by the
published member plus its bound environment and permission policy is a new
unmapped input that must surface; that temporal pre-snapshot must never absorb
it as an expectation.

## Would-be accounting, deliberately not executed

The exact P1 population remains 1,065 rows: 743 `ae list`, 116 `ae ls`, 168
`helper:requests`, and 38 `helper:events-tail`.

- 844 materialisable CLI rows would be `FIXTURE-ABORT` for the store-shaped
  no-mutation expectation, before successor invocation.
- 206 helper rows would be named `NOT-EXECUTED` because the pinned Rust
  successor implements neither helper surface. They are skips, not executions.
- 15 live/no-fixed-fixture rows would separately be `FIXTURE-ABORT` for the
  absence of materialisable bytes.

Thus the prospective terminal run-state census is 859 `FIXTURE-ABORT` (844
store-shaped plus 15 live) and 206 `NOT-EXECUTED`; executed child count is
zero. No result, comparison, or obligation row was fabricated for this stop.

## Restart condition

Run 2 resumes only after the ruled batch lands: the cascade table, member-4
projection and C14 pre/post gate wording, C3 rerun, and C8 rebind. It will use
a fresh clean isolated worktree at the same pre-product-slice successor, then
re-pin every C1 input before invoking.
