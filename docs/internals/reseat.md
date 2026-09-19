# Moving a seat to another tool (`ae reseat`)

The verb is `src/reseat.rs`. It owns the argv, the refusal ladder and the
order; it owns no liveness rule and no paste of its own.

## What it reuses, and why that is the point

`relaunch` already answers "is this seat's tool provably gone, and may ae
paste into its pane". A reseat asks exactly the same question — the only
difference is what gets pasted afterwards — so `src/seat_relaunch.rs` keeps
that answer and lends it out:

| Borrowed | What it decides |
|---|---|
| `prove_dead` | the whole dead-proof ladder: the pane exists, is not dead, carries a slot, records a tool, has a usable work dir, resolves a seat command, is not running that tool, is not busy, and reads `Dead` to the one liveness owner |
| `usable_work_dir` | the session's recorded working copy, or the one refusal that names it gone — shared because the stop needs the same answer BEFORE it kills anything |
| `identity_proven` | whether the seat's own tool holds the pane, which is how the stop decides there is anything to stop |
| `start` | the launch-attempt stamp, the capture floor, the input-line clear, the `cd` + `_run` paste, and the tool-identity poll |
| `observe_identity` | one reading: the pane's foreground always, the process walk when the caller is spending a snapshot |
| `turn_verdict` | what a turn outcome means for the exit code |

They differ in ONE word. `Verb { imperative, past }` is what a refusal names,
so `nothing to relaunch` and `nothing to reseat` are the same sentence rather
than two copies of a ladder that would drift. `RELAUNCH_VERB` keeps every
existing refusal byte-identical.

## The order

Everything durable is answered from the session's records, before tmux is
consulted at all — a stopped session diagnoses a typo exactly as a running one
does, and a live seat is only reached at the dead proof.

1. the argv: `<session> <agent> --using <profile> [--stop-unknown]`, either
   flag anywhere, a `profile@client` override refused as launch-only;
2. the session, in the world the caller already enumerated;
3. the caller rule: a pane ae STAMPED may reseat only its own session; an
   unstamped shell may reseat any;
4. the roster: which slot seats this agent, and what profile it records;
5. the profile differs from the recorded one (`relaunch` is the other verb);
6. the profile resolves — the same resolution `run::read_seat` makes for a
   seat with no client override, so a profile accepted here is one `_run` can
   launch. No tool-class judgement: a reseat accepts exactly the profiles a
   launch does;
7. the pane: not the CALLER'S own — a seat cannot reseat itself, because the
   tool running the command is the one that would be replaced under it — and
   its stamp must agree with the roster slot.

Then, under `.lifecycle.<session>.lock`:

8. the STOP (`stop_running_tool`), for a seat whose tool is still running. A
   seat already gone returns straight to step 9 having read nothing but the
   pane, and every arm here refuses BEFORE the respawn, which is the one
   write: the working copy (a durable fact the records already carry, and
   refusing on it after a kill would be the worst order this verb could
   take); the caller proven not to be running UNDER the target, because the
   pane check at step 7 is only half that rule — a process the target's own
   tool started carries no `$TMUX_PANE` to compare, so `procs::is_descendant_of`
   walks the tree, and a missing pid or unusable table REFUSES; the pane's
   SEND-LOCK, asked with a short wait of its own because the lifecycle lock is
   already held; then the FRAME.

   `harness_state::classify` is the whole frame rule, read TWICE `FRAME_GAP`
   apart. `Busy` refuses always; anything not `Idle` on both readings refuses
   unless `--stop-unknown`; a capture ae could not take is `Unknown`, never
   idle. `deliver`'s busy reading is deliberately not used — it answers
   whether the input box holds text, which is FALSE mid-turn. Only claude's
   and codex's grammars are owned: muse declares claude's input model and
   draws its own frame, so it reads `Unknown` and fails closed (pinned on a
   measured muse capture), every unmodelled composer is `Unknown` always, and
   even claude proves `Idle` from the pane's recent output, so a seat that has
   not finished a turn since it started needs the flag.

   Then `respawn-pane -k -c <work_dir>` through `session_tmux::Op::RespawnPane`
   — the op the monitor panes already use, on the existing tmux door. Measured
   on tmux 3.7b: the pane id, its `@ae_*` options, its index and its scrollback
   survive; its process tree does not; tmux's default shell comes back in the
   `-c` directory. A tmux that refuses the argv leaves the tool running and
   says so in its own words. A BOUNDED 10 s wait for `pane_back_at_shell`
   follows — a shell in the foreground with nothing under it, every gap false
   so it times out rather than call a pane it could not read idle — and on
   timeout the refusal names what holds the pane with the meta untouched.
   Last, the stop's own event, written once the pane is PROVEN back at its
   shell and never before, naming the binary ended because the meta keeps only
   the current one.
9. `prove_dead`;
10. the seed pack is BUILT — before anything is removed, because it reads the
    seat's recorded first message and step 11 deletes that file;
11. the seed is published as `seed.<agent>.md` (0600), then `run::clear_slot`
    removes the start marker, the prompt file, the launch script and the
    per-tool id files, so the successor's `_run` has no stale first message
    and no resume decision to make;
12. ONE guarded meta replacement (`meta::publish_seat_move`);
13. `read_seat` again — the command, the tool and the conversation are the new
    profile's now — then `start`.

The lock is DROPPED before any readiness wait, because a gated turn blocks up
to 45 s and no session may be held out of its own lifecycle for that. Past it:

14. the tool's OWN launch turn where its adapter has one (codex's rollout does
    not exist until a user turn, and that registration handshake must not sit
    under a 24 KB seed);
15. the seed, as `⟦ae:ctx⟧` — `provenance` puts the marker on, because
    `deliver_launch_turn` pastes its text verbatim;
16. the post-launch capture for a slot that still reads `pending`;
17. a LAST identity reading, because exit 0 means the seat is up NOW.

## The meta move

`meta::reseated` is pure; `meta::seat_move_for` is the guard; only
`publish_seat_move` touches a file. The guard compares `seat.<slot>` to the
agent the caller proved: a reseat holds the lifecycle lock so no second reseat
interleaves, but a `retire` plus a re-`spawn` can land a DIFFERENT seat on the
slot, and that successor must not inherit the move.

WRITES `profile`, `agent_bin`, `harness_session` (a fresh UUID where the tool
takes one at launch, `pending` otherwise), `launch_id`, `capture_floor`, and
appends the old conversation to `harness_session_prior` TAGGED with the tool
that owns it — the whole point of the tag, since the successor's tool reads a
different store.

REMOVES `config_home` and `config_home_base` (they belong to the tool that is
leaving; `_run` records the new tool's own at its first start, and a stale row
would point the successor's reader at the predecessor's store), `launch_time`
(it would date a launch that has not happened), `observed_model` and its pin
(they would read as drift the moment the new tool answers), and `client` (a
launch's `profile@client` override, for a profile this seat no longer runs).
The client row is removed rather than emptied: an empty override is not the
same fact as no override.

A conversation the tagger cannot prove is not handed on: `prior_with` refuses
an id it cannot judge rather than recording a guess, so a seat whose capture
never completed leaves no predecessor row.

## The three crash windows

None is atomic and none needs to be — all are finished by hand with a verb
that already exists, and nothing before the stop needs undoing.

| Interrupted | What is on disk | The way forward |
|---|---|---|
| between the stop and the slot cleanup | old profile recorded, launch files intact, pane at its shell | `relaunch <agent>` — the seat comes back on the tool it had |
| between the slot cleanup and the meta write | old profile recorded, no start marker, no launch files | `relaunch <agent>` — the seat comes back on the tool it had |
| between the meta write and the tool starting | new profile recorded, pane at its shell | `relaunch <agent>` — the refusal says so by name |

A stop with no record is the shape ae prefers to a record with no stop: the
first is recovered from the meta, the second would mislead every later reader.

## What the stop does NOT cover

The watchdog's dead verdict has no grace and no launch awareness
(`watchdog::classify_dead`), so a session with a live watchdog can book one
`agent process dead — dropped to shell` alert, and its Telegram notice, inside
the ~1–20 s the pane sits at its shell. This is not new — `spawn` and
`relaunch` leave the same window — it is non-destructive, and the latch clears
with one `dead-cleared` on the next cycle that sees the successor. The stop itself writes no
stamp a grace could key on — `.launch-attempt` is written by
`seat_relaunch::start`, pre-paste, which is AFTER this window opens — so
closing it would mean the daemon learning a stop-time fact it is not told
today. That is that owner's change to make, not this verb's.

## The seed

ONE builder: `lib.rs::seat_pack` renders the pack, `run_seat_pack` prints it
for `ae brief --seat`, and this verb pastes it. What a human reads before a
move and what the successor is given are therefore the same document.

Because it is PASTED, every field the pack takes out of a record goes through
`seatpack::neutralise`, which is also what strips control bytes: an escape
sequence, a bell or a bracketed-paste terminator in a memo, a state reason or
a quoted turn would be KEYSTROKES in the successor's pane. The pin that keeps
that honest poisons every input field and reads the whole rendered document
back looking for any control character but a newline.

ONE field is proven rather than cleaned: a request id, which the pack also
prints into a `reply` command, so a cleaned one would name a request that does
not exist. `seatpack::minted_requests` drops a row failing
`tracked::is_request_id` whole and counts it.

A predecessor list is where a record is lost QUIETLY: `meta::prior_with` restarts
from the new id alone when the row is damaged or over cap, so a row the 2->3
migration preserved byte-for-byte goes on the next reseat or capture, reported on
no surface — no event, no coverage line, no refusal. Per-element salvage would
match the reader; the writer does not.

The seed file is KEPT after a successful move. It is what a human re-sends by
hand when a turn did not land — the refusal prints that exact command — and it
is the record of what the successor was actually told.

## What it does not reset, and why

`@ae_observed` is a pane option this verb deliberately leaves alone, although
the seat behind it has just changed tool. Three facts make that safe, and all
three have to hold:

- the **watchdog is its one writer**, and it republishes the option every
  cycle from that cycle's own live capture;
- its one **reader that acts** is the same watchdog, restoring its own
  cross-restart idle latch — so a stale value costs at most one cycle and then
  heals itself. Every other reader DRAWS it: `ae list`, the `--json` digest,
  the liveness snapshot that carries it onward;
- **delivery never trusts it.** Every paste is gated by a LIVE capture through
  `deliver::input_ready` / `wait_input_ready`, never by a recorded harness
  state, so a stale option cannot let a turn into a pane that is not ready.

If any of that changes — in particular if a reader ever ACTS on the option
without taking its own capture — this verb owes a reset, and the note is the
place that says so.

## What it does not do

It does not end a harness ae cannot read. The blast radius of stopping a tool
mid-turn is the turn's work, so a frame that is not positively IDLE is a
refusal and `--stop-unknown` is the caller saying they meant it — there is no
`-f`, and a frame that reads BUSY refuses whatever is passed.

It does not recreate a pane, and it does not touch anything in the session but
the seat named on the argv.
