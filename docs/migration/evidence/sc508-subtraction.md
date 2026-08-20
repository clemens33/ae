# SC-508 subtraction

Source: `docs/migration/evidence/l-artifacts/L-COMPACT/residual-rc/rc-table.tsv`.
The source has 54 data rows (lines 10–63); this split keeps source order within
each group and quotes every invocation field verbatim.

Counts: Group A = 29, Group B = 25, Group C = 0. **29 + 25 + 0 = 54.**
UNCERTAIN rows: 0. The source has two schema-width anomalies: row 29 has seven
tab-separated fields because its invocation contains three unescaped tabs, and
row 57 has five because its invocation contains one unescaped tab. They remain
quoted as recorded below; every other data row has the four fields
(`arm`, `step`, `rc`, `invocation`).

## Group A — already owned (29)

The owner cue is quoted from the row itself (invocation and status together).

| source row | arm / step | rc | invocation (verbatim) | owner | owner cue |
|---:|---|---:|---|---|---|
| 11 | baseline / `2op` | 1 | `/tmp/aelx/L-COMPACT/baseline/b/ae compact -f --digest-only cp1` | SC-517a | “`compact -f --digest-only cp1`” with rc `1` |
| 12 | baseline / `b01-b_cp_resolver_entry.86134.1.observer.list` | 0 | `-` | SC-1305 | “`b01-b_cp_resolver_entry.86134.1.observer.list`” with rc `0` and invocation `-` |
| 13 | baseline / `b01-b_cp_resolver_entry.86134.1.observer.requests` | 0 | `-` | SC-1305 | “`b01-b_cp_resolver_entry.86134.1.observer.requests`” with rc `0` and invocation `-` |
| 14 | baseline / `b02-b_cp_after_answer.86132.1.observer.list` | 0 | `-` | SC-1305 | “`b02-b_cp_after_answer.86132.1.observer.list`” with rc `0` and invocation `-` |
| 15 | baseline / `b02-b_cp_after_answer.86132.1.observer.requests` | 0 | `-` | SC-1305 | “`b02-b_cp_after_answer.86132.1.observer.requests`” with rc `0` and invocation `-` |
| 16 | baseline / `b03-b_cp_reval_after_confirmation.86132.2.observer.list` | 0 | `-` | SC-1305 | “`b03-b_cp_reval_after_confirmation.86132.2.observer.list`” with rc `0` and invocation `-` |
| 17 | baseline / `b03-b_cp_reval_after_confirmation.86132.2.observer.requests` | 0 | `-` | SC-1305 | “`b03-b_cp_reval_after_confirmation.86132.2.observer.requests`” with rc `0` and invocation `-` |
| 18 | baseline / `b04-b_cp_after_handover.86132.3.observer.list` | 0 | `-` | SC-1305 | “`b04-b_cp_after_handover.86132.3.observer.list`” with rc `0` and invocation `-` |
| 19 | baseline / `b04-b_cp_after_handover.86132.3.observer.requests` | 0 | `-` | SC-1305 | “`b04-b_cp_after_handover.86132.3.observer.requests`” with rc `0` and invocation `-` |
| 20 | baseline / `b05-b_cp_reval_after_wait.86132.4.observer.list` | 0 | `-` | SC-1305 | “`b05-b_cp_reval_after_wait.86132.4.observer.list`” with rc `0` and invocation `-` |
| 21 | baseline / `b05-b_cp_reval_after_wait.86132.4.observer.requests` | 0 | `-` | SC-1305 | “`b05-b_cp_reval_after_wait.86132.4.observer.requests`” with rc `0` and invocation `-` |
| 22 | baseline / `b06-b_cp_pre_relaunch.86132.5.observer.list` | 0 | `-` | SC-1305 | “`b06-b_cp_pre_relaunch.86132.5.observer.list`” with rc `0` and invocation `-` |
| 23 | baseline / `b06-b_cp_pre_relaunch.86132.5.observer.requests` | - | `-` | SC-1305 | “`b06-b_cp_pre_relaunch.86132.5.observer.requests`” with rc `-` and invocation `-`; rc `-` rather than `0` because the requests helper does not exist to run at the pre-relaunch cut, the observed mechanism cited by SC-1305 |
| 25 | config-keephistory-with-keep / `2op` | 1 | `/tmp/aelx/L-COMPACT/config-keephistory-with-keep/b/ae compact -f --keep-history --digest-only cp1` | SC-836 | “`--keep-history --digest-only cp1`” with rc `1` |
| 27 | config-keephistory-without-keep / `2op` | 1 | `/tmp/aelx/L-COMPACT/config-keephistory-without-keep/b/ae compact -f --digest-only cp1` | SC-836 | “`--digest-only cp1`” with rc `1` |
| 29 | exit-identity-no-terminal / `2op` | 1 | `/tmp/aelx/L-COMPACT/exit-identity-no-terminal/b/ae compact -f --digest-only cp1 stdin: /dev/null; stdout and stderr: regular files — no stream is a terminal is_a_tty.stdin	no is_a_tty.stdout	no is_a_tty.stderr	no` | SC-517c | “`no stream is a terminal`” with rc `1` |
| 31 | exit-identity-terminal-attach / `2op` | 0 | `/tmp/aelx/L-COMPACT/exit-identity-terminal-attach/b/ae compact -f --digest-only cp1` | SC-517b | “`compact -f --digest-only cp1`” with rc `0` |
| 34 | handover-rerun-after-interrupt / `3rerun` | 1 | `/tmp/aelx/L-COMPACT/handover-rerun-after-interrupt/b/ae compact -f cp1` | SC-829b | step “`3rerun`”, “`compact -f cp1`”, rc `1` |
| 36 | handover-withholding-neither / `2op` | 1 | `/tmp/aelx/L-COMPACT/handover-withholding-neither/b/ae compact -f cp1` | SC-829a | “`compact -f cp1`” with rc `1` |
| 38 | handover-withholding-only-memo / `2op` | 1 | `/tmp/aelx/L-COMPACT/handover-withholding-only-memo/b/ae compact -f cp1` | SC-829a | “`compact -f cp1`” with rc `1` |
| 41 | handover-withholding-only-reply / `2op` | 1 | `/tmp/aelx/L-COMPACT/handover-withholding-only-reply/b/ae compact -f cp1` | SC-829a | “`compact -f cp1`” with rc `1` |
| 44 | interactive-eof / `2op` | 1 | `/tmp/aelx/L-COMPACT/interactive-eof/b/ae compact --digest-only cp1 stdin: /dev/null (closed by the controller — end of input, no terminal)` | SC-503b | “`end of input, no terminal`” with rc `1` |
| 46 | interactive-force / `2op` | 1 | `/tmp/aelx/L-COMPACT/interactive-force/b/ae compact -f --digest-only cp1` | SC-837 | “`compact -f --digest-only cp1`” with rc `1` |
| 48 | interactive-typed-n / `2op` | 0 | `/tmp/aelx/L-COMPACT/interactive-typed-n/b/ae compact --digest-only cp1` | SC-503a | “`compact --digest-only cp1`” with rc `0` |
| 51 | preview / `2op` | 0 | `/tmp/aelx/L-COMPACT/preview/b/ae archive preview cp1` | SC-507 | “`archive preview cp1`” with rc `0` |
| 54 | recovery-exec-contrast / `2op` | 1 | `/tmp/aelx/L-COMPACT/recovery-exec-contrast/b/ae compact -f --digest-only cp1` | SC-517a | “`compact -f --digest-only cp1`” with rc `1` |
| 57 | recovery-exec-selected / `3recovery` | 1 | `executed	VERBATIM, as one shell command, in the arm environment` | SC-512 | “`executed	VERBATIM, as one shell command, in the arm environment`” with rc `1` |
| 59 | revalidation-after-answer / `2op` | 1 | `/tmp/aelx/L-COMPACT/revalidation-after-answer/b/ae compact -f --digest-only cp1` | SC-828 | “`compact -f --digest-only cp1`” with rc `1` |
| 61 | revalidation-after-handover / `2op` | 1 | `/tmp/aelx/L-COMPACT/revalidation-after-handover/b/ae compact -f --digest-only cp1` | SC-828 | “`compact -f --digest-only cp1`” with rc `1` |

## Group B — harness-only (25)

These rows are setup statuses, deliberate kills, fixture teardown, or
helper/controller statuses rather than the exit status of an ae command under
test.

| source row | arm / step | rc | invocation (verbatim) | harness category |
|---:|---|---:|---|---|
| 10 | baseline / `0launch` | 1 | `/tmp/aelx/L-COMPACT/baseline/b/ae --local cp1` | setup launch |
| 24 | config-keephistory-with-keep / `0launch` | 1 | `/tmp/aelx/L-COMPACT/config-keephistory-with-keep/b/ae --local cp1` | setup launch |
| 26 | config-keephistory-without-keep / `0launch` | 1 | `/tmp/aelx/L-COMPACT/config-keephistory-without-keep/b/ae --local cp1` | setup launch |
| 28 | exit-identity-no-terminal / `0launch` | 1 | `/tmp/aelx/L-COMPACT/exit-identity-no-terminal/b/ae --local cp1` | setup launch |
| 30 | exit-identity-terminal-attach / `0launch` | 1 | `/tmp/aelx/L-COMPACT/exit-identity-terminal-attach/b/ae --local cp1` | setup launch |
| 32 | handover-rerun-after-interrupt / `0launch` | 1 | `/tmp/aelx/L-COMPACT/handover-rerun-after-interrupt/b/ae --local cp1` | setup launch |
| 33 | handover-rerun-after-interrupt / `2op` | 137 | `/tmp/aelx/L-COMPACT/handover-rerun-after-interrupt/b/ae compact -f cp1` | deliberate kill |
| 35 | handover-withholding-neither / `0launch` | 1 | `/tmp/aelx/L-COMPACT/handover-withholding-neither/b/ae --local cp1` | setup launch |
| 37 | handover-withholding-only-memo / `0launch` | 1 | `/tmp/aelx/L-COMPACT/handover-withholding-only-memo/b/ae --local cp1` | setup launch |
| 39 | handover-withholding-only-memo / `3memo` | 0 | `/tmp/aelx/L-COMPACT/handover-withholding-only-memo/h/.ae/sessions/cp1/memo add --topic handover state of play at the boundary` | helper/controller setup |
| 40 | handover-withholding-only-reply / `0launch` | 1 | `/tmp/aelx/L-COMPACT/handover-withholding-only-reply/b/ae --local cp1` | setup launch |
| 42 | handover-withholding-only-reply / `3reply` | 0 | `/tmp/aelx/L-COMPACT/handover-withholding-only-reply/h/.ae/sessions/cp1/reply ae-20260820T180012Z-2a8da3c6 handover done, nothing else outstanding` | helper/controller setup |
| 43 | interactive-eof / `0launch` | 1 | `/tmp/aelx/L-COMPACT/interactive-eof/b/ae --local cp1` | setup launch |
| 45 | interactive-force / `0launch` | 1 | `/tmp/aelx/L-COMPACT/interactive-force/b/ae --local cp1` | setup launch |
| 47 | interactive-typed-n / `0launch` | 1 | `/tmp/aelx/L-COMPACT/interactive-typed-n/b/ae --local cp1` | setup launch |
| 49 | preview / `0launch` | 1 | `/tmp/aelx/L-COMPACT/preview/b/ae --local cp1` | setup launch |
| 50 | preview / `1stop` | 0 | `/tmp/aelx/L-COMPACT/preview/b/ae stop -y cp1` | fixture teardown/setup |
| 52 | preview / `3twinend` | 0 | `/tmp/aelx/L-COMPACT/preview/b/ae end -f twin` | fixture teardown |
| 53 | recovery-exec-contrast / `0launch` | 1 | `/tmp/aelx/L-COMPACT/recovery-exec-contrast/b/ae --local cp1` | setup launch |
| 55 | recovery-exec-selected / `0launch` | 1 | `/tmp/aelx/L-COMPACT/recovery-exec-selected/b/ae --local cp1` | setup launch |
| 56 | recovery-exec-selected / `2op` | 137 | `/tmp/aelx/L-COMPACT/recovery-exec-selected/b/ae compact -f --digest-only cp1` | deliberate kill |
| 58 | revalidation-after-answer / `0launch` | 1 | `/tmp/aelx/L-COMPACT/revalidation-after-answer/b/ae --local cp1` | setup launch |
| 60 | revalidation-after-handover / `0launch` | 1 | `/tmp/aelx/L-COMPACT/revalidation-after-handover/b/ae --local cp1` | setup launch |
| 62 | sigpipe / `0launch` | 1 | `/tmp/aelx/L-COMPACT/sigpipe/b/ae --local cp1` | setup launch |
| 63 | sigpipe / `2op` | 0 | `-` | harness supervisor |

## Group C — true residual (0)

No rows remain after Groups A and B. Therefore there are no UNCERTAIN rows.
