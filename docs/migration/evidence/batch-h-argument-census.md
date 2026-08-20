# Batch H — argument census (SEAT-FACING; source-derived, no captures)

Required pre-step for the Batch H design. Built by READING the frozen `ae` at `72c7293`,
with a line citation for every row. **Nothing here was executed**; no fixture exists yet
and no capture was taken.

**What the class column means.** `ACCEPTED` / `REJECTED` / `IGNORED` / `HANGS` is a reading
of the incumbent SOURCE at the cited line — what the code does with that input shape. It is
not a statement about what the contract requires, and it does not substitute for a capture:
the arms capture behaviour independently, and **a capture that contradicts a row here is a
finding, not an error in the arm**. The census exists so the input matrix is COMPLETE, not
so the outcome is known in advance.

**Spellings are invoked separately, and the count is not typed here.** Where a row is a
list of alternative spellings, EVERY spelling becomes its own entry in the generated
executor list, so a group cannot collapse to one chosen spelling. The splitter is anchored
to the whole cell: a description that merely contains a slash is one input, not two. The
number of entries is whatever the generator reports — an earlier version of this file typed
a count that was already stale when the tool disagreed with it.

**Executor scoping.** Per seat ruling the executor receives
`batch-h-input-list.md` — the same input classes with no class labels and no outcome
citations. This document stays with the seats.

## Top-level dispatcher (SC-012b, SC-014)

| Input | Reaches | Class |
|---|---|---|
| `-h` | outer case arm, ae:16841-16843 | ACCEPTED |
| `--help` | same arm, ae:16841 | ACCEPTED |
| `help` | same arm, ae:16841 | ACCEPTED |
| `version` | outer case arm, ae:16845-16847 | ACCEPTED |
| `--version` | same arm, ae:16845 | ACCEPTED |
| `-V` | same arm, ae:16845 | ACCEPTED |
| an unknown LONG OPTION (`--nosuchflag`) | the launch parser's `--*)` arm, ae:16928-16930 | REJECTED | OUT-OF-BATCH — SC-022 |
| a non-option word (`nosuchthing`) | the launch parser's `*)` arm, ae:16932-16934 — bound to `_LP_NAME` | ACCEPTED (launch candidate) | OUT-OF-BATCH — launch |

**SC-012b owns the help aliases only.** The unknown-OPTION class is SC-022's and a
non-option word enters the launch path rather than the command dispatch; neither can close
SC-012b, both are marked OUT-OF-BATCH, and the generator therefore keeps them out of this
batch's executor brief. They remain in this census so the split is on the record: an
earlier version collapsed them into "an unknown first word", which erased it.

## `steward` (SC-013) — a wider surface than the row's wording implies

`steward | hub)` at ae:16722 opens an ITERATIVE parser (`while` at ae:16730, `case` at ae:16731) in which `--attach`/`--detach` are selectors that `shift` and continue, so a
later selector overrides an earlier one.

**Row ownership (S1MAP):** SC-013 owns the HELP and DETACH spellings only. `--init` is
SC-932's, `--attach`/`--switch` is SC-931's, a bare `steward` is SC-930's, and the `hub`
alias is SC-939f's. They are listed here for completeness of the parser and marked
OUT-OF-BATCH; **they are not SC-013 evidence and no H arm closes them.**

| Input | Reaches | Class | Row |
|---|---|---|---|
| `-h` | ae:16740-16742 | ACCEPTED (exit 0) | SC-013 |
| `--help` | ae:16740 | ACCEPTED (exit 0) | SC-013 |
| `help` | ae:16740 | ACCEPTED (exit 0) | SC-013 |
| `--detach` | selector, ae:16748-16750 | ACCEPTED | SC-013 |
| `--no-attach` | same arm, ae:16748 | ACCEPTED | SC-013 |
| help with trailing args | the help arm ignores the remainder, ae:16740-16742 | IGNORED | SC-013 |
| `--detach --attach` | iterative parser, ae:16730-16750 | ACCEPTED | SC-013 boundary; attach half is SC-931 |
| `--attach --detach` | iterative parser, ae:16730-16750 | ACCEPTED | SC-013 boundary; attach half is SC-931 |
| `--detach --detach` | repeated selector, ae:16748-16750 | ACCEPTED | SC-013 |
| `--init` | ae:16732-16738 | ACCEPTED | OUT-OF-BATCH — SC-932 |
| `--init extra` | ae:16733-16735 | REJECTED | OUT-OF-BATCH — SC-932 |
| `--attach` / `--switch` | ae:16744-16746 | ACCEPTED | OUT-OF-BATCH — SC-931 |
| bare `steward` (no flags) | parser loop never entered | ACCEPTED | OUT-OF-BATCH — SC-930 |
| `hub` spelling | ae:16722 | ACCEPTED | OUT-OF-BATCH — SC-939f |
| a positional argument | ae:16752-16761 | REJECTED | OUT-OF-BATCH — SC-930 |

**Finding for the seats:** `contrib/aesteward` at `72c7293` contains only `CHARTER.md`,
`README.md` and `steward.config` — no executable. Whatever `ae steward` does at these flags
is therefore inside `ae` itself or depends on an artifact the frozen tree does not carry.
SC-013's arm must capture that rather than assume a steward program exists.

## `state` (SC-211a) — ae:12831-12873

| Input | Reaches | Class |
|---|---|---|
| no args | the print-current branch, ae:12836-12845 | ACCEPTED |
| `working` / `waiting-user` / `done` | ae:12853 | ACCEPTED |
| `blocked` with a reason | ae:12854-12859 | ACCEPTED |
| `blocked` with no reason | ae:12855-12857 | REJECTED |
| unknown mode | ae:12860 | REJECTED |
| empty-string mode | `case ""` falls to `*)`, ae:12860 | REJECTED |
| mode with a leading `-` | `*)`, ae:12860 | REJECTED |
| extra words after a legal mode | joined into `reason`, ae:12850 | ACCEPTED |

## `goal` (SC-211b) — ae:14558-14593

| Input | Reaches | Class |
|---|---|---|
| no args | print-current, ae:14560-14566 | ACCEPTED |
| one word | set branch, ae:14577 | ACCEPTED |
| several words | set branch (`text="$*"`), ae:14578 | ACCEPTED |
| `--clear` alone | ae:14568-14572 | ACCEPTED |
| `--clear` + extra arg | ae:14569 | REJECTED |
| `--help` | ae:14574 -> `helper_goal_usage`, ae:14551-14555 | REJECTED (rc 1) |
| `-h` | ae:14574 -> `helper_goal_usage`, ae:14551-14555 | REJECTED (rc 1) |
| text that is only control bytes | stripped to empty, ae:14583-14584 | REJECTED |

## `memo` (SC-211c) — ae:14498-14548

| Input | Reaches | Class |
|---|---|---|
| no args | `cmd` defaults to `read`, ae:14501 | ACCEPTED |
| `add <text>` | ae:14511-14523 | ACCEPTED |
| `add` with no text | ae:14511 | REJECTED |
| `add --topic <t> <text>` | ae:14506-14510 | ACCEPTED |
| `add --topic` with no topic and no text | ae:14507 | REJECTED |
| `add --topic <t>` with a topic but no text | ae:14507 | REJECTED |
| `read` | ae:14525-14534 | ACCEPTED |
| `read --topic <t>` | ae:14527-14529 | ACCEPTED |
| `read --topic` alone | ae:14528 | REJECTED |
| `read <extra>` | ae:14530-14531 | REJECTED |
| `tail` | count defaults, ae:14538 | ACCEPTED |
| `tail <n>` | ae:14539 | ACCEPTED |
| `tail <non-numeric>` | ae:14539 | REJECTED |
| `tail <n> <extra>` | ae:14537 | REJECTED |
| unknown subcommand | ae:14546 | REJECTED |
| any read/tail with no memo file | ae:14533 / ae:14540 | ACCEPTED (early exit 0) |

## `requests` (SC-211d, SC-212c) — ae:14407-14456

| Input | Reaches | Class |
|---|---|---|
| no args | `mode` defaults to `mine`, ae:14409 | ACCEPTED |
| `mine` / `inbox` / `all` | ae:14412 | ACCEPTED |
| unknown mode | ae:14412-14414 | REJECTED |
| `mine` where identity is undetectable | ae:14417-14419 | REJECTED |
| `inbox` where identity is undetectable | ae:14417-14419 | REJECTED |
| `all` where identity is undetectable | passes the guard, ae:14417 | ACCEPTED |
| extra args after the mode | never read | IGNORED |

## `say` (SC-211l) — ae:14470-14486

| Input | Reaches | Class |
|---|---|---|
| text via argv | ae:14471-14472 | ACCEPTED |
| text via a pipe | ae:14473-14474 | ACCEPTED |
| no args on a real TTY | ae:14476 -> `helper_say_usage`, ae:14459-14468 | REJECTED (rc 2) |
| no args with redirected empty stdin | ae:14473-14474 then ae:14480 | REJECTED |
| whitespace-only text | ae:14480-14481 | REJECTED |

## `peek` (SC-211e) — ae:14596-14615

| Input | Reaches | Class |
|---|---|---|
| no args | ae:14597-14601 | REJECTED |
| a target spelling absent from the fixture | ae:14603 | REJECTED |
| an exact `alias:name` present in the fixture, no count | default 80, ae:14605 | ACCEPTED |
| non-numeric count | ae:14607-14609 | REJECTED |
| negative count (`-5`) | fails the numeric test, ae:14607 | REJECTED |
| leading-plus count (`+5`) | fails the numeric test, ae:14607 | REJECTED |
| `0` | clamped, ae:14612 | ACCEPTED |
| count above 2000 | clamped, ae:14611 | ACCEPTED |
| extra args after the count | never read | IGNORED |

## `agents` (SC-211f) — ae:14618-14638

| Input | Reaches | Class |
|---|---|---|
| no args | current-session branch, ae:14632-14636 | ACCEPTED |
| `--all` | ae:14619-14630 | ACCEPTED |
| any other argument | not `--all`, so the current-session branch | IGNORED |
| `--all` with an unreadable session meta | `grep` on it, ae:14623 | (capture; the `-f` test at ae:14622 passes for a mode-000 file) |

## `focus` (SC-211g) — ae:14641-14654

| Input | Reaches | Class |
|---|---|---|
| no args | ae:14642-14646 | REJECTED |
| a target spelling absent from the fixture | ae:14648 | REJECTED |
| an exact `alias:name` present in the fixture | ae:14651-14653 | ACCEPTED |
| extra args | never read | IGNORED |

## `interrupt` (SC-211h) — ae:14657-14701

| Input | Reaches | Class |
|---|---|---|
| no args | ae:14658-14662 | REJECTED |
| a target spelling absent from the fixture | ae:14664 | REJECTED |
| a present target, no message | ae:14673 | ACCEPTED |
| a present target + message, pane running a shell (H3) | ae:14673-14675 | REJECTED |
| a present target + message, pane running the agent binary (H1) | ae:14685 onward | ACCEPTED |

## `spawn` (SC-211i) — ae:14704-14723

| Input | Reaches | Class |
|---|---|---|
| `AE_PATH` missing or not executable | ae:14711-14716 (BEFORE argument handling) | REJECTED |
| no args | ae:14718-14720 | REJECTED |
| any args | `exec "$AE_PATH" _spawn …`, ae:14722 | delegated to `_cmd_spawn` |

### delegated `_cmd_spawn` (ae:11845-) — the non-name classes SC-211i owns

| Input | Reaches | Class |
|---|---|---|
| an alias not defined in the config | ae:11912 | REJECTED |
| a session absent from meta | ae:11895 | REJECTED |
| a session named in meta but not running | ae:11899 | REJECTED |
| a complete meta/config/session fixture with `meta.lock` HELD BEYOND THE 5s WAIT | ae:11928-11930, which is reached AFTER `tmux new-window` at ae:11920-11921 — the case captures pane/window residue as well as the refusal | REJECTED |
| a legal `alias` with no `:name` | prompt/default path, ae:11879 | ACCEPTED |
| a name violating the agent-name grammar | ae:11857 | OUT-OF-BATCH — SC-1201 |

## `retire` (SC-211j) — ae:14726-14747

| Input | Reaches | Class |
|---|---|---|
| `AE_PATH` missing or not executable | ae:14733-14738 | REJECTED |
| no args | ae:14740-14744 | REJECTED |
| any args | `exec "$AE_PATH" _retire …`, ae:14746 | delegated to `_cmd_retire` |

### delegated `_cmd_retire` (ae:12118-) — the classes SC-211j owns

| Input | Reaches | Class |
|---|---|---|
| a session absent from meta | ae:12135 | REJECTED |
| a `%pane-id` not in the session | ae:12153 | REJECTED |
| a bare name carried by two agents | ae:12175 | REJECTED |
| a name absent from the session | ae:12177 | REJECTED |
| the main agent's own reference | ae:12184-12185 | REJECTED |
| a configured worker (not spawned) | ae:12206 | REJECTED |
| a reference absent from `agent.spawned.*` | ae:12208 | REJECTED |
| an agent reference present in `agent.spawned.*` | proceeds past ae:12203 | ACCEPTED |
| extra arguments after the target | never read | IGNORED |

## `events-tail` (SC-211n) — ae:14885-14905

| Input | Reaches | Class |
|---|---|---|
| any argv at all | never read — no argument handling exists | IGNORED |
| events file absent | `while [[ ! -f ]]; sleep 1`, ae:14897-14899 | HANGS until the file appears |
| events file present | banner, `tail -n 30 -f`, ae:14902 | HANGS by design (follow) |

Every invocation of this surface ends by controller termination. There is no input that
makes it exit on its own.

## `_register-sid` (SC-211o) — ae:14750-14825

| Input / fixture fact | Reaches | Class |
|---|---|---|
| no slot argument | `SLOT` defaults to `default`, ae:14752 | ACCEPTED |
| a slot with no `launch_time.<slot>` | falls back to `codex_launch_time.<slot>`, then 0 | ACCEPTED |
| non-numeric launch time | reset to 0, ae:14760-14762 | ACCEPTED |
| candidate file older than launch time | skipped | (selection fact) |
| candidate with matching launch-id token | token pass | (selection fact) |
| candidate with a mismatched token | token fail | (selection fact) |
| no token match anywhere | CWD-fallback scan | (selection fact) |
| a malformed first-line id | that candidate contributes nothing, ae:14780 | (selection fact) |
| a missing first-line id | that candidate contributes nothing, ae:14780 | (selection fact) |
| yesterday's directory | scanned after today's, ae:14770 | (selection fact) |
| two candidates with EQUAL mtimes | the scan compares with strict `>`, ae:14783 | (selection fact) |
| two candidates with DIFFERENT mtimes | same comparison, ae:14783 | (selection fact) |
| a candidate whose recorded cwd MATCHES the invoking cwd | fallback scan, ae:14803 | (selection fact) |
| a candidate whose recorded cwd DIFFERS from the invoking cwd | fallback scan, ae:14803 | (selection fact) |
| an explicitly named slot that is the invoking pane's | `SLOT` from argv, ae:14752 | (input class) |
| an explicitly named slot that is NOT the invoking pane's | `SLOT` from argv, ae:14752 | (input class) |

Selection facts are inputs to vary, not outcomes; the arm captures which sid is written.

**Scope statement.** The slot argument is treated as TRUSTED INTERNAL input in this batch:
`_register-sid` is launched by ae itself, not by a peer, so path- or regex-hostile slot
values are out of scope here. If that scope is ever wrong, the hostile-slot class belongs
to the identity boundary rows, not to SC-211o.

## `ae_resolve` (SC-211p) — ae:12878-12991

| Input | Reaches | Class |
|---|---|---|
| `%<pane-id>` (any) | branch 1, ae:12885-12901 | ACCEPTED (returns 0) |
| `@session:agent`, session exists, agent present | branch 2 then 3 | ACCEPTED |
| `@session:agent`, session absent | ae:12919-12921 | REJECTED |
| `@session` (no colon) | ae:12907-12909 | REJECTED |
| `@:agent` | ae:12914-12916 | REJECTED |
| `@session:` | ae:12914-12916 | REJECTED |
| bare name, unique | branch 3 | ACCEPTED |
| bare name, ambiguous | ae:12975-12976 | REJECTED |
| alias-only, unique | branch 3 | ACCEPTED |
| alias-only, ambiguous | ae:12975-12976 | REJECTED |
| exact `alias:name` present | branch 3 | ACCEPTED |
| name absent | ae:12978 | REJECTED |
| empty string | branch 3 with an empty target | (capture) |

## Usage-exit family (seat-only) — measured across the whole generated helper set

Raised because one row's reading is not legible beside nothing. Each cell below is the
EXIT LINE ITSELF, read inside the function or block, and cited at its own line — not at the
opening brace, which is what a line citation for a function points at. A citation is not a
reading.

| Surface | Usage path | Exit line | Exit |
|---|---|---|---|
| `state` | dedicated function, `helper_state_usage` | ae:12828 | 2 |
| `say` | dedicated function, `helper_say_usage` | ae:14467 | 2 |
| `goal` | dedicated function, `helper_goal_usage` | ae:14555 | 1 |
| `memo` | dedicated function, `helper_memo_usage` | ae:14491 | 1 |
| `requests` | inline block | ae:14414 | 1 |
| `peek` | inline block | ae:14601 | 1 |
| `focus` | inline block | ae:14646 | 1 |
| `interrupt` | inline block | ae:14662 | 1 |
| `spawn` | inline block | ae:14720 | 1 |
| `retire` | inline block | ae:14744 | 1 |

Eight exit 1 and two exit 2. **No explanation of the split is offered here.** An earlier
version of this section asserted that the exit-2 surfaces were the ones with a dedicated
usage function; that is REFUTED by the same table — `goal` and `memo` are dedicated
functions and exit 1. A stated correlation is a claim even when it is framed as shape
rather than intent, and a seat would have reasoned from it. If a shape claim is wanted it
is a seat act, taken with all ten rows in front of it.

The arms capture each surface's usage rc; the batch record places them side by side so the
family is read as a family.

## Row -> input-class mapping (for the seat gate)

| Row | Surface | Input classes |
|---|---|---|
| SC-012b | dispatcher | help trio + unknown LONG OPTION + non-option launch candidate |
| SC-014 | dispatcher | version trio |
| SC-013 | `steward` | help spellings + detach spellings + help-with-trailing-args + selector order/repetition (the OUT-OF-BATCH rows belong to SC-930/931/932/939f) |
| SC-211a | `state` | see the generated list |
| SC-211b | `goal` | see the generated list |
| SC-211c | `memo` | see the generated list |
| SC-211d, SC-212c | `requests` | see the generated list |
| SC-211e | `peek` | see the generated list |
| SC-211f | `agents` | see the generated list |
| SC-211g | `focus` | see the generated list |
| SC-211h | `interrupt` | see the generated list |
| SC-211i | `spawn` + delegated `_cmd_spawn` | see the generated list |
| SC-211j | `retire` + delegated `_cmd_retire` | see the generated list |
| SC-211l | `say` | see the generated list |
| SC-211n | `events-tail` | see the generated list |
| SC-211o | `_register-sid` | see the generated list |
| SC-211p | `ae_resolve` | see the generated list |
| D14b | held pending record correction | — |
| SC-1301 | three meta writers | per-writer cuts, not argument classes |
