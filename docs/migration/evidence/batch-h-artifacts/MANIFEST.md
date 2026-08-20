# Batch H — artifact manifest

Captures for the H-HELPER batch, run under pre-registration against frozen `ae` at
`72c7293`. The design is `../batch-h-design.md`, the seat-facing census
`../batch-h-argument-census.md`, the executor brief `../batch-h-input-list.md`, and the
script hashes and amendments `RUN-MANIFEST.md`.

No capture here is classified. Every reading is a candidate observation until a seat
accepts it.

| section | status |
|---|---|
| A-H4 — SC-211p, `_lib` name resolution | COMPLETE — 15 case runs |
| A-H5 — SC-211o, codex identity registration | not started |
| A-H3 — the argument surface | not started |
| A-H1 / A-H2 — dispatch, version, steward | not started |
| A-H8 — the long-lived query | not started |
| SC-211l — under its containment | not started |
| SC-1301 — hooks and barriers | not started |
| D14b | HELD and EXCLUDED pending the ownership-record split |

## A-H4 — SC-211p (`_lib` name resolution)

Rows: SC-211p. Fifteen cases, one per input class in the executor brief, each invoking the
generated `_lib`'s own `ae_resolve` and capturing the resolver's output contract
(the four `AE_RESOLVED_` variables, ae@72c7293:12983-12989) with rc and stderr.

`focus` is deliberately not the observation surface: it mutates client focus, emits an
event, and a failure it reports can originate downstream of the grammar, so grammar and
pane liveness would be confounded in one rc.

**Instrument controls, per case:** a capture-path canary with known stdout bytes, known
stderr bytes and a known rc, pushed through the exact wrapper the measured invocation uses,
fired BEFORE and AFTER that invocation — 30 canaries across the arm, all passing. Plus an
environment-equivalence record placing the controller's resolution domain
(`_AE_SESSION`, `_AE_SESSIONS_DIR`, the tmux selector, cwd and the pane variables) beside a real
generated-helper invocation's from the same fixture.

**Fixture validity is captured before any case runs** (`A-H4/fixture-validity.txt`): the
roster of both sessions, every pane on the server, and the spawn output that creates the
bare-name collision. The arm's first complete run was discarded because the fixture did not
carry the collisions three of its cases named — see `RUN-MANIFEST.md` amendment A2.

Artifact paths — `A-H4/<case>/`:

- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch: case open, rows,
  fixture, source state, environment equivalence, the measured input, PRODUCT-START and
  PRODUCT-COMPLETE, both canaries with their carried bytes and rc, and case close
- `surface-state.txt` — the `_lib` and `meta` as the invoking uid sees them: existence,
  type, mode, size, interpreter line, and the rc and stderr of a real read attempt
- `invocations.tsv` — label, rc, stdout/stderr sha256 and bytes, bound, timed-out, argv
- `out/resolve.stdout` / `out/resolve.stderr`, and the two canaries' streams
- `env.helper-domain.txt`, `env.controller-domain.txt`, `env.domain-diff.txt`
- `roster.txt` — the session's `agent.*` lines as the case saw them
- `A-H4/resolution-record.txt` (generated), `A-H4/fixture-validity.txt`, `SHA256SUMS.txt`
