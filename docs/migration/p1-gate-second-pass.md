# Second pass — the phase-2 and phase-1 gate text that has not had two independent reads

**By `opus5:lexec`, without reading `src/`.** Subjects, identities verified with `git cat-file`
before reading rather than taken from transcription: phase-2 blob
`91d580b48b6ecec0e830ccc537ad32adb2e62b8e` and phase-1 blob
`8e3c9ec0b031f4947260d4e0327bad562a10fdcd`, both at `HEAD`. Read as a **diff against the phase-2
blob I already critiqued** (`999a7469`), so this pass covers new text rather than re-reading closed
ground.

**Criterion 13 not re-attacked**, per instruction: the serializer boundary is closed, and by the
stronger repair rather than either option I offered — both boundaries observed, flip tests required
at each, and an explicit FAIL for a serializer that drops or forces degradation specifically for
`unknown`.

---

## Finding 1 — SC-017o's universal obligation is checked only on a non-universal fixture *(admits a wrong implementation)*

> **SC-017o:** "Every successor JSON digest emits top-level boolean `inventory_complete`; `true`
> means zero required enumeration losses, `false` means one or more. **It is present even for an
> empty inventory.**"

That is a universal obligation over every successor digest. **`inventory_complete` appears exactly
once in the entire phase-2 gate** — criterion 23, line 245 — and criterion 16, which owns "every
successor digest is schema version 2" and emits six document shapes (empty inventory, running only,
stopped only, unknown only, mixed, the degradation matrix), **does not mention the field at all.**
Its observable remains `schema_version: 2` exactly once plus the closed status domain.

**Passing-but-wrong:** a serializer that emits `inventory_complete` on the paths criterion 23
exercises and omits it elsewhere. 16 emits those six shapes and does not look. 23 is satisfied by
its own arms. 17 compares version-1 baseline semantics and FAILs only on fields "silently
drop[ped]/renamed" — a **new** field absent from the baseline is neither.

**This is the same shape as the finding closed at criterion 13**, one field over: *the criterion
that emits the documents is not the criterion that inspects them.* The repair there was to require
observation at both boundaries; here the repair is one clause in 16 — the field is present in every
document 16 emits, and its value matches the fixture's completeness.

## Finding 2 — the phase-3 consumer of the shared loss fixture discriminates COUNT but not IDENTITY *(answers the second question directly)*

The two-source fixture is consumed in three places, and it does distinguish two losses from one in
all three. **But not identically**, and the weakest consumption is the one furthest from the other
two:

| consumer | requirement | distinguishes 2 from 1 | distinguishes 2 DISTINCT from 1 DOUBLE-COUNTED |
|---|---|---|---|
| phase-1 criterion 24 | "two **distinguishable** logical-source loss facts, not a boolean or first-loss-only record" | yes | **yes** |
| phase-2 criterion 23 | "both **distinguishable** loss facts to cross classification; retaining only the first is a failure" | yes | **yes** |
| phase-3 handoff | "require the reported **count** to be `2`, so a boolean or constant-one diagnostic fails" | yes | **no** |

Phase 3 checks the number, not the identities behind it. A loss carrier that counted per *path
attempted* rather than per *logical source* — which SC-017o forbids explicitly, "one failed
worktrees-root enumeration contributes one loss however many unknown subtrees it may contain" —
would report `2` for a single doubled source and pass the phase-3 diagnostic. Phases 1 and 2 catch
it; phase 3 in isolation does not.

That matters because the shared fixture's purpose is that each consumption is an independent check.
**One clause fixes it**: require the two reported losses to name distinguishable logical sources,
not merely to total two.

## Finding 3 — `inventory_complete` under JSON filters belongs to no phase *(scope seam, unproven)*

Criterion 23 exercises the digest **unfiltered**. The phase-3 handoff opens "Run every **human**
list filter/view over one fixed incomplete snapshot" and its FAIL clause is about warnings and rows.
Neither covers whether `inventory_complete` survives `--all --json`, `--stopped --json`, and the
rest. SC-017o places the JSON boolean and the human diagnostic under one "user visibility is
MANDATORY" heading; the gates split them by surface, and the JSON-under-filters cell falls between.
Ranked unproven rather than wrong: SC-017m owns filtering, so this may be deliberate — but if it is,
nothing says so.

## Finding 4 — criterion 23's isolation clause is ambiguous in a way that makes it either trivial or unsatisfiable *(authoring hazard, unproven)*

> "Feed the same candidate set, selectors, read-loss facts, and backend answers twice. **The only
> difference is a phase-1 completeness fact.**"

Two readings. If phase 2 is fed **synthetic phase-1 output**, this is exact and easy — flip one flag
between two otherwise identical inputs. If it is fed from a **real fixture**, the natural way to
produce incompleteness is to fail a source, and failing a source also removes whatever it would have
contributed — so the two arms differ in candidate set, and "the only difference" is violated. It is
satisfiable only by failing a source whose successful enumeration would have yielded nothing, which
is a distinction criterion 24's own vocabulary already makes ("readable empty source... remain
complete and add no loss") and which 23 does not state.

Not a gap in the obligation — a hazard for whoever builds the fixture, where one reading is
unsatisfiable and the other is trivial. One clause naming which.

## On the class the lead asked me to sweep for

**The "precondition its own fixture makes underivable" class does not appear anywhere else I can
find**, and I checked the places most likely to hide it:

- **Criterion 24's (c) arm** — an entitled tmux server that cannot be enumerated — needs entitlement
  to be derivable. Its healthy source is durable, so a positive selector is readable. Consistent.
- **Criterion 24's fourth arm** — unlistable worktrees root with a healthy canonical candidate.
  Canonical supplies both the candidate and its selector. Consistent.
- **The "live server outside the entitled set" control** requires a non-empty entitled set for
  "outside" to discriminate; phase-1 criterion 19 establishes the ambient server before inventory
  runs, so the set is never empty and the control is never vacuous. Consistent.
- **Criterion 23's empty-set repeat** with a two-source failure: ambient entitlement does not derive
  from durable state, so an incomplete-empty snapshot remains constructible. Consistent.

The combined arm was the only place where the fixture's own destruction of the durable roots removed
the entitlement its healthy source needed, and it is fixed.
