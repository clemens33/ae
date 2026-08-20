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
| A-H5 — SC-211o, codex identity registration | COMPLETE — 16 case runs |
| A-H3 — the argument surface (SC-211a-j, SC-212c) | COMPLETE — 72 case runs |
| A-H1 — dispatch and version spellings (SC-012b, SC-014) | COMPLETE — 6 case runs |
| A-H2 — steward help and detach spellings (SC-013) | COMPLETE — 9 case runs |
| A-H8 — the long-lived query | not started |
| SC-211l — `say` under its containment | COMPLETE — 5 case runs |
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

## A-H5 — SC-211o (`_register-sid`)

Rows: SC-211o. Sixteen cases, each varying ONE fixture fact and capturing the artifact the
surface writes. `_register-sid` takes a SLOT (ae@72c7293:14752), reads `launch_id.<slot>`
and `launch_time.<slot>` from meta, scans today's and yesterday's Codex session directories,
and writes what it selects to `codex.<slot>.sid`.

**Constructed inputs, declared.** The candidate Codex session files and the `launch_id` /
`launch_time` meta lines are written by the CONTROLLER — there is no offline producer for
either, and the fake agent is not a codex-kind tool, so ae never writes those keys. Each
case records the exact planted bytes and their hashes in `planted-inputs.txt`. They are
input DATA the surface reads; every helper byte still comes from a real frozen launch. See
`RUN-MANIFEST.md` A6.

Artifact paths — `A-H5/<case>/`: `admissibility-ledger.txt`, `surface-state.txt`,
`invocations.tsv`, `planted-inputs.txt`, `meta.before.txt`, `meta.after.txt`,
`meta.diff.txt`, `sid-artifact.txt`, and `out/` carrying the measured invocation's streams
and both canaries'.

## The gate

`../gate/` holds the canonical gate. Each arm's own harness directory is a provenance
snapshots of the version each arm ran under; an older snapshot reports violations the
current gate does not, so a reader reproducing a gate result runs the canonical copy.

**Two pairs, answering two questions.** `h5-c09` / `h5-c10` are TOKEN-PRECEDENCE controls:
the candidate carries the matching token and only its recorded cwd differs, so they record
that the cwd fallback is not reached while the token path selects (the guard at
ae:14793). `h5-c15` / `h5-c16` are the CWD FALLBACK pair: a token no candidate carries, so
the fallback is reached and cwd is what decides. Every other byte and time is held constant
within each pair. `RUN-MANIFEST.md` A8-A10 carry how they got there, including a reading
that moved in a case the amendment did not name.

## A-H3 — the argument surface

Rows: SC-211a through SC-211j and SC-212c. Seventy-two cases across ten generated helpers,
one per input class in the executor brief, each a single invocation bracketed by
capture-path canaries. `say` is not here: SC-211l runs under its own containment section.

Each helper group has its OWN launched sandbox, so one group's mutations cannot become
another group's precondition. Cases inside a group run in the listed order and some of them
write; the order is part of the fixture and is recorded in every ledger.

Every case records the session directory's own byte listing before and after the
invocation (the `session-bytes` before, after and diff files) — several of these
helpers write, and a refusal that wrote something is a different reading from a refusal
that did not.

Four cases need the controller manipulation their input class names: an unreadable session meta (mode 000), an emptied session meta, a malformed config file, and a pane respawned as a plain shell.
Each is applied immediately before its case and reverted after. `RUN-MANIFEST.md` A11.

## Ledger chronology

Every ledger in this tree passes `../gate/check-ledger-chronology.py`: sequence identities
unique and monotonic. The check exists because they were not — see `RUN-MANIFEST.md` A12,
where a subshell introduced to fix an invocation directory silently broke the ordering
record in two arms, with presence and hash both passing.

## A-H7L — SC-211l (`say`) under containment

Five cases: text via argv, whitespace-only text, no args with redirected empty stdin, text
via a pipe, and no args on a REAL TTY (a pty, because the surface branches on whether stdin
is a terminal and a redirected stdin cannot present that input).

Containment, in the order of what carries the claim:

- **Layer 1, structural, load-bearing.** The bridge takes its root from `AE_HOME`
  (telegram-daemon:10-11). This fixture's `AE_HOME` is randomly named and created AFTER the
  system-root census, so nothing that predates the fixture could have been started with it.
  Reach is inherited across fork, so a child cannot reach what its parent cannot.
- **Layer 2, the census, corroborating — and it states its own blind spot.** It classifies
  by REACH rather than by name, and reports three classes: IN-RANGE, out-of-range, and
  UNKNOWN-REACH. macOS exposes a process's environment to `ps e` for only a subset of even
  one's own processes, so a process it cannot read is UNKNOWN, never counted as out of
  range. Each case's census reports between 865 and 916 unknown-reach processes; the claim
  this layer supports is bounded accordingly. The in-range rows are the fixture's own tmux
  server and panes, traceable by the pid/ppid columns in the census artifact.
- **Layer 3, refusing `curl`/`wget` stubs, arm-spawned processes only.** An already-running
  bridge never inherited this PATH and is NOT contained by them. The stub is fired
  deliberately in every case and its log must carry the attempt.

Every case's census must report its own deliberately in-range control; a case whose census
cannot see its control is INCONCLUSIVE rather than contained. `RUN-MANIFEST.md` A14-A15.

## A-H1 and A-H2 — spellings, each invoked separately

A-H1 covers SC-012b's help spellings and SC-014's version spellings; A-H2 covers the help
and detach spellings SC-013 owns. Each spelling is its own case: one shared capture cannot
show a divergence between spellings, and that is what these rows are about. The
unknown-option and non-option classes belong to SC-022 and the launch path and are not
here; `--init`, `--attach`, a bare `steward` and the `hub` alias belong to SC-932, SC-931,
SC-930 and SC-939f.

Each record groups cases into spelling FAMILIES BY THE HASH of their captured stdout, so
"these coincide" is a statement about bytes rather than an impression from reading three
tables side by side.

The detach cases run under a longer bound, and the arm records what was running under the
fixture's own AE_HOME at teardown before reaping it — a detach starts something, and what
it started is part of the reading.

**A-H2's five selector cases stop at a config-presence check**, so they establish the
refusal and its bytes and NOTHING about whether the detach spellings, the two selector
orders, or a repeated selector differ. Five spellings with one reading here is not evidence
that the spellings are equivalent; the question was never asked. `A-H2/ARM-GAPS.txt` says
so beside the captures, and `ARM-GAPS.md` collects every arm's gaps with what would lift
each.
