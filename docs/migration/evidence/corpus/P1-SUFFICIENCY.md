# Is the corpus sufficient for P1 parity?

Analysis only — nothing built, nothing captured. Answers the four questions put by the seat
ruling, against SC-017j/k/l/m/n and SC-509d (the only contract rows routed to this worker).

## Verdict, first

**The corpus is complete and pinned over a surface that exercises ONE of the four bucket-3
conditions and cannot speak to two others.** It is **not sufficient** for P1 parity.

And the structural problem is **larger than defect-bearing rows**: SC-509d changes
`schema_version` from 1 to 2 for every successor digest, so **every `--json` row diverges whether
or not a defect fires**. Under a uniform byte-parity rule, **553 of the 1065 P1 rows would go RED
precisely when the implementation is CORRECT** — 52%.

## 1. Which P1 rows exercise behaviour a bucket-3 row changes?

**The identifying property is what the output CARRIES, not which case produced it:**

- **carries a session status field** → SC-017j/k/l/m apply. That is exactly `ae list` and `ae ls`:
  **859 rows** (743 + 116).
- **carries the SC-509 machine digest** → SC-509d applies. That is the `--json` subset:
  **401 rows**, all within the 859.
- **carries neither** → untouched: `helper:requests` 168 and `helper:events-tail` 38 = **206
  rows**. Verified rather than assumed: zero `requests`/`events-tail` stdout files contain a status
  field or `schema_version`.

SC-017n is bucket 2, not bucket 3 — it mandates an order the frozen product never applied, so it
is a divergence question for the same 859 rows but not a fix-known-defect one.

## 2. Does the corpus actually contain defect-exercising cases?

**Mixed, and the mix is the finding.** Per condition:

| condition (from the rows) | present? | rows | evidence |
|---|---|---|---|
| **unreachable recorded server** (SC-017l) | **YES** | **228** | 38 cases ran with no server. Verified end to end: session `tg5` records `tmux_server=/tmp/aecx/tpl/g5/s.sock`, that socket is absent, `tmux.before` shows `error connecting`, and bash printed `status stopped` in human **and** `"status":"stopped"` in JSON. SC-017l mandates `unknown`. |
| **live prefix sibling** (SC-017j/k) | **NO — ZERO** | **0** | Prefix pairs exist in fixtures (`tg2b`/`tg2bl`, `tg1`/`tg10`, `tg1`/`tg11`, `ta7g`/`ta7goalorder*`) and co-occur in 11 cases, **but never in the firing orientation**. The defect needs a durable candidate that is NOT live whose name is a strict prefix of a LIVE session's name. Measured across every case: **0 meet it**. In the co-occurring cases the live member is `tg2b` and the non-live member is `tg2bl` — the reverse. |
| **missing ownership marker** (SC-017j) | **UNOBSERVABLE** | — | Ownership is proved by `tmux show-environment -t <name> AE_SESSION` (ae:2682-2693). **The corpus never captured that marker** — `tmux.before.txt` records `session\|pane\|agent\|window\|cmd` from pane options, not the session environment. The corpus cannot answer whether this path was exercised, in either direction. |
| **non-ambient / ambiguous server** (SC-017k/l) | **NO** | 0 | Every case is either one live server or no server. No case queries server X while a session records server Y, which is the distinct condition SC-017k names. |
| **schema version** (SC-509d) | **YES — universal** | **401** | Every captured digest is `"schema_version":1`; the successor emits 2 unconditionally. |

**A correction to my own first measurement, recorded because it changed the answer.** I initially
bucketed 146 rows as "live prefix sibling present" by testing whether a prefix *pair* co-occurred
in a fixture. That is not the firing condition. Testing the actual orientation — non-live
candidate, live longer sibling — the count is **zero**. A co-occurrence test and a firing test
differ by exactly the asymmetry the defect depends on, and the plausible number was 146.

## 3. The row-level partition, and what it means for parity

| class | rows | under uniform byte-parity, a CORRECT Rust would be |
|---|---|---|
| `list`/`ls`, unreachable server, `--json` | 76 | **RED twice** — status `stopped`→`unknown` **and** schema 1→2 |
| `list`/`ls`, unreachable server, human | 152 | **RED** — status `stopped`→`unknown` |
| `list`/`ls`, healthy server, `--json` | 325 | **RED** — schema 1→2 |
| `list`/`ls`, healthy server, human | 306 | green |
| `requests` / `events-tail` | 206 | green |
| **total P1** | **1065** | **553 red, 512 green** |

So the seat reading is confirmed and extended: parity cannot be "match the corpus" uniformly.
It has to be **"match the corpus EXCEPT where a bucket-3 row mandates divergence"**, and those
rows need **expected-divergence** treatment — a recorded statement of *what must differ and how* —
rather than expected-match. The 325 healthy-server JSON rows show that the exception is not a
narrow special case: it is the majority of the machine surface, driven by one enum value.

## 4. What capture would be needed (NOT built)

To close the two conditions the corpus cannot speak to:

1. **Prefix firing.** A fixture with a durable session whose name is a strict prefix of a
   *separately live* session, with the candidate itself not live — the orientation absent from all
   177 cases.
2. **Missing ownership marker**, with the marker *captured*. Two parts, and the second is the one
   that was missed before: a live exact-name session without `AE_SESSION`, **and** a capture step
   that records `show-environment -t <name> AE_SESSION` per live session, so the condition is
   observable in the artifact rather than inferable from its absence.
3. **Non-ambient server.** A session recording server X while the query runs against server Y —
   distinct from "no server", which is what the 38 cases actually are.
4. **Ambiguous recorded server.** Meta whose `tmux_server` is malformed or unresolvable, which
   SC-017l names separately from unreachable.
5. **`unknown` × `degraded` orthogonality.** SC-017l states the two are independent; a case
   carrying both, and cases carrying one each, are needed to capture that independence rather than
   assume it.

Item 2 generalises: **a condition that is never captured cannot later be shown to have been
absent.** The corpus's silence about the ownership marker is not evidence the path was healthy.
