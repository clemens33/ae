# Batch H — executor input list (BRIEF-FACING)

Derived mechanically from the seat-facing argument census by dropping every column
that names an outcome. What each input DOES is not stated here and must not be
inferred: the arm invokes it and captures what happens.

One row per input class. Where a row names a fixture property rather than an argv,
it is a fixture fact to construct, not an argument to pass.


## Surface: Top-level dispatcher (SC-012b, SC-014)

- `-h` / `--help` / `help`
- `version` / `--version` / `-V`
- an unknown first word

## Surface: `steward` (SC-013)

- `--init`
- `-h` / `--help` / `help`
- `--attach` / `--switch`
- `--detach` / `--no-attach`
- unknown flag
- `hub` instead of `steward`

## Surface: `state` (SC-211a)

- no args
- `working` / `waiting-user` / `done`
- `blocked` with a reason
- `blocked` with no reason
- unknown mode
- empty-string mode
- mode with a leading `-`
- extra words after a legal mode

## Surface: `goal` (SC-211b)

- no args
- one word
- several words
- `--clear` alone
- `--clear` + extra arg
- `--help` / `-h`
- text that is only control bytes

## Surface: `memo` (SC-211c)

- no args
- `add <text>`
- `add` with no text
- `add --topic <t> <text>`
- `add --topic` with no topic/text
- `read`
- `read --topic <t>`
- `read --topic` alone
- `read <extra>`
- `tail`
- `tail <n>`
- `tail <non-numeric>`
- `tail <n> <extra>`
- unknown subcommand
- any read/tail with no memo file

## Surface: `requests` (SC-211d, SC-212c)

- no args
- `mine` / `inbox` / `all`
- unknown mode
- `mine`/`inbox` where identity is undetectable
- `all` where identity is undetectable
- extra args after the mode

## Surface: `say` (SC-211l)

- text via argv
- text via a pipe
- no args on a real TTY
- no args with redirected empty stdin
- whitespace-only text

## Surface: `peek` (SC-211e)

- no args
- unresolvable target
- resolvable target, no count
- non-numeric count
- negative count (`-5`)
- leading-plus count (`+5`)
- `0`
- count above 2000
- extra args after the count

## Surface: `agents` (SC-211f)

- no args
- `--all`
- any other argument
- `--all` with an unreadable session meta

## Surface: `focus` (SC-211g)

- no args
- unresolvable target
- resolvable target
- extra args

## Surface: `interrupt` (SC-211h)

- no args
- unresolvable target
- target, no message
- target + message, pane is a shell
- target + message, live agent pane

## Surface: `spawn` (SC-211i)

- `AE_PATH` missing or not executable
- no args
- any args

## Surface: `retire` (SC-211j)

- `AE_PATH` missing or not executable
- no args
- any args

## Surface: `events-tail` (SC-211n)

- any argv at all
- events file absent
- events file present

## Surface: `_register-sid` (SC-211o)

- no slot argument
- a slot with no `launch_time.<slot>`
- non-numeric launch time
- candidate file older than launch time
- candidate with matching launch-id token
- candidate with a mismatched token
- no token match anywhere
- malformed / missing first-line id
- yesterday's directory

## Surface: `ae_resolve` (SC-211p)

- `%<pane-id>` (any)
- `@session:agent`, session exists, agent present
- `@session:agent`, session absent
- `@session` (no colon)
- `@:agent`
- `@session:`
- bare name, unique
- bare name, ambiguous
- alias-only, unique
- alias-only, ambiguous
- exact `alias:name` present
- name absent
- empty string


