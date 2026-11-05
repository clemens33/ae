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

1. the argv: `<session> <agent> --using <profile[@client]> [--stop-unknown]`,
   either flag anywhere; `profile@client` splits once at `@` (the grammar a
   launch `--seat` and a spawn share), and a malformed value is refused naming it;
2. the session, in the world the caller already enumerated;
3. the caller rule: a pane ae STAMPED may reseat only its own session; an
   unstamped shell may reseat any;
4. the roster: which slot seats this agent, and what profile and client it records;
5. something moves (`relaunch` is the other verb): the profile differs, or the
   same profile asks for another client — a pair for a label the seat does not
   record, or a BARE profile on a seat that records a usable client label,
   which returns it to the profile's own account (the one named exception to
   "a bare reseat behaves as before"). A bare profile on a seat with no client
   row, or one whose row ae cannot read, is refused as `already runs`;
6. the profile resolves — bare: the same resolution `run::read_seat` makes for a
   seat with no client override, so a profile accepted here is one `_run` can
   launch. No tool-class judgement: a reseat accepts exactly the profiles a
   launch does. PAIR: the spawn's own gate, `session_launch::approved_pair` —
   the session's recorded GLOBAL config alone (a project overlay can neither
   approve a pair nor redefine an approved name), `[client_pairs]` approval
   before existence, the same-known-adapter gate and the first-start store
   proof — then the account the pair selects, which ae must be able to name.
   Any move onto a seat whose client row ae cannot read is refused here too,
   with that read's words; a pair move skips the auto-reseat cover warning,
   which judges the profile's own account;
7. the pane: not the CALLER'S own — a seat cannot reseat itself, because the
   tool running the command is the one that would be replaced under it — and
   its stamp must agree with the roster slot.

Then, under `.lifecycle.<session>.lock`:

7a. the CARRY CHECK and the TOOL-CHANGE NOTE, before anything is stopped. A
    move that will carry (the ladder below, step 1) walks the whole copy set
    with `carry::Mode::Check` — the copy's own walk, every node classified,
    read and compared, nothing created — and a failure refuses here, exit 1,
    nothing stopped, written or journaled. What it decided to ARCHIVE is kept
    as its `carry::Proof` and handed to the copy in step 9a. A tool change (recorded `agent_bin`
    differs from the new binary) prints ONE stderr note instead, a dead pane
    included: if the move succeeds it starts a fresh conversation, a recorded
    id that passes the UUID grammar stays behind as a predecessor, and ae
    searches for no conversation the records do not name. A working copy ae
    cannot use skips the Check; step 8 or 9 names it;
8. the STOP (`stop_running_tool`), for a seat whose tool is still running. A
   seat already gone returns straight to step 9 having read nothing but the
   pane, and every arm here refuses BEFORE the respawn, which is the one
   write: the working copy (a durable fact the records already carry, and
   refusing on it after a kill would be the worst order this verb could
   take); the caller proven not to be running UNDER the target, because the
   pane check at step 7 is only half that rule — a process the target's own
   tool started carries no `$TMUX_PANE` to compare, so `procs::lineage`
   walks the tree, and a missing pid, an unusable table, a missing row for the
   pane or the caller, or an ancestry broken before either REFUSES by name; the
   TREE, from that same snapshot: a known harness the seat does not record
   anywhere in it, or one ae cannot place outside it (`procs::harness_rows`),
   REFUSES by tool, pid and parent, because the respawn ends the whole tree,
   and no flag lifts it; the pane's
   SEND-LOCK, asked with a short wait of its own because the lifecycle lock is
   already held; then the FRAME.

   `harness_state::classify` is the whole frame rule, read TWICE `FRAME_GAP`
   apart. `Busy` refuses always; anything not `Idle` on both readings refuses
   unless `--stop-unknown`; a capture ae could not take is `Unknown`, never
   idle. `deliver`'s busy reading is deliberately not used — it answers
   whether the input box holds text, which is FALSE mid-turn. Only claude's
   and codex's grammars are owned: muse declares claude's input model and
   draws its own frame, so it reads `Unknown` and fails closed (pinned on a
   measured muse capture; only its limit stop, below, has a proof), every unmodelled composer is `Unknown` always, and
   even claude proves `Idle` from the pane's recent output, so a seat that has
   not finished a turn since it started needs the flag. The other proof is the
   vendor's usage limit on BOTH readings over an empty box (`limit_proven`):
   with an `Unknown` verdict the limit row must be the nearest row above the
   box, never one left in the transcript, or claude 2.1.296's own measured
   `⏺ Automatic continue cancelled · /rate-limit-options to re-arm` row (a limit
   wait the human cancelled, a finished turn), or the cell still STANDS
   (`watchdog::limit_standing`): only blank rows, user-echo blocks and the
   vendor's own limit furniture follow it down to the box. A message delivered
   to a limited seat is echoed under the cell, and claude 2.1.317 draws its
   `⏺ Usage limit reached · continuing automatically …` head under the cell
   (2.1.296 turn 1 as well), so neither is a turn; the nearest-row rule alone
   refused both. The cell ends at its first blank or non-indented row. An echo
   is a column-0 `❯` with text, then rows that are blank or indented two
   spaces or more and do not open with a transcript mark glyph (`⎿ ⏺ ● ✻ ✽ ✶
   ✳ ✢ ·`): the blanks between a pasted body's paragraphs sit INSIDE the echo
   (measured) and never close it, a column-0 row always does. Furniture is a
   column-0 `⏺ Usage limit reached…` head, the exact cancelled-wait row, or a
   row the frame grammar reads as finished (`✻ … · done`). Every other row —
   an assistant or tool row, a spinner, a quoted `⎿` — refuses, and it is the
   refusal's first clause. The scan reads the same 20-row bottom window as the
   notice, so a delivery echo long enough to push the cell out of it is
   `no limit row` while the echo is held (claude 2.1.317 fails each delivery's
   turn and draws a fresh cell under its echo, which brings the cell back).
   claude 2.1.296 draws a `⚠ Usage limit
   reached` row and its `Continuing automatically at` row BELOW the box, which
   the frame grammar reads as footer there only. The unreadable-frame refusal
   leads with `frame not proven (<why>)` — `no frame` (no capture, a draft or
   queued text in the box, unreadable chrome), `no limit row` (no cell in the
   window, or no empty box under it) or `row after the limit row: "<row>"`
   (the first row that could mean work, trimmed, at most 80 chars, quotes
   removed), from the first reading that proves nothing, or `the frame changed
   between the two readings` when each reading proves but not alike (an idle
   box, then a limit proof) — then the exact
   `ae reseat … --stop-unknown` command and its precondition, because a
   journal record keeps only the line's head, and the reason must be in it.

   Muse has a limit proof of its own, and only that (`watchdog::limit_standing(
   buf, "muse")`; its frames still classify `Unknown`, idle ones included, so
   `limit_proven` skips the claude grammar for it). A Muse seat that hit its
   quota stops on its limit block (muse-wonder-full-1.0, measured on two
   limited seats): the newest transcript block is the column-0 `◆ Usage limit
   reached…` head with its indented `This turn stopped because your usage limit
   was reached.` row and the known follow-ups (`Your usage resets at …`, `Check
   /usage for details.`, `Send a message to continue when usage is available.`),
   followed only by blank rows and the quota warning (`⚠ Usage nearly
   exhausted…`, `This task may stop before it finishes.`), the `── Voice input`
   rule (a labelled `── Voice input (<hint>) ──…` drawn out to the closing rule's
   width), an EMPTY `❯` row, the closing rule, and below it the model footer
   (`model · effort · path · mode`) ALONE: the model one of the six ids the
   fleet's muse profiles run (`watchdog::MUSE_MODELS`, the `-contributor` ids
   included, because a contributor seat has been measured drawing the base id),
   effort `max` or `xhigh`, a `~` or `/` path and the `YOLO` mode every profile
   sets. A NEW Muse model id must be added to that list; until it is, the stop
   fails closed (`row after the limit row: "<footer>"`). Muse does not continue by itself, so
   a move is the only way off. The head is the phrase alone or followed by ` · ` and its tail (`reachedElsewhere`
   is no head). The per-turn `model failed: API error 429` row is
   no stop here. Any other row refuses and is quoted (`row after the limit row:
   "<row>"`: an unknown follow-up, a row after the block, a second or unknown
   row below the box), a draft or a second-line draft is `no empty box under
   the limit row`, a block missing or pushed out of Muse's 16-row bottom window
   is `no limit row`, and ANY `└` row below the box — a background task,
   running, watching or finished, because the stop kills what it watches — is
   `background task: "<row>"`. The scan reads the final frame only, so a
   Muse version that draws these rows differently refuses until its frame is
   measured (named residual: a task list long enough to push the block out of
   the window refuses as `no limit row`).

   Agy has a limit proof of its own too, and only that (`watchdog::limit_standing(
   buf, "agy")`; its input is unmodelled, so it still classifies `Unknown`, and
   headroom moves, effort and a plain reseat are as they were). On BOTH readings
   the capture's tail must be the newest column-0 `⚠ Individual quota reached…`
   row (the glyph, one space), the `Error ID: ` row directly under it, then only
   blank rows and exactly the measured empty composer (rule, an empty `>` row,
   rule, `? for shortcuts` last, read by `deliver::region::composed_ui`). Any other
   row refuses and is quoted (`row after the limit row: "<row>"`), a draft or a
   wrapped draft is `no empty box under the limit row`, and the Error ID row ends
   the cell, so work abutting it refuses. A limit row agy wraps over several rows
   (its changelog says it wraps alerts itself, so a pane narrower than the row
   draws it so) refuses as a row after the limit row, and a limit row with a blank
   or no row under it (no `Error ID:` row) is `no empty box under the limit row`;
   the step is `--stop-unknown`. An agy seat that is not on its limit keeps its
   `no frame` reason. Named residual: no live capture of the rows under the
   limit exists, so a frame that draws anything else there refuses until it is
   measured. The reset in the row is relative and never read: an agy limit has no
   known reset, so the limit trigger moves the seat.

   A background task is kept unless a LONG limit proves it may end (#304). When
   the reading that decides the refusal shows nothing but the stop and its
   tasks (`watchdog::muse_background_tasks`: below the box an optional
   `main · ↓ to select`, one or more `└ <glyph> <title>` rows and the model
   footer last, nothing else, in that order), the refusal is class
   `background-task` and its step reads `once its box is empty and its
   background task may end, run: … --stop-unknown`. A task's identity is its
   title, cut at the first run of two spaces: its glyph (`◇`, `◈`, `◆`), status
   and elapsed time change on their own. Any other row beside a task keeps
   `unknown-frame`.

   An auto reseat move of a LIMIT episode (never a headroom one, never
   `ae reseat` by hand) may stop such a seat, ending its tasks, when ALL of this
   holds (`autoreseat::long_limit_stop`, both bounds inclusive): both readings
   show the same tasks in the same order; each reading's own vendor row
   (`watchdog::muse_limit_reset`) states the same absolute reset, at least
   `LONG_LIMIT_SECS` (12 h) ahead; and the episode's key is at least
   `BACKGROUND_GRACE_SECS` (10 min) old. That vendor row is Muse's column-0
   `◆ model failed: API error 429 [request_id=<uuid>]: Subscription quota
   exhausted. Your usage window resets at <YYYY-MM-DDTHH:MM:SSZ>.
   (rate_limit_error)`, joined from the indented rows Muse wraps it over (three
   at 80 columns, measured), whole, read by the same sentence reader as a
   failed run's stdout (`quota::muse::limit_reason`). It must be the newest such
   row above the newest stop head, with ONLY blank rows and user echoes (a
   column-0 `❯ <text>` row and the blank or indented rows after it) between
   them: a composer, a footer, a task, transcript or shell row there may be an
   earlier incarnation's screen left in the scrollback. Bounded guarantee: this
   refuses every residue shape measured; an old incarnation whose residue is
   ONLY blank rows and echoes is not told apart (after an in-place relaunch onto
   another Muse login, its reset would be lent to this one). A local time in
   the stop block (`Your usage resets at Nov 9 at 1:00 AM`) names no year or
   zone and is never read, and Muse has no quota identity to join a cache
   reading to, so the pane row is the only instant.

   The proof is taken again at the stop's LAST word: after the caller's hook,
   under the lifecycle and send locks, immediately before `respawn-pane`, a
   THIRD reading must prove what the two did at a fresh clock (a draft, a new
   turn, another task or reset refuses), the server's clients must be readable,
   and no client viewing the pane may have given input since the attempt
   opened, or carry an activity ae cannot read: the send-lock does not hold the
   keyboard. Any gap there refuses `background-task` and nothing is stopped.
   With `auto_reseat_grace_secs` under 600 the first attempt refuses (`its
   limit is under 10 min old`) and the move comes a round later, after
   `RETRY_COOLDOWN_SECS`; it is never promised at ten minutes. A limit under
   12 h away refuses (`its limit resets in under 12 h`) as any other round.

   Then `respawn-pane -k -c <work_dir>` through `session_tmux::Op::RespawnPane`
   — the op the monitor panes already use, on the existing tmux door. Measured
   on tmux 3.7b: the pane id, its `@ae_*` options, its index and its scrollback
   survive; its process tree does not; tmux's default shell comes back in the
   `-c` directory. A tmux that refuses the argv leaves the tool running and
   says so in its own words. A BOUNDED 10 s wait for `pane_back_at_shell`
   follows — a shell in the foreground with nothing under it, every gap false
   so it times out rather than call a pane it could not read idle. It must hold
   UNBROKEN for `STOP_SETTLE` (1 s, `settled`), because a fresh shell reads idle
   before its startup hooks start their children and the dead proof would then
   refuse busy; a child later than that window still races (named residual). On
   timeout the refusal names what holds the pane with the meta untouched.
   Last, the stop's own event, written once the pane is PROVEN back at its
   shell and never before, naming the binary ended because the meta keeps only
   the current one.
9. `prove_dead`, which refuses a pane running a KNOWN harness other than the
   recorded one BY NAME (`procs::observed_harness`, diagnosis only; `ae doctor`
   names the same seat) rather than as busy;
9a. the CARRY question (`src/carry.rs`) — see below. Asked here, past the dead
    proof and before anything is removed, so every refusal costs only the
    reading. A move that does not carry leaves steps 10-17 exactly as they
    were. A copy failure here is only what appeared after 7a — the target
    account is outside the lock — or a write the Check never tries, and it
    keeps the loud seeded fallback. The KEEP question (below) is asked at the
    same point, and only for a move that stays in its account;
10. the seed pack is BUILT — before anything is removed, because it reads the
    seat's recorded first message and step 11 deletes that file;
11. the seed is published as `seed.<agent>.md` (0600), then `run::clear_slot`
    removes the start marker, the prompt file, the launch script and the
    per-tool id files, so the successor's `_run` has no stale first message
    and no resume decision to make. A CARRIED seat publishes no seed and then
    puts the start marker BACK: `_run` reads exactly that file to choose
    between creating a conversation and resuming one, so without it the store
    would be copied and the successor would open a new conversation beside it.
    A KEPT seat does the same, its pack held in memory for the post-start proof;
12. ONE guarded meta replacement (`meta::publish_seat_move`);
13. `read_seat` again — the command, the tool and the conversation are the new
    profile's now — then `start`.

The lock is DROPPED before any readiness wait, because a gated turn blocks up
to 45 s and no session may be held out of its own lifecycle for that. Past it:

14. the tool's OWN launch turn where its adapter has one (codex's rollout does
    not exist until a user turn, and that registration handshake must not sit
    under a 24 KB seed);
15. the seed, as `⟦ae:ctx⟧` — `provenance` puts the marker on, because
    `deliver_launch_turn` pastes its text verbatim. A carried seat is handed
    none: it resumed the conversation itself. A kept seat is handed none
    either, unless the post-start proof finds its resume fell back;
16. the post-launch capture for a slot that still reads `pending`;
17. a LAST identity reading, because exit 0 means the seat is up NOW.

## The meta move

`meta::reseated` is pure; `meta::seat_move_for` is the guard; only
`publish_seat_move` touches a file. The guard compares `seat.<slot>` to the
agent the caller proved: a reseat holds the lifecycle lock so no second reseat
interleaves, but a `retire` plus a re-`spawn` can land a DIFFERENT seat on the
slot, and that successor must not inherit the move.

`meta::Conversation` says what becomes of the conversation, as ONE value: the
arms differ in three rows at once, and a caller able to spell half of each
would publish a seat whose records contradict themselves.

`Fresh` WRITES `harness_session` (a fresh UUID where the tool takes one at
launch, `pending` otherwise) and `capture_floor`, and appends the old
conversation to `harness_session_prior` TAGGED with the tool that owns it — the
whole point of the tag, since the successor's tool reads a different store. It
REMOVES `config_home` and `config_home_base`.

`Carried` touches NONE of those four: the seat still holds the conversation the
row names, a retained exact conversation keeps the floor it was born under, and
nothing was abandoned, so there is no predecessor. It REWRITES `config_home`
and `config_home_base` to the target account instead, in that same replacement —
a published conversation whose store no row names is the window that would let a
later reader resolve it in the home the seat just left.

`Kept` touches none of the four and leaves the account rows as they are: the
same tool in the same account resumes the same conversation.

All three arms WRITE `profile`, `agent_bin` and `launch_id`, and REMOVE `launch_time`
(it would date a launch that has not happened), `observed_model` and its pin
(they would read as drift the moment the new tool answers, and a second account
of one tool may well serve a different one), and `client` (a launch's
`profile@client` override, for a profile this seat no longer runs).
The client row is removed rather than emptied: an empty override is not the
same fact as no override.

A PAIR move (`--using profile@client`, `meta::SeatMove::pair`) instead writes
ONE `client.<slot>` row after that removal, so any earlier rows collapse to it,
and the replacement names the account the client selects in every arm: `Fresh`
WRITES the pair's `config_home` and implicit base (a bare `Fresh` removes them
and leaves them to `_run`'s first start); `Carried` writes the account it was
copied into, built from the same resolved account (`reseat::Moving::account`)
as the pair's identity; `Kept` leaves the rows `reseat::keeps` proved, under
the lock, are that account. The pair's first start runs the gate-judged command
as the `_run` snapshot, so an overlay cannot swap either name. The reseat
record's `from` and `to` spell `profile@client` wherever a client applies
(human text only; `profile.<slot>` stays bare).

All three arms also REMOVE every `effort.<slot>` line, bare, damaged and CR-ended
ones included: a quota-driven effort record belongs to the setup that is
leaving, and the `reseat` record resets the seat's effort fold in the journal.
A resume decides the same record in `effort::on_resume`, first rule wins:

| Seat on resume | Effort record |
|---|---|
| no record | nothing |
| not the retained conversation (create, fallback, a new compacted id) | dropped, `fresh`; a `restart-down` record is journaled `effort-failed` instead, its authorized step down spent |
| `auto_effort = off` | dropped, `off` |
| switch unusable, record damaged, another seat or setup, journal unproven, yielded | kept, profile as written |
| a `restart-down`/`restart-up` move, guard ok | replayed to its target when the journal reserves exactly that move for its episode and threshold, unyielded; otherwise refused |
| a move still pending, switch on | refused before the start marker |
| applied, guard and journal agree | one effort value rewritten to the recorded level |

A drop journals `effort-discarded` as the watchdog FIRST, then publishes the
meta without the lines, under one hold of `meta.lock`, and only while the
seat, its launch and the raw lines are still the ones `_run` read. A fallback
drops them in its own replacement. Each discard says one stderr line: a seat
that moved or a record that could not be journaled keeps the lines; a journaled
discard whose meta write failed keeps them too, and the next off-resume
discards again; one whose directory sync failed has them visibly gone, durability
unproven. A kept record says nothing. Either way the launch goes ahead at the
profile's effort.

The live restart under `auto_effort` is settled by the next leg after a
crash, never by a second stop: a reservation with no record or no respawn is
`effort-failed`, and a restarted tool with no outcome is proven again by
`Ctx::prove`. One window is a NAMED residual (plan W7): an `up` move whose
meta clear landed while its journal did not leaves nothing to prove against —
the record carried the identity and the old pid. The next leg journals
`effort-failed` with `outcome unproven: record cleared, identity and pid receipt
lost, profile effort expected` and tells the seat the same, never `restart
failed` and never `restored`, because neither is known. The episode makes no
further attempt.

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

## Carrying the conversation between two accounts (`src/carry.rs`)

`src/carry.rs` owns this and states its own rules; what belongs here is the
evidence it rests on, the ladder, and what is deliberately left open.

MEASURED 2026-09-19, claude 2.1.278: a transcript copied into another config
home's `projects/<key>/` is found by `--resume <uuid>`, the id is global to the
store, and the resume APPENDS. That is the whole basis, and it is claude's
alone — `ToolAdapter::carry` is where the per-tool fact lives and every other
row is `NotPortable`. codex and muse look portable on paper and are unmeasured;
agy, gemini and opencode keep stores ae has not characterized.

The ladder, all of it under the lifecycle lock the verb already holds:

1. `carry::plan`, pure, and its `None` is SILENT — an ordinary tool change, a
   move inside one account, an id that is not a grammar-proven UUID, or a
   non-portable store all behave exactly as they did before this existed. It
   carries only when the recorded `agent_bin` EQUALS the binary the new profile
   lexes to, the adapter says portable, `capture::is_lowercase_uuid` passes, and
   both accounts resolve to usable paths that DIFFER;
2. the conversation's bytes are read, and the target project directory
   classified, before anything is written;
2a. the copy walks the whole set once more WITHOUT writing and requires the
   very decisions the Check fingerprinted (below), then moves each frozen
   sidecar aside;
3. sidecars, then project memory, then — LAST — the transcript, which is the
   COMMIT MARKER: a crash before it never leaves the NEW conversation half
   there. A transcript the target lacked stays absent, and one archived right
   before its publication leaves its name empty, so nothing is findable; an
   older round-trip copy stays under its name with its OLD bytes, beside
   sidecars that may already be archived or refreshed, until its own archive;
   an identical one stays as it was. That is convergence, not a transaction:
   the next attempt re-copies, no-ops over its own identical files, archives
   what is still stale and commits;
4. back in `reseat`: no seed pack, the start marker PUT BACK — proven writable
   BEFORE step 2, because `clear_slot` removing it and the carry rewriting it
   cannot be one write and the state between them is the one that would open a
   new conversation beside the copied store — and one guarded meta replacement
   that keeps the conversation and rewrites the store rows.

Two decisions worth the ink. The target account is resolved with a CONTROLLED
lookup — `HOME` and nothing else — because `reseat` runs in the caller's process
and the caller's environment is not the pane's; the source comes from the seat's
RECORDED row, because for a retained conversation the record is what names the
store. And paths are COMPUTED from `carry::project_key`, never searched for,
which is what makes the result resumable: `project_key` implements claude's own
rule, every character that is not an ASCII letter or digit becomes `-` (claude
2.1.299: `path.replace(/[^a-zA-Z0-9]/g, "-")`, per UTF-16 unit), and
`run::resumable` calls it, so the copy and the probe cannot disagree about the
rule. They can still disagree about the INPUT: the carry keys the RECORDED work
dir and the probe the pane's physical cwd (`getcwd`), so a working copy reached
through a link can key differently in the two. The carry then finds no source
transcript at its key and refuses, though the probe may still resume that seat
in its OLD account.

ROUND TRIPS (human ruling 2026-10-14). A move back to an account the
conversation already visited meets that account's OLDER copy: measured on the
lead's own move, the target transcript was an exact byte prefix of the source
(171 MB of 674 MB) and one file-history sidecar held other bytes, and both
refused. Such files are now ARCHIVED, not refused: a transcript that is a STRICT
byte prefix of the source (shorter, `source[..len] == target`; empty included)
and a sidecar regular file holding other bytes are renamed, inside the TARGET
account, into a fresh directory of this move —
`<account>/ae-carry-archive/<conversation>/<YYYYMMDDTHHMMSSZ>-<pid>.<n>/` (the
UTC second, no `-` or `:`, so the leaf as a bare name cannot look like
`host:path`), `0700`, created exclusively (a taken name is never reused; the
next `<n>` is tried), each file under its own account-relative path — so bytes,
inode and mtime are kept and nothing is deleted. Equal files stay untouched; target-only
files are never visited; a forked or longer transcript, a link, a directory
where a file belongs and an unreadable node still refuse as before. The move
prints ONE line naming the count and the directory, and the move record carries
both (`conversation <id>, carried, archived N target files in <dir>`); a move
that archived nothing prints and records exactly what it did before.

THE CHECK DECIDES, THE COPY RE-PROVES. Each archive decision is FINGERPRINTED
at the pre-stop Check — device, inode, length, mtime and a SHA-256 digest
(`install::sha256_hex`), fixed size per file, so the proof does not hold every
conflicting sidecar's bytes across the stop — and the Check also proves, from
mode bits alone, that the deepest existing directory on the archive path and
each archived file's directory are real directories their owner may write in
and search, on the archived file's device. That is a PROXY: an ACL, a read-only
mount, a file flag or a directory another user owns can still refuse the copy,
which is then the loud seed fallback. The copy archives EXACTLY what the Check
fingerprinted, or nothing: before its first write it walks the set again and
abandons on any drift — a frozen file gone, changed or now equal, or a
difference the Check never saw — and it re-proves each file against its
fingerprint, its identity taken again after the read, only once that file's
destination directory is prepared, so no step of ae's stands between that
proof, the destination proven absent by the same no-link walk, and the
rename. The stale transcript goes aside LAST, right before the
conversation is published under its name, so the commit marker still comes
last. A crash between a rename and its publication leaves the name free and the
next attempt converges; a failure after an archive names the count and the
directory in the fallback's reason; an earlier archive directory is never
touched again.

NAMED RESIDUALS of archiving, left open on purpose: a writer of the same OS user
planting a file at an archive destination between the absent check and the
rename would be replaced by it (`RENAME_NOREPLACE` needs libc); a change after
the re-proof, before its rename, is not seen (no `openat` without libc); a
change the re-proof finds leaves the file where it was and a partial or empty
archive, which the reason names; and
a tool running in the target account that holds the stale transcript open
keeps writing the ARCHIVED inode — ae adds no `lsof` door to look.

THE BAR, stated because a guard that oversells itself is the defect it exists
to prevent: both homes belong to the SAME OS user, so this is no silent
clobber, no followed link and no silent skip — not a guarantee against someone
who can already write in the account. A target name ae publishes is claimed by
the one exclusive creation ae already owns, `init::create_exclusive` — a temp under a
nonce that call opened, then a `hard_link` onto the final name — and never by a
rename, which would replace whatever appeared after the check. Every component
from the account root down is classified before it is used; and an `lstat` that
fails for any reason but absence is a failure, never an absence. What is NOT
closed is the window between a classification and the read or write that
follows it: that needs `openat`/`O_NOFOLLOW`, which has no safe-Rust spelling
without a libc dependency this crate forbids.

RESIDUAL: claude keeps a key of 200 characters or fewer as is and, for a longer
one, cuts it at 200 and appends `-` plus a hash of the whole path (read in
2.1.299's bundle, never measured against a real long-path folder).
`project_key` does not implement that: a work dir whose key passes 200
characters is not found, so its seat starts fresh and its carry refuses.
`tests/it/doors.rs` guards that no product file but `src/carry.rs` spells the
rule.

The human's rulings, 2026-09-19: typing the move with the other account's
profile IS the consent, so ae prompts for nothing and prints ONE line naming the
crossing; project `memory/` is copied only into an account that has none and is
NEVER merged; a copy that fails falls back LOUDLY to the seed path and the move
still happens; and there is no model-availability pre-check, because the
existing post-launch identity reading is the check. Narrowed 2026-09-23 (#155):
a failure the pre-stop Check (step 7a) can see refuses the move instead, so the
fallback keeps only a failed write and a change the Check could not see.

THE RECORD SPELLS THE CONVERSATION BY WHAT HAPPENED TO IT. Every other arm
names what the move left behind — `prior <id>` — but a CARRIED conversation is
not left behind, it is the same one continuing in the other account, so the
carry arm reads `conversation <id>, carried`. `Carried::field` is the one place
that decides, beside `::word`, because a record that called a live conversation
prior would send a later reader after it believing it dead. A seeded fallback
really did abandon one, so it keeps `prior`, and a move that was never a carry
question writes what a tool change always wrote, byte for byte.

## Keeping the conversation on another model or effort (`reseat::keeps`)

A move inside ONE account of ONE tool that changes only the model or the effort
resumes the SAME conversation on the new flags instead of seeding a fresh one.
`keeps` is pure and asked under the lock from the records: the same recorded
`agent_bin`, the same recorded account (path, mode and implicit base), an id
`launch::id_probeable` accepts, and the recorded profile's command (resolved as
`_run` resolves it, client override included) equal to the new one under
`launch_cmd::keep_skeleton`. That skeleton drops the ONE model flag's value and,
where the row allows, the ONE effort setting whole — absent on either side
compares equal; a duplicate or unreadable one proves nothing.

The rows are `tool::KeepSpec`, MEASURED 2026-10-21 by a codeword recalled across
the change: claude 2.1.306 `--resume` with `--model`/`--effort`, codex 0.163.0
`resume` with `-m` and `-c model_reasoning_effort=`, opencode 1.19.3 `-s` with
`-m` (model only: `--variant` is a `run` flag). Every other row is unmeasured
and seeds as before. BOTH commands must name a model: a model-flagless profile
never keeps, because nothing then says what the resume would run on.

Past the dead proof, `run::resumable_in` — the one probe `_run` resumes with —
asks the store keyed by the seat's RECORDED working copy; a miss seeds and
stdout says `could not keep conversation <id> (…)`. A kept move writes no
predecessor, keeps the id, floor and account rows (`meta::Conversation::Kept`),
puts the start marker back as a carry does, and publishes no seed: its pack is
built before `clear_slot` and held. KEPT IS PROVEN ONLY AFTER THE START, by the
id `_run` left: an unchanged id prints `kept conversation <id> from profile …
(model … -> …, effort … -> …)` and the record reads `conversation <id>, kept`;
a fresh id means the resume fell back, so the pack is published and pasted, the
record reads `prior <id>, seeded (resume fell back)` and the exit stays 0. A
tool never seen claims nothing: a fresh id there is the same fallback, an
unchanged one leaves the conversation out of the record.

Named residuals: `_run` probes the pane's PHYSICAL cwd while the reseat keys the
recorded one, so a working copy reached through a link can disagree — the
post-start proof turns that into a loud seed, never a false `kept`; and
opencode's probe is `RecordedId`, which checks no store, so a vanished opencode
session still reads as kept.

THE STOP RECORD NAMES NO CONVERSATION AT ALL (`Mentions`). Which binary was ended,
in which pane, and which move it belongs to is the whole of that event: a
conversation id is not a fact about stopping a tool. It matters most where the
move is about to carry, because the stop is durable at the moment the pane is
proven back at its shell — BEFORE the verdict exists — so any word it chose
would be a guess, and `prior` would be the wrong one. Deferring the record until
the verdict is not open: a stop that finished must be on the record whatever
happens next.

## Automatic reseat

The watchdog moves a seat PROVEN stuck on its vendor usage limit the same way,
through the same verb: the only difference is who types it. The decisions live
in `src/autoreseat.rs`, the legs in `src/autoreseat/leg.rs`, the daemon step in
`src/watchdog_daemon.rs::auto_reseat`. The config is `[workspace] auto_reseat`
(`off` by default), `auto_reseat_sessions`, `auto_reseat_grace_secs`,
`auto_reseat_at`, and the `[auto_reseat]` map; see
[Configuration](../getting-started/config.md).

One episode is the journal suffix starting at the first `limit` the watchdog
booked for a seat after its newest `alert-cleared` or `spawn`; its KEY is that
record's timestamp. A `reseat` record is no boundary, because the move itself
writes one before its outcome. Every record of the path carries the key as its
`ref`, except `auto-reseat-done`, whose `ref` is the profile the seat LEFT —
the next episode's chooser reads that row. A restarted daemon re-books `limit`
for the same key: the fold ignores a second `limit` while the episode stands,
so the key survives the restart.

**A near reset waits.** A limit episode with no attempt open is left to the watchdog's limit
wake ([Watchdog](watchdog.md)) instead of moving while its carried binding reset is SHORT (every
binding window states a length under a day) and under `LIMIT_WAIT_MAX_SECS` (60 min) away,
judged each cycle at decision time, until `LIMIT_WAKE_FALLBACK_SECS` (300 s) past it
(`autoreseat::reset_wait`); the first-sight forecast then says ae waits and wakes the seat
instead of promising a move. Past the fallback the move applies as before, unless the seat
resumed on that reset — its wake LANDED in the journal, or Claude Code's own resume was armed
for it — which holds the move until a fresh reading names a new binding reset. The wait is
silent: no trigger, no hold record, no unmapped naming. An unknown reset, a long or unstated
window, a reset 60 min or more away, an attempt already open and headroom episodes move or
close exactly as before.

Past the grace the daemon hands at most one due seat per cycle to the trigger,
through the session's own `send` helper with the action `auto-reseat` — the
trigger re-derives the whole decision under the seat's lock, so a forged call
does exactly what the daemon would do now. The trigger journals `auto-reseat`
BEFORE it spawns the detached `_auto-reseat <dir> <slot> <key>` leg, and
releases the seat's lock between the two, so the leg can take it itself; a leg
that could not start closes the attempt `failed` at once, under the lock taken
again and only while the journal still shows that attempt open. The leg takes
the seat's lock without waiting before it reads anything — held by another
writer, it writes nothing. It recomputes the key: a seat gone, or an episode
that is not the key's, REFUSES; an attempt already closed, or none open, is not
its to act on and journals nothing. A limit and a headroom episode opened in the
same second share a key, and the leg takes the one whose attempt is in flight. It
re-chooses from the global config, then moves through `reseat::run_as_watchdog`
with `StopPolicy::Proven` — a running tool is stopped only on a frame proven
idle, or proven on the vendor's usage limit over an empty box, on both
readings — and journals ONE outcome. The first closing of a round wins; a later
one closes nothing new. A round gets at most two attempts: one, and one more
after a transient hold; an attempt still open past 180 s is booked `failed` by
the daemon.

A LIMIT round closed `auto-reseat-refused` or `-failed` is not the end of its
episode. Thirty minutes after that closing (`RETRY_COOLDOWN_SECS`), while the seat
is still latched on the same limit, it is due again under the same key, through
the same grace, frame and hold rules, one seat a cycle: the trigger journals
`auto-reseat` (`ref` the key) as ever, which opens a new round, and a refusal the
trigger writes with no attempt before it (nothing usable) counts as a closing
too. Three closings in all (`MAX_ROUNDS`, so at most six attempts per key) end
the path: the daemon journals `auto-reseat-held` (`ref` the key) with
`other: held: gave up after repeated refused or failed moves` once, tells the lead pair
and the spawner as it tells every ending, and a restarted daemon reads the hold
back and says nothing more. An `auto-reseat-done` is final whenever it lands, and
an `alert-cleared` or a `spawn` still starts a new episode with a fresh count.
Nothing here is remembered outside the journal: the fold is the same after a
restart. A headroom episode is not retried.

The candidates are the `[auto_reseat]` row, then the seat's own family —
`autoreseat::listed`, which `leg::listed` feeds from the session's identity
config, the seat's recorded profile through its recorded client, and the store
its meta records. ONE comparer decides the family: `launch_cmd::family_skeleton`
reduces a command to every word but its model flag's value, under `Drop` without
the tool's config-home assignment — the follow keeps it, under `Keep`, so a
followed model never crosses an account. A store is `reseat::account_of`'s
canonical path: equal paths are one account, and an unknown one equals nothing.
A tool with no account variable has one shared store, where only a
byte-identical command is the same profile: a respelled flag is not. A seat with
no row and no family is unmapped: a leg that finds it so holds with `held: no
declared or derived candidate for this profile`, and the daemon, which opens no
attempt for it, journals that same hold ONCE per limit episode past the grace
(`AutoAct::Unmapped`, deduped by the journal, so a restart repeats nothing) and
tells the lead pair and the spawner to add an `[auto_reseat]` row or move it by
hand. A hold is not terminal: a row added later moves the seat next cycle. The daemon asks for a family only for a seat eligible but for its row,
and only while it could act — on its limit, at or near its threshold, or inside
an episode — reading the identity config once for each such seat per cycle.

COVER is the question asked BEFORE a limit: does this profile have a candidate
that leaves its own account? `autoreseat::cover` answers it over the very list
`listed` builds, so no second notion of family or account exists. A candidate is
off the account when its tool differs, or when the same tool's proven store
differs (`Shape::off_account`); a sibling or twin on the same store is a
candidate but no exit, which is how a profile whose only candidates share one
quota bucket — the Muse case, every model one shared allowance — reads as
uncovered. A tool ae cannot name, or a store it cannot prove, is never proof:
the verdict is `Unproven`, and says whether the profile's OWN account or a
CANDIDATE's is the one in doubt. `leg::covers` judges every `[profiles]` key
(a key whose command does not lex is `Unproven`, scope `?`), and only while the
switch is on; with `auto_reseat = off` there is no verdict and no nudge. The
verdict is read in three places and nowhere is it a gate: `ae doctor`'s
`reseat-cover` row, one WARN row per account scope; and ONE stderr line from
`ae spawn` and `ae reseat --using` (`leg::cover_warning`) naming the target
profile, then proceeding with the same exit code. The watchdog's own move
prints none. The suggestion on a WARN row is built from the declared rows first,
then `[profiles]` order, off-account only, at most two with the second preferring
another tool; it never invents a profile. Named residuals: the account is the
config home ae can prove, not the vendor's live quota buckets, so a sibling
whose window is model-scoped still reads as no exit; doctor judges the
invocation's global config and cwd overlay, `spawn` and `reseat` the session's
recorded config, and a session with no recorded `config=` row gets no nudge; a
suggestion may name a profile the operator keeps for rare use.

The chooser takes the first usable candidate of the best tier, in listed order —
and every declared candidate outranks every derived one: a declared candidate on
a worse tier still wins over a fresh twin, and an operator who wants the family
first changes the row. Among derived candidates a critical one ranks last; the
rest rank by `autoreseat::Step` — `Kept` (same store, commands equal under
`launch_cmd::keep_commands`, the comparison `reseat` decides a kept move by),
then `Account` (another proven store), then `Seeded` — and only then by tier.
The step is a prediction: the move still proves the conversation kept or seeds
loudly. A plan-routed candidate (`tool::PlanRoute::routes`) is never derived. A candidate is judged by its EFFECTIVE percentage, never the
raw window: a reached spend cap judges 100 and reads exhausted, and only a FRESH
window places a tier — below critical first, then unknown, then critical. A
window scoped to another model family binds no one else, while a pinless
candidate stays bound. Another seat of the session still on its limit passes
over the candidate's account — the moving seat's own latch is left out, because
its limit may bind only its model family. A profile the seat left on its limit
waits for a window read after the move that proves relief: a usable reading
below critical, or a reset that has passed. Residual, named not fixed: a capped
account with no usable window reads Unknown.

Every writer of the path's records — the daemon, the trigger and the leg —
takes the seat's lock WITHOUT waiting, and takes it BEFORE the journal is
re-read and appended to, never after. A daemon or trigger write that finds it
held skips, and the next cycle asks again. A leg that finds it held writes
nothing and is not retried: its attempt stays open until the daemon books it
`failed` past 180 s.

Each ending is said once on the human's chat and told to the lead pair minus
the moved seat, plus the seat's spawner when that names another roster seat not
told already — each through the session's own `send` helper, as the watchdog,
which ends no declaration. Refusals and failures the trigger closes itself are
chat-only; the two endings a forged argv can reach journal their record and say
nothing further.

A CARRIED or KEPT move hands the moved seat one turn before it is told about:
its conversation came along, so no seed pack did; the turn names what moved
(another account, or another model or effort), and the records say `kept`
where the conversation stayed. After the `-done` record, the leg
sends one continuation (re-declare, check requests, memo and brief, resume only
authorized work, keep real waits and parked scope) through the same `send`
helper and its guards, journaled as `auto-reseat-notice` to the seat. The
notice's `Next:` then says the nudge was submitted only on the helper's exit 0
and its own unmarked record; `unconfirmed` or `refused or unrecorded` name a
peek before any resend. A notice over its bound loses details, never that step.
Nothing retries the nudge, and a seeded move gets none.

The records are `auto-reseat` (the attempt, quoting from and to),
`auto-reseat-held` (a hold before an attempt, or a transient refusal of one),
`auto-reseat-done` (the seat moved, `ref` the profile left),
`auto-reseat-refused` and `auto-reseat-failed` (no attempt this round; a limit
episode tries again after its cooldown until its third), and `auto-reseat-notice` (one per delivery). A move interrupted
between the stop and the tool starting is finished the same way a manual one
is: `relaunch <agent>` — the failed notice says so by name.

Every hold, refusal and failure summary, and the notice that quotes it, LEADS
with a cause class — `<class>: <message>` — so a clip of a long message (a path,
a profile name) never takes the cause. The notice puts the class AHEAD of the
agent and session names (`auto reseat: <class>: <agent> not moved — …`), and
its cut reserves that head and the whole safe next step (a `--stop-unknown`
command included) before it spends any room on the body. A profile name has no
length bound, so a step that quotes one can fail to fit beside the head: it is
then replaced WHOLE by a fixed step naming only the session and the agent
(`move it by hand (ae reseat <session> <agent> --using <profile>)`), never
clipped to half a command or half a resend condition. A refusal's step is
said ONCE, after `Next:`: the details drop it and keep what stood around it.
A cut of the details lands on a word boundary and ends `…`. A move whose
stop ended background tasks leads with that fact (`auto reseat: ended
<agent>'s background task "<title>"; <agent> moved …`), ahead of every
unbounded name, so no cut takes it; the done record is unchanged. The class is named
where the refusal is decided, never read back out of the printed text: `carry-source-missing` (the
conversation's transcript is not in the account it moves from),
`carry-target-differs` (the account it moves to holds a different transcript, a
node of the wrong kind, or a place ae could not archive into),
`carry-unreadable` (any other carry refusal), `busy` (a running turn, a draft, a
prompt, human input), `unknown-frame` (ae cannot prove the frame idle),
`background-task` (a Muse frame shows only the stop and its background tasks,
and no long limit proves they may end), `lock`
(another writer holds the lifecycle or send lock), `no-candidate`, `ineligible`
(the operator's rules say this seat does not move), `observed-choice` (the
seat's persisted observed model or effort choice stands), `pair-provider` (no
usable candidate keeps the seat's review pair on two proven providers),
`pair-changed` (under the lifecycle lock the chosen destination no longer does)
and `other` (the rest, and the give-up hold of a spent limit episode). `busy`,
`lock`, `observed-choice` and `pair-changed` are transient classes: their record
is a hold, every other class a refusal. `ae reseat` itself prints its messages unchanged.

### The pair guard (#305)

A seat with a review-pair partner is never moved onto the partner's provider by
auto reseat. The partner comes from the roster alone (`autoreseat::partner`): in
a `layout = lead-pair` session `main` and `worker.0` are each other's, before any
name; otherwise two SPAWNED seats named `<name>` and `<name>-nav`. Those name
links are walked over every spawned seat, and a seat whose linked group holds
more than two seats (a chain `x`, `x-nav`, `x-nav-nav`, or a linked name written
twice) is AMBIGUOUS: paired with a partner nothing proves, never unpaired. A seat
no name links is solo, whatever else the roster holds, and a solo seat's choices,
records and notices are exactly what they were. Nothing is persisted: the link is
re-read from the roster every time.

A provider is proven from a resolved command alone (`tool::served_provider`):
Claude Code is Anthropic, Codex OpenAI, Muse Meta, Grok Build xAI, Gemini CLI
Google — unless the command carries an `env` prefix, a leading assignment other
than the tool's own account variable, codex `--oss`, `--profile`, `-p` or a
`model_provider` config pair, or muse `--provider` naming another lab.
Antigravity and OpenCode prove none (their in-pane model switch reaches other
labs), nor does an unknown tool. A SEAT serves what its CURRENT roster row
resolves to — its recorded profile through its recorded client, as its own
launch reads it — and only while that command's tool is the tool the row records
running (`agent_bin`); a disagreement, or anything ae cannot read, proves
nothing. A CANDIDATE serves what its profile resolves to, as a move to it
resolves it (`leg::pair_of`).

Every chooser — the first-sight forecast, the trigger and the leg — runs
`autoreseat::choose_paired`: after every other rule, a candidate is usable only
when the partner's provider and the candidate's are both proven and differ
(`PairGuard::judge`); the rest are passed over as `partner's provider` or
`provider unproven`, and the order among the usable ones is the order it always
was. There is no exception on any route: headroom, a drawn limit and an urgent
start all HOLD rather than collapse a pair. A pair ALREADY collapsed — the seat
and its partner both proven on one provider (`PairGuard::collapsed`) — has no
diversity to protect: it still prefers a candidate that makes it diverse, and
with none moves to the best candidate PROVEN on that same provider
(`PairGuard::stays`), its `auto-reseat-done` summary and its one notice led by
`pair-provider: pair stays on <Provider>` and no collapse line after it; an
unproven candidate stays passed over, so with only those it holds, and an
unproven seat or partner gets no such fallback. With nothing usable, and at least one
candidate passed over by the guard alone, the leg or trigger journals
`auto-reseat-refused` with the class `pair-provider`, naming what the seat
serves and what its partner serves (or that the partner's provider is unproven,
or the link ambiguous), and tells it as every refusal is told; a refusal for
quota, history or configuration alone keeps `no-candidate`, byte for byte. A
headroom refusal stays closed until the re-arm, a limit one cools and retries
as ever, and a later round refusing for the very same reason under the same key
is journaled but not told again. Manual `ae reseat` is not guarded and prints
what it always printed.

A seat whose partner has an attempt in flight declines at the trigger, writing
nothing. The binding check runs under the session's lifecycle lock — the one
lock every move of either half takes, held through the meta move — in
`reseat::prepare`'s automatic gate, before any carry check or stop
(`leg::pair_changed`): the partner's row read under that lock, against the
destination's provider. A pair the chosen destination no longer keeps on two
proven providers, and that it does not stay on as above (`stays_serving`, the
same proof as the chooser's), holds as `auto-reseat-held` `pair-changed`
(transient), with nothing stopped. After an automatic move the leg reads the
pair once more, BEFORE the move's record, and judges what it says on that read
(`autoreseat::after_move`): the `pair stays on <Provider>` lead only when the
move was the already-collapsed fallback and the pair still serves only that
provider; otherwise a pair no longer on two proven providers is told one more
line, `auto reseat: pair-provider collapse: …`, beside the move's own notice,
and a pair made diverse meanwhile is told neither. Two test
seams, `@auto-reseat-pair` (after the choice, before the lifecycle lock) and
`@auto-reseat-after` (after the move returns, before that read), ride the
existing read gate (`src/lib.rs::read_gate`).

Named residuals, a command-based approximation: a profile or client edited in
config after its seat launched, with no change of tool, reads as the edited
command; a reroute inside a tool's own config file (codex `config.toml`, Claude
Code `settings.json`) is invisible; and a manual partner move after an automatic
move's lifecycle lock drops is told only when it lands before the leg's
post-move read, and otherwise neither blocked nor told.

### Headroom episodes

A seat can be moved BEFORE its limit. With `[workspace] auto_reseat_at` (global,
`95` by default, `off` disables, `50`-`100`; anything else is `95` with one note
the daemon journals target-less as `auto-reseat-notice` once per start), the
daemon judges every eligible seat whose limit latch does NOT stand against its
OWN client scope — the one `quota::recorded_identity` join the throttle line and
the checkpoint ask make — over its held quota observation: the worst usable
(fresh or stale) window proven at least one day long, rows scoped to another
model family left out, by the one derived percentage. A codex scope is its whole
config home, and each rostered conversation's rollout keeps its own last
reading, so per window (bucket, qualifier and length) only the newest usable
reading of the scope counts: one a newer usable reading of the same window
supersedes — stale or fresh — decides nothing, a tie keeps the worse, and a
lone stale reading still binds (`autoreseat::current`, applied in
`leg::own_scope`; auto effort and the app's lead/colead weighing read the same
scope). At or past the threshold it books
`auto-reseat-headroom`, whose `ts` is the episode KEY and whose summary,
`headroom <pct>% <window>`, every later record of the episode quotes whole. The
episode stays due under that key while the scope reads within 5 points of the
threshold, and is re-armed by `auto-reseat-headroom-cleared` (`ref` the key)
only once it reads strictly below threshold minus 5 with no attempt open; a
terminal outcome therefore stays until then — one refusal per episode, not per
cycle — while a move ends it, since the seat's account changed. No usable reading
decides nothing either way.

The rest is the limit path's: the same grace from the key, the same holds — plus
`held: the seat's frame is not proven idle`, because a headroom move never ends a
turn and a frame the classifier cannot prove idle (muse, a fresh claude) is not
one — the same two attempts, the same trigger, leg, records and endings, each
summary naming the trigger. A hold the journal already ends on is not named
again, a daemon restart included. The trigger re-reads the scope and declines
once it has relief, and the detached leg reads it once more under the seat lock
before it moves: a headroom episode that no longer reads at or within 5 points
of the threshold, or reads nothing usable, is closed `held: the account no
longer reads at the headroom threshold` with the seat untouched; a limit or an
urgent start's episode is not re-judged there. The chooser judges each candidate
account by its own newest usable reading per window, never by another
account's, and adds two skips: `no headroom` for a candidate
with a usable window at or past the threshold that is proven at least one day
long or critical, and `left on headroom` for a
profile the seat left on headroom until a reading after the move proves it
below the threshold, or its reset passed. A successful headroom attempt names
each candidate it passed over. A drawn limit outranks the whole path: a latched
seat is the limit path's alone, which moves exactly as before, and a limit move
still takes a candidate past the threshold. The move's shell is no death and
books no limit.

The two kinds are ONE path for the seat. An attempt in flight under either
kind decides the seat until it has an outcome, so a limit drawn meanwhile books
no second attempt and an unlatched seat opens nothing; a limit supersedes only
a headroom episode no attempt is moving. An `auto-reseat-refused` or
`-failed` under the live key of either kind closes EVERY episode still open on
the seat, and a move closes both whatever its `ref`; an outcome under a key
neither holds closes nothing. A limit episode closed `refused` or `failed`
waits out its cooldown (and is then due again, up to its third closing), a
closed headroom one for its re-arm; a `done`, or a spent limit episode, waits
for its `alert-cleared` and a fresh `limit`. The
kind a profile was left on is the kind of the attempt that moved it, judged when
that attempt was journaled, so an opener booked in the same second afterwards
cannot relabel the move.

### Moving back (#305)

An automatic move is a detour. Once the account the seat left proves room
again, the watchdog moves an idle seat back to where it started, through the
same trigger, leg, records and endings, as a third kind of episode: RETURN.

**The chain.** `autoreseat::chain` folds the seat's journal in append order,
from its newest `spawn`. A watchdog `reseat` that moved the seat
(`reseat::moved_seat`), closed by a LATER watchdog `auto-reseat-done` of a
limit or headroom attempt, is one hop; the first hop's `done` names the ORIGIN
(its `ref`, the profile left) and its `ts` is `away_at`, and later hops keep
both. Any `reseat` journaled by anyone but the watchdog — a move, a stop, or one
that only says the seat already uses that profile — ends the chain, and so does
a `spawn` or a return's own `done`. A `done` with no moved `reseat` before it
proves nothing. `ae reseat` journals its record with the target's slot and no
session, so `autoreseat::addressed` takes a `reseat` record by its exact name
and slot with no session key at all; every other record is addressed by the
routing rule. Only the root counts: a seat moved A → B → C goes back to A, never
to B, and an intermediate profile is never a destination. The origin is resolved
through TODAY's config: a profile gone from it leaves the seat where it is, and a
profile whose command was edited since the move binds the seat to the account
and model that command names now. The chain is read from the journal alone, so a
seat moved automatically before this release, with its history intact, can
return too.

**The proof.** `leg::home` reads the origin's accounts from the one held quota
observation (`leg::origin_samples`): every row of every group naming the origin
profile, each rollout, rows scoped to another model family than the origin's pin
left out, and rows ae cannot judge KEPT. `autoreseat::return_ready` then needs,
for every window of those accounts present NOW, its NEWEST reading to be judged,
fresh, observed strictly after `away_at`, and strictly below the band; a tie at
the newest stamp must agree. The band is `auto_reseat_at` − 5 (95 − 5 when off).
At least one window proven a day long must be among them, and the worst such one
is the cause: `return to <origin>: <pct>% <window>`. Stale, unknown, reset-passed,
unjudged, unstamped, or tied-opposing readings deny it; a passed reset alone
proves nothing, while a fresh healthy reading after `away_at` proves a manual
reset. A usable spend cap on an origin account, or another seat of the session
latched on one (`leg::origin_blocked`), denies it too.

**The episode.** With the proof and no return episode open, the daemon books
`auto-reseat-return` (its `ts` the key, its summary the cause). A drawn limit
outranks it and so does any headroom episode: a seat its own account presses is
the headroom path's. An unmapped seat — no `[auto_reseat]` row, no family — may
still go home, since the origin is the return's one candidate; only the return
path decides it. The rest is the limit path's: the plain grace and the plain
frame (a resume's waivers never apply, and a return never takes
`StopPolicy::QuietStart`), the same rounds and cooldowns, the pair guard, and a
`done` whose `ref` is the profile left (the substitute) and whose summary names
the return. A return's `done` never makes the substitute a profile "left on"
pressure. The daemon holds a return SILENTLY: a busy, drafted or unread seat
books no hold, attempt or notice record. The trigger and the leg re-derive the proof: gone, the leg closes the
attempt `held: the origin account no longer proves room`
(`held: the seat is no longer on its automatic chain`, `held: the return is not due yet`).
Under the lifecycle lock `reseat::prepare` re-checks that the chain's newest move
and the locked row's profile are still what the return was chosen from
(`leg::return_changed`); anything else holds as `return-changed` (transient)
with nothing stopped.

**The flap limit.** A seat goes home at most `MAX_RETURNS` (2) times in a rolling
24 h, counted from return `done`s in the journal, so a restart forgets nothing.
Past it the daemon journals `auto-reseat-held` once per chain, `other: held:
returned 2 times in 24 h; move it back by hand (ae reseat <session> <agent>
--using <origin>)`, and tells it as every ending.

Nothing new is configured: `auto_reseat`, `auto_reseat_sessions` and the
switch's scope gate a return as they gate every move, and with quota off no
reading proves anything, so nothing returns. Named residuals: an origin whose
account no seat writes stays dark (a fresh reading needs a writer on it, so a
seat may wait on its substitute); a short window that vanishes from the
observation is no longer judged, since only the windows present now are; an
origin account that fills again right after a return moves the seat away again
through the ordinary paths, bounded by the flap limit; an origin whose tool ae
reads no quota for never returns; a busy seat waits for an idle frame
however long that takes; and an origin that ran on a `profile@client` pair
returns to its BARE profile, the profile's own account — the chain records the
origin's profile only, and the return's readiness is judged on that same
default account, so it is not unsafe, just not the original login (follow-up
with automatic `@client` destinations, which no automatic move chooses today).

### A resume on an exhausted seat

A stopped session started onto a dead account should not spend a failed turn, or
a grace, finding that out. So a resuming launch writes `launch_settled=<epoch>`
once, after its last launch turn ended, whatever each outcome was. The rebuild
of every launch drops the row, and a new session or a crash before it leaves
none: without it nothing changes. `autoreseat::start_fact` takes it only when it
is no older than `started` and not in the future. A slow serial delivery delays
every seat behind it; the window starts when delivery ends, not when it began.

For `START_WINDOW_SECS` (360) after that, `leg::start_read` is the ONE read every
actuator asks: the daemon's step, the trigger, the leg, the effort step and leg,
and the quota notices. It needs the seat in auto reseat's reach, then a FRESH
window of its own account binding its model at 100 judged percent, of any length
and whatever `auto_reseat_at` says, or a fresh spend cap. The newest reading of a
window rules, a tie between opposed readings proves nothing, and another window
never answers for this one. Stale, unknown, uncorrelated or absent readings prove
nothing.

Binding is stricter here than for an ask or a headroom reading: an account-wide
window binds, and a model-scoped one binds only when `quota::scoped_here` proves
the window's family and the pin's are the same recognized family. So a pinless
seat, or an unknown qualifier, does not move on a resume however full that
window reads; it takes the ordinary paths.

An urgent seat opens the ordinary `auto-reseat-headroom` episode, and the same
trigger, leg and records carry it, so resume semantics, rounds and cooldown stand.
Only two waits change: the grace is waived for an episode keyed inside the window,
limit or headroom, and client input holds for `deliver::VIEW_GRACE` instead of the
grace. A busy, drafted, unread or prompting frame holds as ever. So does an
unproven one, with one exception below. Three more seams follow from a move that
may run before a turn could:

- An armed brief of the current incarnation on the live pane blocks urgency: the
  move mints a new launch id, which would end the record undelivered. Once it is
  delivered the seat moves.
- A seat that urgent gets no quota advisory or checkpoint ask, because any turn on
  its account would fail, and effort stands down as it does for a drawn limit.
- No lock spans the trigger and the leg, so the leg reads again. An attempt ahead
  of its plain grace whose proof has gone closes `auto-reseat-held`, and the next
  cycle decides by the plain grace.

A resumed claude never reads idle: claude 2.1.316 draws its last reply over an
empty box and no done row, so the classifier says `Unknown` and a move would hold
for good, on exactly the seat this section is for. `autoreseat::quiet_pair` proves
that frame instead, for an urgent episode alone (`StartRead::frame`): two captures
that match byte for byte, hint rows included, with the tool's own box drawn and
empty, no spinner and no human prompt. The classifier does not change. Three pairs
are taken, each owing nothing to the others: the daemon compares consecutive
cycles, the trigger takes its own `deliver::FRAME_GAP` apart, and the stop takes
its two under the send lock. The stop reaches it only as `StopPolicy::QuietStart`,
which only `run_as_watchdog` sets and no word of `ae reseat` names, and its last
word re-reads `start_read` under its locks. A real claude's hint rows settle within
40 s and then held for the 200 s measured, so a default 60 s pair lands. Moving at
the default interval takes about two minutes: one cycle opens, the next attempts.

A first quota pass may run before the seat feeds its first reading, 2-3 s into a
real claude start, so the daemon takes ONE follow-up pass 5 s after settling,
outside the cadence's counter. It runs only with a seat in auto reseat's reach, so
an off or excluded session reads exactly as before, and never at
`quota_every_secs = 0`.

With `auto_reseat_at = off` the seat's `Room` reads `leg::own_relief`: relief needs
a healthy fresh window, and every exhausted reading answered by a strictly newer
healthy one of its own window, short windows included. A tie, a window still full
or no fresh window says nothing. A later start ends an earlier start's
episode with `-headroom-cleared`, then opens its own, so each start answers for
itself and a refusal is one per start.

What this does not reach, each behaving as before: an account with no statusline
feed, codex before its first turn, a fresh claude's logo frame (`Unknown`, held), a
status line that never stops changing (no pair, held), a watchdog interval over
180 s, a limit episode keyed before settling, an effort
restart before settling, an in-band reading first seen before settling (the watchdog starts ahead of phase
2, so this is the usual first pass, and it books today's ask: that turn fails, the
limit path picks it up, and the seat moves all the same), an episode held past the window (effort no longer stands down
for it), and a brief record on the pane the stop took, which brief retry gives up
on its own. The plain grace path still does not wait for an armed brief.

A seeded successor is handed the seat pack, whose declared-state section reads
`ae reads this as YOUR declaration until you re-declare`: the predecessor's
last declaration stands for the successor until it declares its own. A carried
seat gets no pack — it continues its own conversation.
