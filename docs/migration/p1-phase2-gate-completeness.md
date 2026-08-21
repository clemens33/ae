# Completeness critique of the SC-017k / SC-017l / SC-509d phase-2 gate

**By `opus5:lexec`, without reading `src/`.** Subject: blob
`999a74698e09463d6766489e0f855b390234a0cd` at commit `b1014167`, 22 criteria — identities
verified against `git cat-file` before reading, not taken from transcription.

**Frame pre-registered before reading, at the lead's request and mine:** for an orthogonal pair
`(X, Y)`, the defect is an implementation computing `X` **from** `Y`. A criteria list testing only
the **diagonal** cells is fully satisfied by that implementation, because the derivation and the
truth agree on the diagonal. The **off-diagonals** discriminate, and they are the cells a fixture
author is least likely to build.

---

## Verdict on the pre-registered frame: ALL THREE PAIRS COVERED

I named three concrete off-diagonals before reading. Each is covered, and two are covered by a
**stronger** construction than the one I proposed.

| pair | my pre-registered off-diagonal | gate |
|---|---|---|
| `unknown` × `degraded` | readable record + unreachable server → `unknown`, **not** degraded | **criterion 13**, and stronger — it builds all four cells *and* adds an independence test my version lacked: "flip degradation alone and require identical query/status; flip liveness proof alone and require identical degradation" |
| selector-knowledge × record-read-knowledge | readable record with **no** selector; readable record with an **ambiguous** selector | **criterion 7**, exactly — three separate `missing` cells (readable / absent / unreadable) plus `ambiguous`, with "the three `missing` cells retain distinct phase-1 record-read facts even though their selector state agrees" |
| source-provenance × liveness-proof | candidate that is **both** durable and live-discovered; durable on a **reachable** server with the name absent | **criteria 10, 3(b), 5, 11** — and my *reverse* derivation is caught by **criterion 9**: "its durable-source provenance remains absent, no durable selector/record is fabricated … a before/after filesystem capture is unchanged" |

**Criterion 13 is the part worth naming.** My attack was cells-only, and cells alone can be
satisfied by coincidence. The flip test — vary one axis, require the other unchanged — tests the
*independence claim itself* rather than four points that happen to agree with it. That is the
correct instrument for an orthogonality obligation and it is better than what I brought.

---

## The one finding: the pair is tested at the classifier, and its EMISSION is tested only where status is running or stopped

**Category: no criterion unambiguously owns it. Ranked as admitting a wrong implementation, at the
digest boundary SC-509d governs.**

Three criteria touch degradation-in-the-digest and none closes the `unknown` case:

- **Criterion 16** *emits* the right fixture — "successor digests for … unknown only; mixed
  statuses; **and the degradation matrix**" — but its observable is only `schema_version: 2`
  exactly once and `sessions[].status` within the closed domain. **It emits the matrix and does
  not look at the degradation field.**
- **Criterion 17** does check SC-509b survival, but is scoped "**for a fixture whose liveness
  remains running/stopped across the flip**" — explicitly excluding `unknown`.
- **Criterion 13** builds the four-cell matrix and mentions JSON once, in an *OPEN CHOICE* about
  "exact JSON presence for false". Its observable is "each status/degradation pair independently",
  which does not say at which boundary.

**Passing-but-wrong:** an implementation that classifies correctly — 13 green — and then, in the
successor serializer, drops or forces the degradation field on sessions whose status is `unknown`.
16 emits those documents and checks only version and status domain. 17 never sees them. 13 is
green because the classification was right. **The orthogonality survives to the classifier and
dies at the emitter**, which is exactly where SC-509d's consumer-visible contract lives.

**This is the pre-registered shape one level down.** The pair is guarded on the axis where both
facts are computed, and unguarded on the axis where they are *rendered together*. If criterion 13's
observable is intended to be the emitted digest, the gap closes and only the wording needs to say
so; if it is the classifier, a criterion is missing. **Either way the fix is one sentence**, and
which one it is depends on an intent only the author can state.

---

## Criterion 22, examined specifically (no prior independent pass)

**Sound.** It mirrors phase-1 criterion 13 correctly, and its FAIL clause carries the sweeper
lesson intact — "even when its result is discarded and every final status is otherwise correct" —
which is the clause that makes an absence-of-output assertion insufficient. Instrumenting both
filesystem enumeration and every backend target is the right pair of observables.

**One observation, ranked unproven rather than wrong.** 22 bounds the entitled set **from above**
(no expansion) and explicitly permits "querying any subset". It therefore does not distinguish an
implementation that **consumes** the phase-1 entitled set from one that **recomputes** entitlement
from the carried selectors and happens to land inside it. Criterion 14 forbids rediscovering
phase-1 facts *by filesystem reread*, which recomputation from carried selectors does not do. A
recomputation that diverged — for instance by not carrying the ambient server, which is entitled
without appearing in any selector — would be caught only if some criterion required an answer from
it. Whether recomputation is permitted at all is not stated.

---

## No over-reach found, and criterion 21 is why

I looked specifically for criteria that could fail a *correct* implementation, which is the
failure mode nobody looks for. I found none — and the reason is structural rather than luck:
**criterion 21 is an explicit anti-over-reach guard that fails the gate itself** — "FAIL this gate
if a test rejects an otherwise correct classifier for one of those unratified choices." Criterion
19's "if no version-selectable successor serializer exists, do not invent one for the test" is the
same instinct applied locally. A gate that can fail itself for being too strict is the first one
I have read that guards both directions by construction.
