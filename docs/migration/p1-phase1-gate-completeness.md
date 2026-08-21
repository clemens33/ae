# Completeness critique of the SC-017j phase-1 gate

**By `opus5:lexec`, without reading `src/inventory.rs` or any phase-1 implementation.**
Disclosure: I have previously read `src/lib.rs:105-115` — the `NO_SESSION_SOURCE` constant and its
doc comment — while verifying an unrelated sequencing claim. That is the *refusal* that predates
phase 1, not an implementation. Nothing else in `src/` has been read.

**This critiques COMPLETENESS, not correctness.** Each criterion is well-formed; the question is
what a wrong implementation could do while passing all nineteen. The gate is a forward list, and a
forward list is structurally blind to what it does not enumerate.

**Verdict: the gate is not complete.** Three gaps admit a wrong implementation; three leave an
obligation unproven; three are over-reach that could fail a correct one. The over-reach group
includes one internal conflict between two criteria.

---

## A. Gaps that admit a WRONG implementation (urgent)

### A1 — The union is never asserted to be a union *(category 1: no criterion)*

> "The list inventory is the **union** of (a) durable current-session state … and (b) positively
> identified ae-owned live tmux sessions on a server the product is already entitled to query."

No criterion tests the case where **both** sources yield the **same** identity. Criterion 6 forbids
merging *distinct* identities under a basename; criterion 18 makes "same-identity multi-source
merge representation" an *OPEN CHOICE*; criterion 14 compares a *projection* onto planted durable
identities, which a duplicate survives.

**Passing-but-wrong:** an implementation that adds a durable candidate for `mdk` and, separately, a
live-only candidate for the same `mdk` on the entitled server, emitting **two candidates for one
identity**. It passes 2, 5, 6, 14 and 18. A union contains each member once; this does not. Phase 2
would then classify one identity twice, and the duplicate is invisible to every criterion because
each is individually well-formed.

**Fix shape:** a criterion where one identity is *both* durable and live-and-marked, with the
observable being **exactly one candidate carrying both provenances**.

### A2 — The `degraded` separation is stated by the row and enforced nowhere *(category 1)*

> "A positively live tmux-only candidate remains visible; **loss of its durable record is the
> separate SC-509b `degraded` fact**."

Criterion 5 covers *visibility* of the tmux-only candidate. Nothing covers the *separation*. Worse,
criterion 4 explicitly declines to constrain the adjacent half — *"OPEN CHOICE: how the read
failure is carried forward for later degradation"* — so both provenances are unconstrained at once.

**Passing-but-wrong:** an implementation that carries a single boolean, "no readable meta", set for
**both** the tmux-only candidate (which never had a durable record) and the unreadable-meta
candidate (whose record exists and could not be read). Every criterion passes. But SC-017j names
these as different facts, and SC-509b's `degraded` is read/parse loss specifically — so phase 2
cannot distinguish them, and `degraded` becomes unimplementable without re-reading the filesystem.
The gate would have certified the phase that destroyed the distinction.

**Fix shape:** a criterion asserting the two carry **distinguishable provenance** at the phase-1
boundary, without dictating representation.

### A3 — A temporal obligation is checked by a static no-status diff *(category 3: output for behaviour)*

> "**inventory candidates exist before liveness is classified**" (the row's title, and its
> operative claim)

Criterion 16 is a **diff check** that no `Running`/`Stopped`/`Unknown` is assigned in inventory.
Criterion 15 forbids gating durable *inclusion* on query success. Neither forbids **constructing
candidates from a liveness result** without assigning a status.

**Passing-but-wrong:** inventory enumerates the entitled servers first, builds a live-name map, and
constructs its candidate set **from that map** — live names first, durable directories appended
afterwards. No status value is ever assigned, no durable candidate is dropped, so 15 and 16 both
pass. The phase ordering the row is *named for* is inverted, and the seam criterion 1 requires
would observe a correctly-populated collection built in the wrong order. Ordering is not visible in
a snapshot of the result.

**Fix shape:** the seam of criterion 1 needs an ordering observable — that the durable candidate
set is complete **before** any server enumeration occurs — not merely a post-hoc set.

---

## B. Obligations left unproven (not urgent)

### B1 — Criterion 2 is output-only, so the legacy root can be satisfied by special-casing

> "(a) durable current-session state under the canonical sessions root **plus SC-400a's
> legacy-readable worktree layout**"

Criterion 2 observes that both identities appear. It does not constrain *how* the legacy layout is
discovered. An implementation applying a narrower rule to the legacy root — one that happens to
admit the single fixture — passes, and fails on a real legacy tree. SC-400a describes a *layout*,
not a directory, so one fixture is weak evidence. Unproven rather than wrong: the criterion is
satisfiable correctly.

### B2 — Which positive ownership evidence counts is never pinned

> "(b) **positively identified** ae-owned live tmux sessions"

Criterion 5 requires the marked session to appear and the unmarked control to be absent; criterion
15 says phase 1 *may* read `AE_SESSION`. Permissive, not mandatory. An implementation keying on a
different signal that happens to co-vary in the fixture passes. SC-017j does not name the marker
either, so this may be a contract gap rather than a gate gap — worth a seat deciding which.

### B3 — "Inert" is enforced as "absent from output", not as "untouched"

> "**Archives are inert** and never enter this inventory."

Criterion 3's observables are the candidate collection and the queried-server trace. Criterion 13's
instrumentation would catch archive-root *enumeration* if its "access limited to canonical durable
roots" clause is read broadly — but 13 is titled and scoped as a socket sweep, so that coverage is
incidental. Nothing forbids inventory *writing* to the archive (a lock, an access stamp). Low
severity, and arguably SC-017's read-side property rather than j's.

---

## C. Over-reach — could fail a CORRECT implementation (category 4)

### C1 — Criteria 3 and 10 conflict on whether the query trace is a set or a sequence

- Criterion 3: "queried-server trace **identical to baseline**"
- Criterion 10: "*OPEN CHOICE:* query grouping/caching — **do not assert an exact query count**"

If "trace" means the **set** of servers contacted, the two agree. If it includes count, order or
batching, they contradict: an implementation whose batching legitimately varies with the number of
meta records present would **fail criterion 3 while satisfying criterion 10's explicit permission**.
This is an internal inconsistency, and it is the failure mode nobody looks for — a correct
implementation rejected. **One word fixes it**: "the *set* of queried servers is identical to
baseline."

### C2 — Criterion 16 forbids a representation that criterion 16 also declares an open choice

Criterion 16 FAILs on constructing "a rendered `SessionEntry` merely to carry a candidate", while
its own *OPEN CHOICE* is "candidate representation and which raw facts/provenance it carries".

Whether this over-reaches **depends on a fact I am barred from checking**: if `SessionEntry` cannot
be constructed without a status value, 16 is sound and its FAIL clause is just the no-status rule
restated. If status is optional or absent on that type, then reusing it as a carrier violates no
SC-017j obligation and 16 rejects a correct implementation. **Reported conditionally**, since
resolving it requires reading the type — which the assignment forbids and which would contaminate
the rest of this critique.

### C3 — Criterion 10 mandates exhaustive discovery that SC-017j does not state

> "(b) positively identified ae-owned live tmux sessions **on a server the product is already
> entitled to query**"

That clause makes entitlement a **precondition** for discovery. It does not say every entitled
server **must** be discovered from. Criterion 10 FAILs "if B is not queried", which imposes
exhaustive discovery across the entitled set.

An implementation discovering live-only sessions on the ambient server, and using B's entitlement
only for phase-2 liveness, is arguably conformant to the row as written and fails the gate. **This
is the gate resolving a genuine contract ambiguity by fiat.** That may be the right policy — but it
is normative policy being set in an acceptance criterion rather than in a row, which is the thing
the gate is otherwise scrupulous about. A seat should either amend SC-017j or record that criterion
10 narrows it.

---

## What I did not find

No gap in the **non-deletion** obligations. "A failed liveness query, a prefix-only name match, or a
live exact-name session whose ownership marker is missing cannot delete the candidate" is covered
by criteria 7, 8 and 9 respectively, each with the correct orientation — and criterion 8's
insistence on "short-dead/long-live" with "mere co-occurrence is an invalid test" is precisely the
asymmetry an earlier measurement of mine got wrong. Criterion 14 then re-tests all three as a set
invariance, which is the right shape: an obligation about *survival* is checked by a projection
that must not move.
