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

---

## Re-derivation against the agent-liveness rows (contract `01353d8c`)

The freshness gate fired for real: `derived against contract blob a535f8ca69f8; HEAD is 01353d8cdbdb — re-derive`. Re-derived, and the table now carries **1362 obligations** (was 1242):

| row | obligations | what it mandates |
|---|---|---|
| SC-509d | 401 | `schema_version` 1 → 2 |
| SC-017o | 573 | `inventory_complete` on every digest; stderr diagnostic on incomplete human rows |
| SC-017l | 134 | `sessions[].status` `stopped` → `unknown` |
| SC-017m | 134 | membership: absent rows become present as `unknown` |
| **SC-509e** | **42** | `agents[].alive` → `null` on unreachable digests |
| **SC-017r** | **78** | agent health marker: blank → an unambiguous `unknown` |

Verdict counts are unchanged at **573 / 492** — the new obligations attach to rows already
divergent, which is what a reason-projection repair should do.

**SC-017q's matrix is an implication, not an orthogonality**, and that is what makes the agent
obligations derivable: *session `unknown` implies agent `unknown`*. So wherever a session diverges
to `unknown`, every roster agent's health diverges with it — **including agents captured as
`alive:true`**, which is why SC-509e carries two `from` values (`true` and `false`) rather than one.
The two surfaces move in opposite directions from the same frozen defect, so they are two
obligations rather than one.

---

## ⚠ THE FINDING BELOW WAS INVERTED — colead reproduced the count and refuted the label

**Retained with the error visible, per standing practice.** I reported that roughly half the corpus
may be scored `EXPECTED-MATCH` when it should diverge — an UNDER-claim. The truth is the opposite:
**the table asserted a direction on records where the decisive fact was never observed.** It was
OVER-claiming.

**The normative correction, which I had wrong:** *staleness is not itself a liveness result.* I
treated a stale recorded server as routing to `unknown` by construction. It does not. A positive
selector NAMES the server that must be queried and **the exact query outcome decides** — failure or
unreachability gives `unknown`, a success proving absence gives `stopped`, a success with exact name
and ownership gives `running`. SC-017k/l already settled this; I read a gap into a rule that had
none.

**My class A was mislabelled and I had called it the solid one.** Verified on my own specimen:
`arms/A1/c01-healthy-ro` has `AE_TMUX_SERVER=/tmp/aecx/arms/A1/c01-healthy-ro/none.sock` in
`env.txt` — the socket whose failure I recorded — while its session meta records
`tmux_server=/tmp/aecx/tpl/g1/s.sock`. **The captured failure proves the CASE socket unreachable and
says nothing about the recorded server.** The class established "by a positive marker in the
artifact" was established by a marker about the *wrong server*.

**A third silent narrowing, mine, one level deeper than the two I had already disclosed:** the
parity universe is **148** P1 cases reachable through `INVOCATIONS.tsv`, not the 177 `case.txt`
files I partitioned. Verified.

**I never read `env.txt`.** It carries `AE_TMUX_SERVER` in 162 cases and is the queried target;
I inferred the queried server from `case.txt`'s `tmux_socket=` instead. The deciding field was in
the corpus the whole time, in a file my method did not open.

---

## Repair applied — obligations now carry SUPPORT, and nothing is subtracted mechanically

Per colead's repair 3. A new `support` column separates *whether the obligation holds* from
*whether this corpus can score it*:

| support | obligations | which |
|---|---|---|
| `OBSERVED` | **697** | SC-509d 401 (schema — independent of liveness); SC-017o 268 (`inventory_complete: false` and the stderr diagnostic — the captured ambient/entitled-server failure is itself a loss); SC-017l 14 + SC-017m 14 (selector `missing` by construction, which routes to `unknown` with no server outcome) |
| `UNSCORABLE` | **665** | SC-017o 305 (the value `true` needs every enumeration proven clean, including recorded servers never queried); SC-017l 120 + SC-017m 120 (need a recorded-server outcome); SC-509e 42 + SC-017r 78 (agent liveness follows the session's) |

**14 + 14 OBSERVED and 120 + 120 UNSCORABLE reproduce colead's independently derived figures
exactly** — they said six absent/unreadable-meta cases can still reach `unknown` through
`selector=missing`, and the remaining 42 cases, 120 plus 120 records, depend on an outcome the
corpus never captured. Two derivations, different methods, same split.

Nothing was deleted: an `UNSCORABLE` obligation still states what must change and why, and says
plainly that this evidence base cannot judge it. Phase 4 must either inject and record a
product-valid recorded-server result per candidate, or carry that locus as unscorable — because a
whole-row `EXPECTED-MATCH` would canonise `stopped`, and `EXPECTED-DIVERGENCE` without a direction
accepts anything.

---

## FINDING AS ORIGINALLY RAISED (inverted; kept for the record)

While deriving, I checked which server the successor would actually query, because SC-017k requires
the answer to come from the candidate's **recorded** server. The corpus's 91 cases partition:

| | cases |
|---|---|
| queried/recorded server **proven unreachable** in the snapshot | 39 |
| case runs **on** the template socket, so recorded == queried | 7 |
| **case socket differs from the recorded server, and that server's state is UNRECORDED** | **45** |

In the third group a session's meta records `tmux_server=/tmp/aecx/tpl/<t>/s.sock` while the case
ran on `/tmp/aecx/arms/<a>/<c>/live.sock`. **Whether that recorded server was reachable during the
run is not captured anywhere** — `tmux.before.txt` observes the case's socket only.

**Why this reaches beyond the new rows.** My existing SC-017l/m obligations key on *case-level*
unreachability. If the recorded server was in fact unreachable in those 45 cases, then under
SC-017k/l their sessions are `unknown` too, and roughly half the corpus is currently scored
`EXPECTED-MATCH` on status when it should diverge.

**I am not re-scoping the table on my own reading**, for two reasons: it would silently change
about half the corpus's verdicts, and the answer turns on what the successor does with a *stale*
recorded server — a contract question rather than a derivation one. It is the third answer again:
not present, not absent, **unobservable** — for 45 of 91 cases.

The obligations emitted above are confined to cases where the server state **is** recorded, so
nothing in the table asserts a fact the corpus cannot support.

