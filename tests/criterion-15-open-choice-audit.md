# Phase-3 criterion 15 audit

Gate: `docs/migration/p1-phase3-gate.md`, live accepted blob
`8cccbe44787d4ea6007ad9cf9d1cc83a3d03936c` (commit `7b70310e` — each choice
retains the surface scope of its owning criterion; this is not a flat
cross-surface union).

The named set, quoted: human table layout, colors, headers, whitespace;
incomplete-human diagnostic wording, path detail, and exit status from
criterion 12; JSON stderr warning policy from criterion 13 while JSON process
rc remains retained; JSON object field order; detailed machine loss records
and their order; internal collection or sort implementation; and equal-name
tie-breaking.

Register: `OC-P3-HUMAN-DIAGNOSTIC` (human stderr and rc, scope
`human_incomplete_observed`) excludes scalar process rc.
`OC-P3-JSON-WARNING` (JSON stderr, scope `digest_all`) excludes warning
presence/wording/path and **STILL_REQUIRED** includes "The JSON document and
process rc".

This file is the mandated record. Every unit, integration, and doctest
assertion in `src/` and `tests/it/` that touches one of those surfaces is listed
with its source location and the required semantic fact it checks. A row
**FAIL**s when an expected value depends on a named open representation or
implementation choice.

`tests/it/phase3.rs` `criterion_15_this_suite_asserts_no_unratified_presentation_choice`
greps a handful of concatenations in that file only. That is not this audit.

Line-references for `tests/it/phase3.rs` and `src/lib.rs` are against
`68448c18ddded25276c99969b51d32544ecec419`. Other files are the holding
working tree (uncommitted C15 owned-file splits).

## Verdict

No remaining FAIL. Rc is split per surface in both `tests/it/cli.rs` criterion 1
and `tests/it/phase3.rs` criterion 12 / `presented`: incomplete-human rc is
unpinned; JSON process rc is asserted `0`; complete-human rc remains asserted
where the world is complete.

No assertion in the suite plants byte-identical session names and asserts their
relative order. No assertion pins a sort algorithm, a collection type, or a
JSON loss-record schema — the machine surface carries the boolean only.

## Remaining FAILs

None.

## Resolved FAILs (this record)

| Location | Surface | Was | Now |
|---|---|---|---|
| `src/digest.rs:338` doctest `Digest::render` | JSON object field order | `starts_with` pinned version as first member | `contains(r#""schema_version":2"#)` |
| `src/digest.rs:403` / `:625` / `src/listing.rs:780` | JSON object field order | whole-document bytes | parsed member set/values via `same_members` |
| `src/digest.rs:536` / `:559` | JSON object field order | ordered key vec / `keys.last() == degraded` | member set / set-difference `{degraded}` |
| `src/json.rs:545` `objects_render_in_field_order` | JSON object field order (comment only) | comment claimed SC-509 documents an order | assertion kept (determinism of `Value::obj`); comment corrected |
| `tests/it/phase3.rs:832` `criterion_14` | JSON object field order | rendered-string replace of `inventory_complete` | `get` plus `json_members_match` on remaining members |
| `tests/it/cli.rs:110` `criterion_1` | incomplete-human vs JSON rc | shared `assert_eq!(code, Some(0))` covering both loop legs, then a wholesale drop of both | **split:** `if json { assert_eq!(code, Some(0)) }`; human leg asserts nothing about rc |
| `tests/it/phase3.rs:664` `criterion_12` / `:692-693` | incomplete-human rc | shared `assert_eq!(code, 0)` on every human view | `if expected.is_none() { assert_eq!(code, 0) }` — complete human only |
| `tests/it/phase3.rs:1304` `presented` | incomplete-human vs JSON rc | human `assert_eq!(code, 0)`; JSON rc discarded | human `let (human, _, _)` with no rc pin; JSON `assert_eq!(json_code, 0)` |

## Method

Surfaces in the named set were searched across `src/**/*.rs` (unit tests and
doctests) and `tests/it/**/*.rs` (the single integration target). Assertions
that never touch list/ls human or JSON presentation — CLI usage `2`,
`--version`, events.jsonl, inventory discovery, tmux transport — are out of
scope even when they mention "exit" or "JSON".

`get` / `get_str` / `contains` of a member name or value do not pin object
field order. `PartialEq` on `json::Value` does, because `Value::Obj` is a
`Vec`. Tests that must not pin order go through `Value::same_members`.

---

## JSON object field order

### Failures found and fixed here

| Location | Was | Semantic fact now checked |
|---|---|---|
| `src/digest.rs:338` doctest on `Digest::render` | `starts_with(r#"{"schema_version":2,"#)` pinned the version as the first member | `contains(r#""schema_version":2"#)` — successor documents carry numeric version 2 |
| `src/digest.rs:402` `sc_509_renders_the_documented_example_field_for_field` | `assert_eq!(rendered, expected)` whole-document bytes | parsed member set and values of commands.md's worked example (`same_members`) |
| `src/digest.rs:626` `an_empty_digest_is_still_a_versioned_document` | whole-document bytes | parsed members: version 2, epoch stamp, empty `sessions`, `inventory_complete: true` |
| `src/listing.rs:780` `sc_509_a_world_with_no_sessions_still_renders_a_complete_document` | whole-document bytes (comment claimed only the ratified half was pinned) | parsed members of a complete empty digest (`same_members`) |
| `src/digest.rs:536` `sc_509b_loss_is_visible_and_sparsity_is_not` | `keys == ["name", "status", "needs_attention", "agents", "degraded"]` (ordered) | `same_members` against that member bag; `degraded` omitted on a sparse entry |
| `src/digest.rs:559` `sc_509b_is_additive_so_a_degraded_entry_keeps_the_other_members` | `keys.last() == Some("degraded")` (position) | key-set difference versus the non-degraded entry is exactly `{degraded}` |

### Kept — determinism of `Value::obj`, not a schema order

| Location | Semantic fact |
|---|---|
| `src/json.rs:545` `objects_render_in_field_order` | `Value::obj` preserves insertion order rather than hashing. Comment previously claimed SC-509 documents an order; SC-509 does not. Assertion kept. |
| `src/json.rs:563` `object_member_equality_ignores_field_order` | `same_members` is order-insensitive while `PartialEq` is not — the helper this audit relies on would be vacuous if both compared equal under `==`. |

### Pass — member lookup, not order

| Location | Semantic fact |
|---|---|
| `src/json.rs:57-58` doctest `Value::obj` | a one-field object renders that field (order is N/A) |
| `src/json.rs:234-235` doctest `parse` | `get_str("action")` reads a member by key |
| `src/json.rs:578-583` `every_scalar_shape_renders` | scalar JSON spellings |
| `src/json.rs:592` `nested_arrays_and_objects_render` | one-field object inside an array |
| `src/json.rs:596` `a_documented_event_line_round_trips` | events.md documented line round-trips, including that schema's documented key order. Not the list-digest open choice. |
| `src/json.rs:681` empty object round-trip | `{}` parses and renders |
| `src/json.rs:692` number round-trip | parse preserves numeric literals; re-render of the parsed object is deterministic |
| `src/listing.rs:257` doctest | `contains(r#""name":"live""#)` / not `"old"` — selection membership |
| `src/listing.rs:410` `sc_509_the_json_rendering_is_the_digest_verbatim_plus_a_newline` | listing JSON stdout is `Digest::render` plus a newline. Both sides share one renderer; no expected member order. |
| `src/listing.rs:428-555` filter/scope tests | `sessions[].name` sequences (array order is ratified C-byte/group order, not object field order) |
| `src/listing.rs:527` `sc_521a_an_empty_intersection_is_still_a_complete_document` | `get("schema_version")`, empty `sessions`, `inventory_complete: true` |
| `src/listing.rs:558` `sc_521b_repeating_a_scope_flag_changes_no_byte_of_the_output` | repeating a scope flag is a no-op (identity of two renders; determinism) |
| `src/listing.rs:460` `sc_017i_running_renders_exactly_what_the_bare_default_renders` | `--running` equals the default, both surfaces |
| `src/digest.rs:460` `sc_509_the_document_is_one_object_carrying_the_version` | `get("schema_version")` / `generated_at` present / one session. Renamed: it no longer claims "first". |
| `src/digest.rs:474-493` attention members | `needs_attention` / `attention` / `attention_rank` / omitted-when-absent |
| `src/digest.rs:496` `sc_506_a_degraded_entry_keeps_its_identity_and_closes_the_document` | three sessions remain, identity of the damaged one, document closes |
| `src/digest.rs:586` `sc_510d_text_in_the_digest_is_escaped_not_pasted` | goal text is escaped; parsed value round-trips |
| `src/digest.rs:607` hostile session name | one `sessions[]` entry, no injection |
| `src/lib.rs:489` `a_wired_list_json_is_the_digest_document` | `get("schema_version") == 2`, `get("inventory_complete") == true` |
| `tests/it/cli.rs:141-153` criterion 1 JSON arm | `contains(r#""schema_version":2"#)` and `contains(r#""inventory_complete":false"#)` — member presence, not position |
| `tests/it/fixtures.rs:566-588` | version, stamp, selected session members via `get` |
| `tests/it/phase2.rs` criterion 16/17/19/15 (phase-2 numbering) | version 2 once, completeness boolean once, status domain, classified status preserved; all via `get` / `contains` / membership |
| `tests/it/phase3.rs:314` `json_members_match` | order-insensitive object compare; arrays stay order-sensitive. Local copy because `Value::same_members` is `pub(crate)` and the integration target cannot call it |
| `tests/it/phase3.rs:748` `criterion_13_every_document_carries_version_2_and_the_completeness_boolean` | `get("schema_version") == 2` exactly once, `get("inventory_complete")` the supplied boolean exactly once |
| `tests/it/phase3.rs:832` `criterion_14_flipping_completeness_changes_only_the_warning_and_the_boolean` | completeness boolean via `get`; remaining members via `json_members_match` after stripping that key |
| `tests/it/phase3.rs:1843` `sc_509e_the_agent_liveness_field_is_present_even_when_null` | `alive` is present as `true` / `false` / `null`; `contains(r#""alive":"#)` is key presence |
| `tests/it/phase3.rs:1928` `sc_017q_an_unknown_agent_keeps_its_declared_state_and_reason` | `alive` null does not drop `state` / `reason` / `session_id` |
| `tests/it/phase3.rs:1959` `sc_017q_the_entry_point_reports_unknown_agents_rather_than_dead_ones` | machine surface says `"alive":null` and not `"alive":false` |

---

## Human table layout, colors, headers, whitespace

No test asserts a header row, a colour sequence, a column width, or the exact
tabular bytes. Isolated-world `contains` checks and identity between two
renderings are the pattern.

| Location | Semantic fact | Notes |
|---|---|---|
| `src/listing.rs:275` doctest | default human listing contains `live` | membership, not layout |
| `src/listing.rs:472` `sc_017f_json_honours_the_filters_and_never_widens_them` | human `render(flags)` equals `table` of the JSON-selected sessions, in that order | both sides share `table`; layout bytes move together. Selection identity, SC-017f |
| `src/listing.rs:569` `sc_017h_a_session_line_carries_the_attn_marker_only_when_it_needs_one` | isolated `table` contains `attn:<reason>` iff attention is set; full listing `attn:` count matches flagged sessions | content, SC-017h/g |
| `src/listing.rs:604` `sc_017h_every_reason_reaches_the_marker_by_its_own_name` | every `Reason` spelling reaches `attn:` | content |
| `src/listing.rs:617` `sc_017h_the_roster_lists_every_agent_and_drops_none` | every agent `reference` is present; `dead` and `alive` both appear in a mixed roster | membership. Health *words* are SC-017r (not this named set) |
| `src/listing.rs:638` `sc_017h_an_agent_that_declared_nothing_is_not_rendered_as_blank` | undeclared state leaves a non-whitespace residue; it is not the empty-string rendering and not `working` | content vs form; explicitly unpins glyph and separators |
| `src/listing.rs:676` `sc_017h_the_agent_level_reason_is_not_the_session_marker` | toggling the agent's own reason changes no byte; exactly one `attn:` (the session's) | semantic rule, not a line parse |
| `src/listing.rs:708` `a_tabular_listing_that_selected_nothing_carries_no_session` | empty selection renders as an empty world; planted names are absent | structural; empty-listing text unpinned |
| `src/listing.rs:728` `sc_017h_every_listed_session_brings_its_agents_health_and_declared_state` | isolated agent rendering contains that agent's reference, health word, and declared state | content via isolation, not column index |
| `src/listing.rs:766` `sc_017e_the_window_is_the_one_the_flag_selected` | `--active` omits a stale name; default still contains it | membership |
| `src/lib.rs:478` `a_wired_list_writes_the_listing_to_stdout_and_exits_zero` | stdout contains `live`, not `old` | SC-017a membership. Exit 0 is a complete snapshot. |
| `tests/it/cli.rs:116-138` criterion 1 human arm | planted names reach stdout; status is `unknown` not `stopped` | SC-017l spelling. `unknown` as a word is criterion 5, not layout. |
| `tests/it/phase3.rs:269` `human_rows` | session rows start at column 0; name then status are the first two whitespace tokens | **observation method**, not an expected layout string. A card layout or status-first columns would make every criterion 4–11/14 human assertion fail. Residual coupling the gate's own "parse human rows into identity + status" requirement forces. |
| `tests/it/phase3.rs:369-635` criteria 4–9, 11 | identity sequences vs the fixture | required selection/order; observed through `human_rows` |
| `tests/it/phase3.rs:1889` `health_cell` | the token after the agent reference is the health cell | observation method for SC-017r (distinct nonempty cells). Column index is layout; the test binds to the product's own cell rather than a word this file picked. |

No assertion compares human output to an ANSI sequence. Phase 3's own scope
guard greps `\\u{1b}[` in `phase3.rs` only.

---

## Incomplete-human diagnostic wording, path detail, and exit status

Criterion 12 / OC-P3-HUMAN-DIAGNOSTIC only. JSON process rc is **not** in this
item — it is retained under OC-P3-JSON-WARNING. CLI usage `2` and
`NO_STATE_ROOT` are a different contract.

| Location | Semantic fact | Verdict |
|---|---|---|
| `src/listing.rs:238` `diagnostic` (product, not a test) | wording, paths, and exit status documented as open; the count is not | n/a |
| `tests/it/phase3.rs:688-719` | complete stderr empty; incomplete stderr nonempty and contains the distinct-source count | PASS (presence + count) |
| `tests/it/phase3.rs:692` | complete-human rc is 0; incomplete-human rc unpinned | PASS (split) |
| `tests/it/phase3.rs:726` `criterion_12_the_count_is_not_hardcoded_because_two_sources_read_two` | one-loss and two-loss warnings differ; they contain `1` and `2` | PASS (count, not wording) |
| `tests/it/phase3.rs:867-868` | `diagnostic(complete).is_none()`, `diagnostic(incomplete).is_some()` | PASS (presence) |
| `tests/it/phase3.rs:874` scope guard | greps wording and compact `inventory_complete":true`; does not forbid `assert_eq!(code, 0)` because that shape is required on JSON legs | process check on one file |
| `tests/it/phase3.rs:1310` `presented` human | human rc discarded | PASS — incomplete-human rc is open |
| `tests/it/cli.rs:110` criterion 1 | JSON leg `assert_eq!(code, Some(0))`; human leg asserts nothing about rc | PASS (split) |
| `tests/it/cli.rs` criterion 1 human arm | incomplete human listing has nonempty stderr | PASS (presence) |
| `src/lib.rs:478` `a_wired_list_writes_the_listing_to_stdout_and_exits_zero` | human list over a **complete** injected world exits 0 | PASS — complete-human rc is not the open choice |
| `src/lib.rs:459` `a_list_with_no_state_root_says_so_on_stderr_and_exits_one` | unwired/missing `AE_HOME` is `EXIT_UNAVAILABLE` | out of this named item |
| `src/cli.rs` / `tests/it/cli.rs` usage-error `2` | SC-022 | out of this named item |

No assertion requires a path from `FailedSource` to appear in stderr. Fixture
paths such as `/home/x/.ae/sessions` are inputs, never expected diagnostic text.

---

## JSON stderr warning policy (JSON process rc retained)

JSON *stderr warning* presence/wording/path is open. JSON **process rc** is
retained (OC-P3-JSON-WARNING STILL_REQUIRED).

| Location | Semantic fact | Verdict |
|---|---|---|
| `tests/it/phase3.rs:766-772` `criterion_13_every_document_carries_version_2_and_the_completeness_boolean` | JSON stderr is captured and discarded; the comment states that requiring a warning or requiring silence would fail criterion 15 | PASS (warning policy open) |
| `tests/it/cli.rs:110` criterion 1 JSON leg | incomplete JSON listing exits 0 | PASS (rc retained) |
| `src/lib.rs:489` `a_wired_list_json_is_the_digest_document` | JSON list over a complete world exits 0 | PASS (rc retained) |
| `src/lib.rs:544` `a_request_that_succeeded_says_nothing_on_stderr` | `ls --json` over a complete world has empty stderr and exits 0 | PASS — complete JSON has empty stderr; rc retained |
| `tests/it/phase3.rs:1313` `presented` JSON `invoke_over` | `assert_eq!(json_code, 0)` | PASS (rc retained) |

No test requires JSON incomplete listings to warn, or forbids them from warning.

---

## Detailed machine loss records and their order

The successor digest carries `inventory_complete` (boolean). There is no
loss-record member on the machine surface, and no test asserts one.

| Location | Semantic fact | Verdict |
|---|---|---|
| `src/listing.rs:527` / `src/lib.rs:507` / `tests/it/phase3.rs:748` | the boolean is present with the supplied value | PASS (criterion 13) |
| `src/listing.rs:63-80` `World::losses` | presentation receives a count, not paths | product, not a test |

---

## Internal collection or sort implementation

Criterion 9 requires C-byte name order within a group and running/unknown/stopped
group order. That is ratified. The open choice is *how* the product sorts, and
whether it filters before sorting.

| Location | Semantic fact | Verdict |
|---|---|---|
| `src/filters.rs:123` doctest `Selection::select` | default selection is the running session | membership |
| `src/filters.rs:545` `sc_017b_all_shows_running_sessions_then_unknown_ones_then_stopped_ones` | group order running, unknown, stopped | ratified SC-017b/n |
| `src/filters.rs:615` `sc_017e_active_keeps_sessions_with_an_event_inside_the_window` | membership in C-byte order, not supply order | ratified SC-017n; comment refuses to pin supply order |
| `tests/it/phase3.rs:562` `criterion_10_every_adversarial_supply_order_differs_from_the_required_one` | planted supply / natural / case-folded orders differ from C order | control calibration |
| `tests/it/phase3.rs:601` `criterion_9_the_output_order_is_c_byte_within_group_and_running_unknown_stopped` | output identity sequence is the ratified C/group order for every supply | ratified order, not the algorithm |
| `tests/it/phase3.rs:1573` `criterion_10_a_non_c_locale_collates_these_names_differently_and_output_does_not` | `/usr/bin/sort` under a non-C locale differs; product output does not | control uses the platform `sort` binary; it does not name ae's sort |

No assertion names `sort_by`, `sort_unstable`, `BTreeMap`, or "filter then sort".

---

## Equal-name tie-breaking

No test plants two identities whose session names are byte-identical and asserts
which one comes first. `src/filters.rs:150-152` documents stable-sort as the
current choice and that SC-017n supplies no tie-breaker; that is comment, not
an expected value.

---

## Phase-3 tests reopening phase-1 discovery or phase-2 liveness/schema

Criterion 15 also fails a phase-3 test that rediscovers or reclassifies instead
of consuming the completed snapshot.

The deleted name `criterion_3_the_output_is_a_function_of_the_snapshot_and_nothing_else`
is **gone** from this tree. Live criterion-3 tests (fn-start lines against
`68448c18`):

| Location | Semantic fact | Verdict |
|---|---|---|
| `tests/it/phase3.rs:943` `criterion_2_presentation_starts_from_one_completed_classified_snapshot` | presentation input equals the classified set; marker precedes filter/sort/format | consumes the snapshot |
| `tests/it/phase3.rs:1400` `criterion_3_presentation_output_does_not_rederive_any_planted_snapshot_fact` | opposed post-entry worlds through the real `current_world` → `Presentation::enter` → list/ls route | consumes the snapshot |
| `tests/it/phase3.rs:1477` `criterion_3_a_new_listing_through_the_real_route_sees_every_opposed_axis` | a fresh listing after the opposed flip still sees the planted axes | consumes the snapshot |
| `tests/it/phase3.rs:1743` `criterion_3_the_places_this_crate_can_read_the_world_are_the_inventoried_ones` | compiler inventory of world-reading sites | capability half, not a re-open |
| `tests/it/phase3.rs:1959` `sc_017q_the_entry_point_reports_unknown_agents_rather_than_dead_ones` | end-to-end plant → classify → `Presentation::enter` → list. Comment names the injection, not a missing transport | consumes a completed snapshot |

`tests/it/cli.rs` criterion 1 invokes the real binary against a real state root.
That is criterion 1's required entry point, not a phase-3 test rediscovering
inside the renderer.
