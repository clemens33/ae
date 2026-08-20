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

**Executor scoping.** Per seat ruling the executor receives
`batch-h-input-list.md` — the same input classes with no class labels and no outcome
citations. This document stays with the seats.

## Top-level dispatcher (SC-012b, SC-014)

| Input | Reaches | Class |
|---|---|---|
| `-h` / `--help` / `help` | one arm, ae:16841-16843 | ACCEPTED |
| `version` / `--version` / `-V` | one arm, ae:16845-16847 | ACCEPTED |
| an unknown first word | the `*)` arm of the dispatcher | REJECTED |

## `steward` (SC-013) — a wider surface than the row's wording implies

`steward | hub)` at ae:16722; `hub` is a retained deprecated alias, so the row's surface has
two spellings before any flag is considered.

| Input | Reaches | Class |
|---|---|---|
| `--init` | ae:16722 sub-case | ACCEPTED |
| `-h` / `--help` / `help` | sub-case | ACCEPTED |
| `--attach` / `--switch` | sub-case | ACCEPTED |
| `--detach` / `--no-attach` | sub-case | ACCEPTED |
| unknown flag | sub-case `*)` | REJECTED |
| `hub` instead of `steward` | same arm | ACCEPTED |

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
| `--help` / `-h` | ae:14574 | REJECTED (usage, rc 2 via `helper_goal_usage`) |
| text that is only control bytes | stripped to empty, ae:14583-14584 | REJECTED |

## `memo` (SC-211c) — ae:14498-14548

| Input | Reaches | Class |
|---|---|---|
| no args | `cmd` defaults to `read`, ae:14501 | ACCEPTED |
| `add <text>` | ae:14511-14523 | ACCEPTED |
| `add` with no text | ae:14511 | REJECTED |
| `add --topic <t> <text>` | ae:14506-14510 | ACCEPTED |
| `add --topic` with no topic/text | ae:14507 | REJECTED |
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
| `mine`/`inbox` where identity is undetectable | ae:14417-14419 | REJECTED |
| `all` where identity is undetectable | passes the guard, ae:14417 | ACCEPTED |
| extra args after the mode | never read | IGNORED |

## `say` (SC-211l) — ae:14470-14486

| Input | Reaches | Class |
|---|---|---|
| text via argv | ae:14471-14472 | ACCEPTED |
| text via a pipe | ae:14473-14474 | ACCEPTED |
| no args on a real TTY | ae:14476 | REJECTED (usage, rc 2) |
| no args with redirected empty stdin | ae:14473-14474 then ae:14480 | REJECTED |
| whitespace-only text | ae:14480-14481 | REJECTED |

## `peek` (SC-211e) — ae:14596-14615

| Input | Reaches | Class |
|---|---|---|
| no args | ae:14597-14601 | REJECTED |
| unresolvable target | `ae_resolve` non-zero, ae:14603 | REJECTED |
| resolvable target, no count | default 80, ae:14605 | ACCEPTED |
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
| unresolvable target | ae:14648 | REJECTED |
| resolvable target | select-window/select-pane + event, ae:14651-14653 | ACCEPTED |
| extra args | never read | IGNORED |

## `interrupt` (SC-211h) — ae:14657-14701

| Input | Reaches | Class |
|---|---|---|
| no args | ae:14658-14662 | REJECTED |
| unresolvable target | ae:14664 | REJECTED |
| target, no message | skips the dead-pane guard by design, ae:14673 | ACCEPTED |
| target + message, pane is a shell | ae:14673-14675 | REJECTED |
| target + message, live agent pane | paste path, ae:14685 onward | ACCEPTED |

## `spawn` (SC-211i) — ae:14704-14723

| Input | Reaches | Class |
|---|---|---|
| `AE_PATH` missing or not executable | ae:14711-14716 (BEFORE argument handling) | REJECTED |
| no args | ae:14718-14720 | REJECTED |
| any args | `exec "$AE_PATH" _spawn …`, ae:14722 | delegated — outcome belongs to `_cmd_spawn` |

Name-grammar inputs are SC-1201's (F-IDENTITY) and are not part of this row.

## `retire` (SC-211j) — ae:14726-14747

| Input | Reaches | Class |
|---|---|---|
| `AE_PATH` missing or not executable | ae:14733-14738 | REJECTED |
| no args | ae:14740-14744 | REJECTED |
| any args | `exec "$AE_PATH" _retire …`, ae:14746 | delegated — outcome belongs to `_cmd_retire` |

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
| malformed / missing first-line id | that candidate contributes nothing | (selection fact) |
| yesterday's directory | scanned after today's | (selection fact) |

Selection facts are inputs to vary, not outcomes; the arm captures which sid is written.

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

## Row -> input-class mapping (for the seat gate)

| Row | Surface | Input classes |
|---|---|---|
| SC-012b | dispatcher | help trio + unknown word |
| SC-014 | dispatcher | version trio |
| SC-013 | `steward`/`hub` | six classes above |
| SC-211a | `state` | eight |
| SC-211b | `goal` | seven |
| SC-211c | `memo` | fifteen |
| SC-211d, SC-212c | `requests` | six |
| SC-211e | `peek` | nine |
| SC-211f | `agents` | four |
| SC-211g | `focus` | four |
| SC-211h | `interrupt` | five |
| SC-211i | `spawn` | three (name grammar excluded — SC-1201) |
| SC-211j | `retire` | three |
| SC-211l | `say` | five |
| SC-211n | `events-tail` | three, all long-lived |
| SC-211o | `_register-sid` | nine fixture facts |
| SC-211p | `ae_resolve` | thirteen |
| D14b | held pending record correction | — |
| SC-1301 | three meta writers | per-writer cuts, not argument classes |
