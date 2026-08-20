# Batch L — section L-COMPACT artifacts — MANIFEST

Worker: `opus5:lexec`. Maps **arm → row ids → artifact paths → fixtures → mutation diffs**
for the L-COMPACT (compact/handover + preview + exits) evidence run. Captures only:
bytes, hashes, recursive manifests, byte diffs, rc values, barrier orderings, trace
channels, pty transcripts, process statuses. No verdicts, no expected-vs-actual
statements. Seats classify.

## Run-wide provenance

Frozen commit `72c729343a0117af2968b66e1c43f89ad25fc0b2`, frozen `ae` sha256
`b7b8aa9f…`; tool hashes in `_harness/env-record.txt`; `LANG=LC_ALL=en_US.UTF-8`,
`TZ=UTC`; no live models, no network. Each arm in its own disposable sandbox under
`/tmp/aelx/L-COMPACT/<arm>/`. Fixtures are real sessions launched with the
renamed-interpreter fake tool (a copy of `bash` named `claude`) that accepts real sends.

## L-HOOKS-v3 — the third instrumented copy

| field | value |
|---|---|
| instrumented `ae` sha256 | `b1b07709b01a66f7467333a151e641d2139b6950e58ca4cd53b47aa020afdfdd` |
| patch sha256 | `9243b21168b2efe1c031856c36651c8f5c230af269fa4e8029acff1563c7ad6c` (`_harness/hooks-v3.patch`) |
| generator | `_harness/mkhooks3.py` |
| content | **v2 plus exactly three compact TRACE CHANNELS, nothing else**: `b_cp_resolver_entry` (`_compact_freeze_source` entry — the tuple-freeze site), `b_cp_reval_after_confirmation` (the FIRST revalidation site), `b_cp_reval_after_wait` (the SECOND revalidation site) |

`_H` is unchanged across v1/v2/v3: it returns 0 immediately when `AE_L_HOOKS` is unset,
and when active it only appends a barrier ordinal to a harness file and optionally blocks
on a release file. It never reads, hashes or computes over product state. Every `ARM.txt`
carries `hook_patch_version` and the binary's own sha256. **Measured from the tree: 5 of
the 18 arms ran under v3** — `baseline`, `recovery-exec-selected`, `recovery-exec-contrast`,
`revalidation-after-answer`, `revalidation-after-handover` — and the other 13 ran on the
unmodified frozen binary.

This v3 is unrelated to the SC-515b bound question ruled on in L-STOP: no constant was
patched anywhere. The three additions are barrier calls at three sites.

### v3 admissibility — proven BEFORE any v3-hooked capture (`_admissibility/`)

| file | comparison | comparator verdict |
|---|---|---|
| `equiv-I-v3-inactive-compact.txt` | frozen vs v3 with `AE_L_HOOKS` unset, on the COMPACT fixture (launch, `archive preview`, `compact -f --digest-only`) | NO_DIFFERENCES |
| `equiv-I-known-difference.txt` | same fixture, `--digest-only` dropped under a short handover bound | DIFFERENCES_PRESENT |

v1's and v2's proofs are untouched.

## The named trace channels (SC-827 material)

`baseline/trace-channels.txt` records the channels in the order they fired, with pid and
monotonic clock, plus a legend that names each channel's SITE and nothing else.

**Two pid columns exist and they are not the same number.** `hook-trace.tsv` (and the copy
inside `trace-channels.txt`) records `$$` — the shell's own pid, identical for every
channel of one invocation. `barrier-order.tsv`'s barrier KEY records `${BASHPID}`, which
differs when a site runs inside a command substitution: in this arm the resolver-entry key
reads `.86134.` while every other key and every trace line reads `86132`. Both artifacts
are correct; they answer different questions, and any claim about "a different pid" has to
name which of the two it comes from. The
resolver entry is a separate channel from the two revalidation sites, so one authoritative
resolution and the permitted revalidation reads are separable without counting raw meta
reads. What that ordering MEANS is not stated anywhere in this section.

## Common capture set (every arm)

`ARM.txt` (arm, roster ids, construction, hook patch version, binary sha256, session
uuid, rc values, bounds) · `<step>.stdout` / `.stderr` / `.rc` / `.invocation` ·
`<step>.stdout.od` / `.stderr.od` and `<step>.stream-sizes.txt` — **both streams captured
SEPARATELY and byte-exactly, with `od -c` dumps, sizes and sha256** ·
`1pre.*` / `3post.*` (`AE_HOME`, sessions and archive manifests, tmux snapshot, a copy of
every session's `events.jsonl`) · `preflight-tab.txt` (the blocking environment-as-instrument
proof in the arm's own `env -i`) · `consumer-inproc.txt` + `tmux-argv.log` (rule (e)) ·
`SHA256SUMS.txt`. Barrier arms add `barrier-order.tsv`, `hook-trace.tsv` and, per cut,
`<tag>.stdout-at-cut` / `.stderr-at-cut` / `.stdout-at-cut.od` plus sessions and archive
manifests.

## Arms

| arm | roster ids | construction | key artifacts |
|---|---|---|---|
| `baseline` | SC-500, SC-501, SC-502, SC-827, SC-1305 | a real `compact -f --digest-only` under the v3 trace channels, blocking at every named compact cut; both streams captured separately and byte-exactly and snapshotted AGAIN at each cut; a concurrent `ae list --json` and the generated `requests` helper run from a separate process at EVERY cut | `trace-channels.txt`, `hook-trace.tsv`, `barrier-order.tsv`, `b0*-*.stdout-at-cut(.od)`, `b0*-*.observer.list.*` and `.observer.requests.*` (35 observer artifacts), `2op.stream-sizes.txt` |
| `recovery-exec-selected` | SC-512 | the compact is cut at `b_cp_pre_relaunch` — archive published, source removed, relaunch not yet `exec`d — by SIGKILLing the whole tree, so the live state IS the specimen at its own path; the printed `Recovery:` line is then extracted and executed VERBATIM | `specimen.txt`, `specimen.pre-relaunch.*` (manifests, tmux snapshot, and a mode-preserving copy of the whole `AE_HOME`), `recovery-line.txt` + `.od`, `recovery-command.verbatim.txt`, `3recovery.*` |
| `recovery-exec-contrast` | none — contrast only | the same compact allowed to COMPLETE, so the state afterwards already contains the replacement session. Captured for contrast and explicitly NOT the SC-512 specimen (that is SC-822's territory) | `2after-compact.*`, `3post.*` |
| `interactive-typed-n` | SC-503a | `ae compact --digest-only` on a real terminal, `n` typed at the confirmation | `2op.pane.at-prompt.txt`, `2op.*`, `.od` dumps |
| `interactive-eof` | SC-503b | the same command with stdin closed by the controller (`/dev/null`) and no terminal | `2op.invocation` (states the stdin construction), `2op.*` |
| `interactive-force` | SC-837 | `ae compact -f --digest-only`, so no confirmation is asked at all | `2op.*` |
| `sigpipe` | SC-504b | producer and early-closing consumer as SEPARATELY SUPERVISED processes: the consumer creates an explicit pipe, hands the write end to the producer, reads exactly ONE line, closes the read end, then reaps. **No shell pipeline is placed over the subject** | `sigpipe-harness.py` (byte copy of the harness that ran), `sigpipe-record.json` (both statuses, `WIFEXITED`/`WIFSIGNALED`, the exit code, the term signal and its name, and the one line the consumer read), `producer.stdout.firstline`, `producer.stderr`, `3post.*` for the relaunch state |
| `revalidation-after-answer` | SC-828 | the controller replaces the live session's recorded identity at `b_cp_after_answer` (temp + chmod-to-original-mode + rename), then the compact continues | `<tag>.meta.before.txt`, `.after.txt`, `.diff`, `<tag>.controller.txt` (which state changed), `<tag>.sessions.after-mutation.tsv` |
| `revalidation-after-handover` | SC-828 | the same mutation at `b_cp_after_handover` | same set |
| `handover-withholding-only-reply` | SC-829a | a real `compact -f` under a shortened handover bound (`AE_COMPACT_HANDOVER_SECS=25`); once the request row is observed the controller supplies ONLY a reply, through the REAL generated `reply` helper, and no memo | `source-trace.what-completion-polls.txt`, `request.txt`, `request-bodies.txt`, `baseline-bytes-used.txt`, `planted-pane.txt`, `3reply.*`, `state-at-bound.txt` |
| `handover-withholding-only-memo` | SC-829a | the same, supplying ONLY a handover memo through the real generated `memo` helper, and no reply | same set with `3memo.*` |
| `handover-withholding-neither` | SC-829a | the same, supplying NOTHING | same set with `withheld.txt` |
| `handover-rerun-after-interrupt` | SC-829b | a real `compact -f` is SIGKILLed AFTER its handover request is published, then compact is run again on the same session | `interrupt.txt`, `events.after-first-request.jsonl`, `events.after-rerun.jsonl`, `request-events.txt` (ask/reply row counts, every ask row verbatim, every baseline line), `baseline.run1.txt`, `baseline.run2.txt`, `baseline.diff` |
| `config-keephistory-with-keep` | SC-836 | the session's own config sets `[workspace] purge_agent_history = true`; compact runs WITH `--keep-history` | `config.txt`, `planted-conversations.txt`, `conversations.before.tsv`, `.after.tsv`, `.diff` |
| `config-keephistory-without-keep` | SC-836 | the same config, compact runs WITHOUT `--keep-history` | same set |
| `exit-identity-terminal-attach` | SC-517a, SC-517b | compact runs on a REAL terminal (a driver pane on a dedicated control server), so the relaunch it `exec`s into reaches a terminal attach; the controller then detaches the inner client with the tmux prefix and `d` | `2op.pane.attached.txt`, `attach.txt`, `2op.pane.after-detach.txt`, `2op.rc` |
| `exit-identity-no-terminal` | SC-517c | compact invoked with `-f` (so it does not exit at confirmation EOF) and with NO stream attached to a terminal; the fresh session is created but nothing can attach | `2op.invocation` (records that none of the three streams is a tty), `2op.stdout(.od)`, `2op.stderr(.od)`, `2op.rc` |
| `preview` | SC-507a, SC-507c, SC-507d | `ae archive preview` runs on a FROZEN (stopped) session; a TWIN of that same frozen session is then ended for real, and the twin's ARCHIVED `digest.md` bytes are captured alongside the preview's stdout bytes | `twin.txt`, `twin-meta.before/after.txt`, `twin-meta.diff`, `twin-vs-source.manifest.diff`, `twin.archived-digest.md` + `.od`, `2op.stdout(.od)`, `aehome.before-after.diff`, events deltas |

Plus one section-level artifact:

| path | roster id | content |
|---|---|---|
| `residual-rc/rc-table.tsv` | SC-508 | the capture-only exit-status table across EVERY arm in the section — arm, step, rc, and the argv that produced it — assembled from each arm's own recorded `.rc` and `.invocation` files. 54 data rows (plus a header and seven comment lines). No status is compared with another and no arm is interpreted |

### The twin construction, stated exactly

Two coexisting sessions cannot share one uuid, so the twin is a byte copy (`cp -Rp`) of
the FROZEN (stopped) session directory carrying exactly TWO named byte diffs — `session=`
and `session_id=` — both applied mode-preserving and both recorded in `twin-meta.diff`.
`twin-vs-source.manifest.diff` shows the rest of the directory unchanged. The worker
captures the preview stdout bytes and the twin's archived digest bytes and compares
nothing between them.

### The planted agent pane, stated exactly

The frozen `reply` helper proves the responder from the CURRENT PANE's `@ae_slot`
(72c7293:14316 region), so a controller process cannot answer a request without one.
The withholding arms therefore split a fresh pane in the main agent's own window and set
`@ae_agent` / `@ae_slot` on it to the source pane's values — a named controller
manipulation, recorded per arm in `planted-pane.txt` together with what it did NOT change
(the agent pane itself, its process, and every file under `AE_HOME`). The `reply` and
`memo` helpers that then run are the REAL generated ones.

## Roster coverage as executed

All 21 L-COMPACT roster ids have an arm above: SC-500, SC-501, SC-502, SC-503a, SC-503b,
SC-504b, SC-507a, SC-507c, SC-507d, SC-508, SC-512, SC-517a, SC-517b, SC-517c, SC-827,
SC-828, SC-829a, SC-829b, SC-836, SC-837, SC-1305.

## Known limits of this section, stated

- `recovery-exec-contrast` carries no roster id by design: it exists so the SC-512
  specimen cannot be confused with a post-relaunch state.
- The `handover-withholding-*` arms depend on the planted agent pane above. If a seat
  reads that plant as changing what the arm observes, the arm is re-runnable with a
  different responder construction; the plant is recorded rather than hidden.
- `sigpipe`'s record shows the producer's disposition as the kernel reported it
  (`WIFEXITED` / `WIFSIGNALED`, the code and, when signalled, the signal and its name).
  No interpretation of which of those two shapes it is accompanies the record.
- The watchdog is left ENABLED (the launch default) in every arm.

## Harness stability at the section boundary

`L-COMPACT/harness-snapshot/` is a byte copy of the shared `_harness/` exactly as this
section ran, hashed by `L-COMPACT/HARNESS-SHA256SUMS.txt`;
`L-COMPACT/ADMISSIBILITY-SHA256SUMS.txt` hashes the admissibility proofs it rests on.
Nothing under `L-COMPACT/` changes after this point.
