# Which axes the phase-2 evidence varies — re-scoped over the whole evidence base

**By `opus5:lexec`.** Supersedes `p1-phase2-axes.md`, whose three findings were artifacts of a
scope that excluded `src/liveness.rs`. Evidence base as corrected: **`src/liveness.rs`'s test module
(lines 246–1030, 37 functions) plus `tests/it/phase2.rs` (48 tests)**, and the adapter tests in
`src/tmux.rs`'s test module, which I read because criterion 6's obligation turned out to live there.
**No product function body was opened.** Where a test module sits beside product code in one file I
read only from `#[cfg(test)]` onward; I stopped at `src/inventory.rs:215` after reading the
declaration `pub struct QueryFailed;`, which is a type, not a body.

Event axis excluded by instruction.

---

## Verdict: CLEAN on axes

Every axis I could name is varied somewhere, most of them deliberately and with the reason written
down. One control does not exercise the branch its name claims; that is the only finding, and it is
ranked unproven rather than wrong-admitting.

| axis | varied? | where |
|---|---|---|
| status | yes | running / stopped / unknown throughout |
| degradation | yes | c13, flipped independently of status **at both boundaries** |
| selector state | yes | all four SC-405l states — `Positive(Name)`, `Positive(Socket)`, `Missing` ×5, `Ambiguous` ×3 |
| **positive selector KIND** | yes | c3 runs its three opposed cells **twice**, once Name and once Socket, with the reason in the comment: "a classifier that flattened them would pass with one and fail with the other" |
| `MetaRead` | yes | `Parsed` ×6, `Absent` ×2, `Unreadable` ×3 |
| durable layout | yes | `Canonical` and `WorktreeNested`, and c12 pairs them with **distinct paths under one leaf name** — SC-400d's anti-deduplication clause, exercised |
| candidate provenance | yes | durable-only, tmux-only, dual (c9, c10, c2) |
| ownership marker | yes | matching, absent (`None`), **and mismatched** — `sighting(Ambient, "live-unowned", "someone-else")` at :569 |
| server availability | yes | live / down |
| **transport result vs identical payload** | yes, **at the right seam** | not at the classifier, where `QueryFailed` is a unit struct and a failure structurally cannot carry bytes — but in `src/tmux.rs`'s `a_failed_run_is_a_failure_whatever_it_printed`, which loops `""`, `"alpha\nbeta\n"`, `"no server running…"` against `ok=false` |
| ambient vs recorded server | yes | `ServerId::Ambient` used 7× in liveness, 2× in phase2 |
| entitlement | yes | c22, named-by-phase-1 vs unentitled-but-present |
| completeness | yes | c23, and phase2's second arm earns the delta through a real `read_dir` failure |
| schema version | yes | 1 / 2 |
| record snapshot | yes | default in liveness (statuses are its subject), and varied in phase2 via the real reader, which is where degradation is decided |

**Criterion 6 is the one worth explaining rather than ticking.** Its FAIL clause — "partial output
from a failed query supplies positive or negative proof" — is not enforceable at the classifier,
because `Result<Vec<DiscoveredSession>, QueryFailed>` with `pub struct QueryFailed;` makes a failure
incapable of carrying sessions. The obligation is discharged **by the type**, and the risk relocates
to the adapter that decides success from failure — where `src/tmux.rs` tests exactly it, with
plausible session lines behind `ok=false`. That is the correct place for it, and the classifier test
is a restatement rather than the proof.

---

## The one finding — a control that does not exercise the branch it names *(unproven, not wrong-admitting)*

`src/liveness.rs:989`, `a_candidate_whose_server_was_never_asked_is_unknown_rather_than_stopped`:

> "Defensive: if an answer is missing for any reason, the fallback is the one SC-017l mandates.
> `stopped` requires a successful query, and **a query that never happened is not one**."

The fixture is `Backend::new().down(named("never-asked"))` — a server that is **registered and
fails**. That is the failed-query branch, already covered by c3(c), c6 and c7. **A query that never
happened is a different branch**: `Answers::get` returns `Option`, so the classifier can hold *no
entry* for a server, and that is the branch the name and comment describe.

**What it admits:** if the missing-answer arm of `Answers::get` returned `Stopped` rather than
`Unknown`, this test would still pass, because it never reaches that arm.

**Why I rank it unproven rather than urgent:** the missing-answer state looks unreachable by
construction — `gather` enumerates the servers the candidates name — so this is a defensive branch
guarding an internal inconsistency, and a defect there may be unreachable in practice. It is still a
mislabelled control: the assertion passes through a different path than its own comment claims, and
the *stated* purpose is unverified.

**It is also not constructible with this fixture.** `Backend::enumerate` answers `Ok(Vec::new())` for
any unregistered server, so "never asked" cannot be simulated by omission — omission produces a
successful-empty query, which yields `stopped`. Exercising the named branch needs a backend that can
withhold an answer, or a direct test of the fallback.

## One observation on fixture robustness *(not an axis)*

That same fallback — unregistered server → `Ok(Vec::new())` → `stopped` — means **a mistyped server
name in any fixture is silently a successful-empty query**, and a test asserting `stopped` would
pass with the typo. Seven tests use a bare `Backend::new()` where empty-everywhere is the intent, so
the behaviour is load-bearing and should not simply be removed. It is the same family as the hazard
the `phase2.rs` author already documented — "the double answers with the FIRST world it holds for a
server, so a second `live` for the same server is shadowed" — and is worth the same one-line comment
so the next fixture author knows omission is not neutral.
