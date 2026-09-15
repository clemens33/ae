# Muse composer fixtures

Captured frames of the muse CLI's REAL input composer, for the delivery
grammar. Unlike the hand-built fixtures elsewhere, every byte here was captured
from a live pane.

| Fixture | Tool | Frame | Captured | Provenance |
|---|---|---|---|---|
| `muse-stuck-composer.esc` | muse-spark-1.3 (`spark13cli`, muse CLI) | a pasted turn the harness REFUSED ("Turn-submit backlog full (4); try again."); the composer still holds `[Pasted Content 1494 chars]` | 2026-09-14, session `mac:qwen36eval`, pane `%245` (retired since) | `capture-pane -e` bytes, sha256 `8436f3b73302070fd20e1fa6ad6ed79e8b47d204ca1ca59f43ae2fe23b40ada3`; transcript text as captured, unredacted |
| `muse-occupied-composer.esc` | muse-spark-1.3, live ae-dev seat | staged, before Enter; composer holds `[Pasted Content 1766 chars]`; the transcript above carries an earlier backend refusal | 2026-09-14, ae-dev | sha256 `8536d03c560fb380b408bae27548b8070f64513fad160e1e43efc44bbbde0b47` |
| `muse-accepted-composer.esc` | muse-spark-1.3, live ae-dev seat | ACCEPTED: the composer cleared, `◆ Working (0s)` is drawn, and the staged token moved into the TRANSCRIPT as `❯ [Pasted Content 1766 chars]` | 2026-09-14, ae-dev | sha256 `e48be9a36b04f1f9ed5b5da3e89328ce22216011e75cdae25fb10bd9f4c1e00d` |
| `muse-idle-composer.esc` | muse-spark-1.3, live ae-dev seat | nothing staged; a backend 429 error sits in the transcript, composer empty | 2026-09-14, ae-dev | sha256 `e086277a93f190367f8d6328a7d4b105e47a5fe795d3ed389f9b708bd4ae1055` |

Every frame is `capture-pane -e` bytes. Transport, widths and timestamps of the
live captures are in the capturing session's provenance note
(`.local/muse-capture-provenance.md` in the checkout that produced them).

**What is load-bearing, and what is not.** Only the composer frame at the bottom
— its titled rule, the `❯` composer row, the full-width bottom rule — and the
`❯` ornament rows are contractual. The transcript rows above are incidental
capture noise (a context document, backend errors) carried verbatim because
provenance beats cosmetics on a captured specimen; a later "cleanup" of those
rows must not touch the composer frame or the three ornament rows, or the
hazard-2 regression pin (`a_transcript_echo_of_a_submitted_turn_does_not_read_as_still_staged`)
silently stops testing what it names.

**`❯` is NOT unique on screen.** The accepted and occupied frames each carry
THREE `❯` rows: transcript echoes of already-submitted messages (one still
reading literally `❯ [Pasted Content 1766 chars]`), the refusal glyph rows, and
the live composer. Only the composer — the ornament row inside the bottom
rule-delimited frame — is the input box. The read must never be a "does the
capture contain the token" test; `muse-accepted-composer.esc` is the regression
specimen.

**Two refusal shapes, both captured.** A FULL composer (`muse-stuck`,
`muse-occupied`) means the turn was never submitted, so the existing retry loop
is the right answer. An EMPTY composer (`muse-idle`, `muse-accepted`) means the
turn WAS submitted, backend error or not — ae owns delivery, not the answer, and
must not retry.

**Nothing restores the text after a backend refusal.** A second live batch
(`muse-accepted-t600ms.esc` sha256
`91a9e208e71805ce52e99463c4499d974216f4cf19996321106f483ba6ec1b41`,
`muse-accepted-t2s.esc` sha256
`4f69a0b6822e8e5ca55a95b2f7d429464d3e8c6d32f383de54dad0295a513d6e`,
`muse-accepted-settled.esc` sha256
`4044bd7a5a16ef6298c2449c32b4635864a52044f239f1dddab14b3f41d4a85f`, and
`muse-accepted-190.esc` sha256
`760f3f698c745569ef3c6e49942c08c9a35aa2bc7e00ff1b65e05c2744f3a9ff`,
captured in the producing checkout's `.local/`) shows the composer empty at
+600 ms, +2 s and +25.5 s, in two runs, at 80 and 190 columns. A 429 landed
~1.44 s after the clear and the composer stayed empty. ae verifies at
`VERIFY_POLL` 300 ms across `VERIFY_RETRIES` 2, well inside that. Caveat carried
honestly: both runs were QUOTA-REFUSED — submit ACCEPTED, backend never
answered. "Muse accepts and clears" is proven; a full answer round trip is not,
and the composer behaviour is what ae's predicate reads.

**A transcript echo carrying an ornament is the NORMAL steady state**, not a
one-off: every settled capture carries one at line 4. The multi-`❯` condition is
present on every real screen a used pane shows, so the bottom-most-ornament
selection is load-bearing everywhere, not only in the accepted fixture above.
