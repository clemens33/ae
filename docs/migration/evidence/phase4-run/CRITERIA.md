# Phase-4 criteria — findings-first

Runner: `grok46:txreview`. Successor at this scoring pass: `e2d11da5` (evidence
only vs `24c66612`; product bytes unchanged). Frozen inputs unchanged (C1
re-checked). Report, not repair. C8 recon not amended.

**Divergence is not failure.** SC-017l/m and SC-509d/b require the successor
*not* to match the defective bash capture. Matching the incumbent would FAIL
those OBSERVED loci.

Verdicts: MET / NOT MET / PARTIAL / INCONCLUSIVE / UNMEETABLE-AS-WRITTEN /
CONSUMED. Findings ranked **product-defect** / **test-defect** / **criterion-defect**.
FIXTURE-ABORT is not a product fail.

## Findings

1. **product-defect — SC-509c (102 OBSERVED FAIL).** Successor JSON carries
   `agents[].reason` as JSON null (or omits the session under some filters)
   where the table requires `dead` / `stale` / `throttled` / `blocked` /
   `waiting-user` from self-declared state or target-named alerts. Example:
   `list --all --json` on A2 c01-filters-ro includes `twda1` with
   `fake:probe` `alive: null` and no `reason`, want `dead`. 120 of 222
   OBSERVED SC-509c loci PASS; 102 FAIL. Not a bash-match error: the
   incumbent also lacked the successor field; the obligation is the
   mandated `to`.

2. **test-defect — SC-017o on meta-mode-000 / meta-absent (28 OBSERVED FAIL).**
   All 28 FAILs are unreadable-meta or absent-meta cases. Successor emits
   `inventory_complete: true` and empty human stderr, *and* SC-509b
   `degraded: true` (14/14 PASS). SC-017o says discovered-candidate
   meta unreadable is SC-509b record-loss, not enumeration incompleteness.
   The obligation table asks `inventory_complete: false` and a human
   diagnostic here. Product matches the contract row; the table over-assigns
   SC-017o. Not repaired.

3. **test-defect — criterion 14 clone fingerprints (0/844).** Frozen
   `fixture-bytes` do not reproduce `case.txt` `clone_fingerprint` (G1/healthy
   on-disk `2f1b2ea3…` vs recorded protected `c940ecaed0…`). Product still ran
   on the frozen fixture-bytes. Fingerprint miss is FIXTURE/test, not product.

4. **product PASS (mandated divergence) — SC-509d 395/395 executed, SC-017l
   14/14, SC-017m 30/30, SC-509b 14/14.** Schema 2, unknown not omitted,
   degraded-true after actual meta loss. Matching bash would have failed
   these.

## Per-criterion

| # | verdict | evidence |
|---|---|---|
| 1 | **MET** | `RUN-MANIFEST.txt`; pre+post verifiers rc=0; identities listed in HANDBACK; C13 calibrate before pins |
| 2 | **MET** | `results.tsv` 1065; surfaces 743/116/168/38 |
| 3 | **CONSUMED** | C3 blob `343fcd80` + loci `a555379f`; `verify-c3` rc=0 |
| 4 | **PARTIAL** | `verify-obligations.py` rc=0. Stock `redproof-obligations.py` mutates the tracked table; not run against freeze inputs. **criterion-adjacent test-defect** of that script's isolation, not of C4's demand |
| 5 | **PARTIAL** | UNSCORABLE 665 preserved in `obligation-scores.tsv`. Mixed-row mutation not executed |
| 6 | **MET** | SC-017m OBSERVED 30/30 present-unknown, including the selector-missing live-socket set named by the criterion |
| 7 | **NOT MET** | OBSERVED 949: PASS 813, FAIL 130, FIXTURE-ABORT 6. FAIL = finding 1+2. ABORT = 6 live json rows with no capture |
| 8 | **PARTIAL** | Register recon consumed (`7bab671b`). Human specimen `0001-list` locator −2 selects `unknown` with state in −1. Three-value in-process calibration **INCONCLUSIVE** this pass (not fed through `table()` for alive/dead/unknown). JSON-warning/machine-loss cross-arms not run. Comparator does not declare extra open choices |
| 9 | **PARTIAL** | 395/401 digest captures: exactly one `schema_version: 2` and boolean `inventory_complete`. 6 live uncloned = FIXTURE-ABORT. Completeness *value* scored under C7/SC-017o |
| 10 | **NOT RUN** | multi-obligation mutation |
| 11 | **NOT RUN** | no product-valid live arm |
| 12 | **REPORTED** | 15 live rows named; pane matrix pending; 78 SC-017r remain UNSCORABLE; not laundered into coverage |
| 13 | **MET** | no frozen binary/helper; calibrate in `exec.log` |
| 14 | **NOT MET** | finding 3; clones from frozen fixture-bytes anyway |
| 15 | **MET** | `verify-invocations.py` both arms |
| 16 | **PARTIAL** | 1065 named rows. Per-obligation vector in `obligation-scores.tsv`. Full 4,260 comparison-locus set not emitted |
| 17 | **UNMEETABLE-AS-WRITTEN this seat** | this seat authored `runner.py`; independent mutation must come from another seat (lexec). Not self-served |
| 18 | **NOT RUN** | paired/reversed replay |
| 19 | **MET** | did not reopen P1–3; did not amend C8; did not turn open choices into required bytes |

## OBSERVED obligation totals

```
SC-509d  PASS=395 FAIL=0   ABORT=6
SC-017o  PASS=240 FAIL=28  ABORT=0
SC-509c  PASS=120 FAIL=102 ABORT=0
SC-017m  PASS=30  FAIL=0   ABORT=0
SC-017l  PASS=14  FAIL=0   ABORT=0
SC-509b  PASS=14  FAIL=0   ABORT=0
UNSCORABLE preserved: SC-017r 78, SC-509e 42, plus the rest of 665
```

## Four answers (updated)

1. **Scorable parity:** not passed (C7 FAIL 130, mostly SC-509c).
2. **Partial/unscorable:** 665 UNSCORABLE untouched; 6 FIXTURE-ABORT on live json.
3. **Closers:** SC-509c product work; C11 live arms; C12 pane matrix; fixture-bytes vs clone_fingerprint (test).
4. **Unimplemented/pending:** helper CLI; SC-017p/q/s matrix; C8 three-value calibration; C17 foreign mutation; C10/C18.
