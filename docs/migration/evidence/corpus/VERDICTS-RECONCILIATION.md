# Reconciling VERDICTS.tsv against the contract as it now stands

**By `opus5:lexec`, and deliberately NOT by re-running `verdicts.py`.** That generator produced the
column and has a demonstrated blind spot — it excused 268 rows by keying status divergence on
whether the captured output carried a status field. Re-deriving with it would inherit that. Method
here: read `VERDICTS.tsv` **as data** through its own typed fields, read the contract rows as text,
and cross-check by counting over surfaces rather than by regenerating.

`VERDICTS.tsv` is at `da61fa5b`. Six contract commits have landed since, plus one that **predates**
it and was never named.

---

## 1. Which rows now mandate a divergence that `mandated_by` does not name

### The claim, verified independently: **573 — confirmed**

| obligation | rows | check |
|---|---|---|
| SC-017o adds `inventory_complete` to every successor digest | **401** | every digest-bearing row; 305 healthy (`true`) + 96 unreachable (`false`) |
| SC-017o adds the human stderr diagnostic to incomplete snapshots | **172** | list/ls, unreachable, non-digest |
| **union** | **573** | the two sets are disjoint — digests versus human — and 401 + 172 = 573 = the entire divergence set |

`mandated_by` names SC-017o on **zero** of them. Counts do not move; the **reason projection is
stale across the whole divergence set.**

### My first count was 659, and the difference is the part worth keeping

Counting `server_unreachable == yes` directly gives **354** rows, not 268, and
401 + 258 = **659**. The gap is **86 rows** — 48 `helper:requests` and 38 `helper:events-tail` —
which sit in unreachable *cases* but are neither digests nor `list`/`ls`.

**`server_unreachable` is a CASE-level fact recorded on every row of that case.** Reading it as a
row-level mandate over-counts by 86 and makes the partition look wrong. SC-017o names its two
obligations by surface — "*Every successor JSON digest*" and "*Human `list`/`ls` output*" — so the
86 gain neither and correctly remain `EXPECTED-MATCH`. Anyone verifying this figure from that field
alone will get 659 and conclude the split is broken.

### SC-521c — checked, and it adds nothing HERE, for a coverage reason

SC-521c **predates** the column (`bab1b6fe` is an ancestor of `da61fa5b`), so its absence is a
derivation gap rather than staleness — I keyed on status and digest and never on filters. Under
schema v2 it widens `--needs-attn`/`--active` to match `running` **or** `unknown`, which changes
*which sessions appear*, not their status.

Measured: **231 live-scope filter rows, and all 231 are in healthy cases. Zero pair a live-scope
filter with an unreachable server.** With no `unknown` present the widened domain selects the same
sessions, so no corpus row changes. **SC-521c mandates nothing here — but only because the corpus
never builds the pairing that would trigger it**, which is a coverage fact, not a semantic one. Its
membership semantics are unexercised end to end.

### The rest, checked rather than assumed

- **SC-400d, SC-405l** (`032f0aaa`, `9cf5a2ad`): already reconciled in `P1-SUFFICIENCY.md` §8 — both
  bucket 2, defining *reading*; no emitted value moves.
- **SC-017k** (`eb7dc30b`, a coalesced sighting stays proof): changes no emitted value on any
  captured row. Its effect needs a dual-provenance candidate with a matched sighting; the
  unreachable cases have no server and therefore no sighting.

---

## 2. Can the column support exact directional assertions? **No — expect-divergence only**

The schema is `case, consumer, surface, output_shape, status_bearing, digest_bearing,
server_unreachable, verdict, mandated_by, baseline_provenance`.

A consumer can ask **"does this row differ?"** and **"which rows mandate that it differ?"** It
cannot ask **"differ HOW."** The column carries no *from* value, no *to* value, no field locus, and
no stream.

**Worked example — one unreachable human `list` row now owes three simultaneous obligations:**

| # | locus | from | to | column can check? |
|---|---|---|---|---|
| 1 | the status cell | `stopped` | `unknown` | **no** — only that something differs |
| 2 | stderr | *(absent)* | a diagnostic carrying loss count ≥ 1 | **no** — stderr is not a field it models |
| 3 | `inventory_complete` on the paired digest | *(absent)* | `false` | **no** — though the pairing itself is derivable by `case` |

**So an implementation emitting `status: "gone"`, no stderr, and no `inventory_complete` satisfies
every check this column can support.** It diverges, which is all the column asks. That is the
failure the lead named: *expect-divergence degrades into expect-anything-but-this*, and the
population it fails to judge is exactly the one parity exists for.

---

## 3. The shape it needs (proposed, not built)

**The unit must change from one row per invocation to one row per (invocation, obligation).** A row
owing three obligations cannot be represented by one record however many fields it gains — that is
the structural change, and everything else follows from it.

Proposed `OBLIGATIONS.tsv`, joined to `VERDICTS.tsv` on `(case, consumer)`:

| field | purpose |
|---|---|
| `case`, `consumer` | join key to the existing column |
| `obligation_id` | the contract row that mandates it — `SC-017l`, `SC-017o`, `SC-509d`, `SC-521c` |
| `stream` | `stdout` \| `stderr` \| `digest` — stderr is currently unmodellable |
| `locus` | the addressable thing: `sessions[].status`, `inventory_complete`, `schema_version`, `(whole stream)` |
| `from` | the captured value, quoted from the artifact — `stopped`, or `ABSENT` |
| `to` | the mandated value where fixed — `unknown`, `false`, `2` |
| `predicate` | `equals` \| `matches` \| `at-least` \| `present` — the loss count is `at-least 1`, not `equals` |
| `baseline_provenance` | `OBSERVED` \| `SOURCE`, as now, but per obligation rather than per row |

`verdict` then becomes **derived**: a row is `EXPECTED-DIVERGENCE` iff it has ≥1 obligation. It
stops being a stated fact that can disagree with its own reasons.

### And a freshness relation, which is the part provenance cannot supply

**A derived artifact goes stale the moment its source grows, and nothing re-runs to say so.** This
reconciliation exists because a human noticed. The artifact should carry:

- `contract_rev` — the **blob hash** of `semantic-contract.md` it was derived against, not a date or
  a row list;
- a checker that fails when `HEAD`'s blob differs, naming the rows added or changed in between.

That converts staleness from something discovered in review into something a gate reports. The
lineage stamp says where the artifact came from; only the hash comparison says whether the source
has moved since.

**And the sizing lesson, because it argues for the check being automatic rather than periodic:**
one row landing invalidated the reason projection on **every** divergent row, because SC-017o
reaches `inventory_complete` — a field every digest carries. **The size of an amendment is no guide
to the size of the invalidation it causes**, so "was that a big change?" is not a usable trigger.

---

## Addendum — the freshness relation is HEAD-relative, stated rather than discovered

Built at `940215e` as `FRESHNESS.tsv` plus a gate clause. One property is worth writing down
because it is a deliberate choice with a real cost, not an implementation detail:

**The relation compares the recorded contract blob against `HEAD`, not against the working tree.**
It therefore answers *"is the COMMITTED table fresh against the COMMITTED contract"* — the question
a reviewer or CI asks — and one agent's in-flight contract edit cannot fail everyone else's gate.

**The cost:** someone editing `semantic-contract.md` locally and running the gate gets a pass that
says nothing about their own edit. So the **success** line names what it was fresh against, not just
the failure line:

```
OBLIGATIONS VERIFIED — fresh against COMMITTED contract a535f8ca69f8 at HEAD
  (HEAD-relative: an uncommitted local edit to the contract is NOT assessed)
```

A gate whose green message does not name its reference invites being read as *fresh against what I
just wrote*, which is the one thing it does not check.

