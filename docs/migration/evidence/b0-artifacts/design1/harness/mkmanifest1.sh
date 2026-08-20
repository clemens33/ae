#!/opt/homebrew/bin/bash
set -euo pipefail
shopt -s nullglob
SB=/tmp/aeb0
D=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/b0-artifacts/design1
sha() { shasum -a 256 "$1" | awk '{print $1}'; }
M="$D/MANIFEST.md"
{
cat <<'HDR'
# B0 Design 1 — SC-507b archive preview/digest stitch cut: run manifest

Captures only. This file records what was run, what was captured, and where the
bytes are. It contains no verdict, classification, or expected-vs-actual claim.

## Frozen source of truth

| Item | Value |
|---|---|
HDR
echo "| frozen commit | \`72c729343a0117af2968b66e1c43f89ad25fc0b2\` |"
echo "| frozen \`ae\` sha256 | \`$(cat "$SB/frozen/ae.sha256")\` |"
echo "| instrumented \`ae\` sha256 | \`$(cat "$SB/instr/ae.sha256")\` |"
echo "| hook patch | \`harness/h507.patch\` |"
echo "| hook patch sha256 | \`$(cat "$SB/instr/h507.patch.sha256")\` |"
echo "| environment / tool hashes | \`harness/env-record.txt\` |"
echo "| runner scripts | \`harness/run-arm.sh\`, \`harness/manifest.sh\`, \`harness/build-template.sh\`, \`harness/harvest.sh\`, \`harness/harvest2.sh\`, \`harness/envrec.sh\`, \`harness/assemble.sh\` |"
cat <<'HDR2'

## Hook sites (one patch, two sites, both in the patch above)

| Hook | Site (frozen line numbers) | Emission |
|---|---|---|
| `H507_AFTER_FACTS` | inserted after `ae:4956` — after the `_ar_facts_row` read in `_ar_compose_meta`, before `_ar_build_meta` (`ae:4960`) | writes an arrival ordinal + timestamp to the barrier log outside the cloned AE_HOME, then blocks on `release.<n>`; bounded by `AE_H507_MAXPOLL` (600 polls x 0.1s), on expiry writes a `timeout` line and returns |
| `H507_PASS` | inserted after `ae:5029` (outer `digest="$(_ar_preview_once …)"` block) and after `ae:5038` (retry analogue) | appends a pass ordinal + timestamp to `pass.log`; reads no product state |

Both hooks are no-ops when `AE_H507_DIR` is unset. All hook and controller
artifacts live outside the cloned AE_HOME (`arms/<arm>/h507/`), so they are not
part of any product-state manifest.

## Template fixture (producer-derived)

HDR2
echo "| Item | Value |"
echo "|---|---|"
echo "| template AE_HOME manifest | \`template/manifest.tsv\` |"
echo "| template fingerprint (sha256 of that manifest) | \`$(cat "$SB/template/fingerprint.sha256")\` |"
echo "| session name | \`b0tmpl\` |"
echo "| session_id | \`$(grep '^session_id=' "$SB/template/.ae/sessions/b0tmpl/meta" | cut -d= -f2)\` |"
echo "| meta / memo.tsv / events.jsonl / messages | \`template/session/\` |"
echo "| launch config | \`template/config\` |"
cat <<'HDR3'

Construction (all bytes producer-derived, per the batch-c-design.md
producer-derivation rule):

1. isolated `HOME` + `AE_HOME` (separate assignments), dedicated tmux server
   (`-L aeb0tmpl`, `TMUX_TMPDIR` under the harness temp root), real-`~/.ae/config`
   fingerprint tripwire armed for the whole build;
2. real frozen `ae --local b0tmpl` launch in a fresh git repo, agent commands
   `bash` (config at `template/config`);
3. real generated helpers, in order: `spawn dummy2:helper`, `goal`, `state`
   (both panes), `memo add` (two topics), `ask` -> `reply` (closed pair), `ask`
   (left open);
4. the generated `watchdog` was stopped and the tree left to settle before the
   snapshot; the snapshot is the template.

The template's `events.jsonl` also contains two events emitted by ae itself
during the build (`_watchdog` `alert`, `dummy:dummy` `spawn-failed`) — recorded
here because they are part of the fixture bytes.

## Mutation payloads (harvested AFTER the template snapshot, same session lineage)

HDR3
echo "| Payload | Path | sha256 | Producer |"
echo "|---|---|---|---|"
echo "| meta variant | \`payloads/meta.variant\` | \`$(sha "$SB/payloads/meta.variant")\` | real \`spawn dummy2:helper2\` on the template session |"
echo "| meta variant diff vs template meta | \`payloads/meta.variant.diff\` | \`$(sha "$SB/payloads/meta.variant.diff")\` | \`diff -u\` |"
echo "| memo row | \`payloads/memo.row\` | \`$(sha "$SB/payloads/memo.row")\` | real \`memo add --topic mutationtopic\` |"
for i in 1 2 3 4; do
echo "| ask event $i | \`payloads/events.ask.$i\` | \`$(sha "$SB/payloads/events.ask.$i")\` | real \`ask dummy2:helper\` |"
done
cat <<'HDR4'

Recorded properties of the payloads (checked before the arms ran):

- the roster in `payloads/meta.variant` differs from the template meta by the
  two lines `agent.spawned.1=dummy2:helper2:pending` and
  `agent_bin.spawned.1=bash` (one agent entry plus its bin line) — full diff in
  `payloads/meta.variant.diff`;
- the memo row carries topic `mutationtopic`; occurrences of that string in the
  template `memo.tsv`: 0;
- each `events.ask.<i>` carries a distinct `ref`; occurrences of each ref in the
  template `events.jsonl`: 0;
- the `body_file` value inside each harvested ask event names an absolute path
  under the TEMPLATE session directory; the file it names is not present in an
  arm clone's `sessions/b0tmpl/messages/` directory (the arm clones carry only
  the template's own `messages/` files, listed in `template/manifest.tsv`).

## Arms

Every arm starts from a fresh clone of the template AE_HOME
(`cp -a`), with its own clone fingerprint recorded. Every arm ran under
`env -i` plus the allowlisted set recorded in each run's
`env.allowlisted.txt` (`PATH`, `HOME`, `AE_HOME`, `TERM`, `TZ`, `LANG`,
`TMUX_TMPDIR`, `AE_TMUX_SERVER`, `AE_TMUX_SERVER_KIND`, and for active runs
`AE_H507_DIR`, `AE_H507_MAXPOLL`). Outer bounded wait per invocation: 90s
(none expired). Barrier wait in the controller: 60s (none expired).

Every arm carries a per-fixture inactive-equivalence result
(`equiv/RESULT.txt`): the instrumented binary with `AE_H507_DIR` UNSET vs the
uninstrumented frozen binary, each on its own fresh clone of the same template,
compared on `stdout`, `stderr`, `rc`, the recursive after-manifest, and the tmux
probe (the probe's own socket path and server name are harness-per-run values
and are normalised before that one comparison; both raw files are kept).

| Arm | Row id | Mutation (controller action at `H507_AFTER_FACTS`) | Artifacts |
|---|---|---|---|
HDR4
declare -A MUT=( [arm1-stable-control]="none" [arm2-transient-meta]="pass 1 only: writer-shaped temp+rename of sessions/b0tmpl/meta from payloads/meta.variant" [arm3-transient-memo]="pass 1 only: append payloads/memo.row to sessions/b0tmpl/memo.tsv" [arm4-transient-events]="pass 1 only: append payloads/events.ask.1 to sessions/b0tmpl/events.jsonl" [arm5-persistent-events]="EVERY pass: append payloads/events.ask.<pass> to sessions/b0tmpl/events.jsonl (a distinct harvested line per pass)" )
for a in arm1-stable-control arm2-transient-meta arm3-transient-memo arm4-transient-events arm5-persistent-events; do
  echo "| \`$a\` | SC-507b | ${MUT[$a]} | \`arms/$a/\` |"
done
cat <<'HDR5'

### Per-arm captures

HDR5
for a in arm1-stable-control arm2-transient-meta arm3-transient-memo arm4-transient-events arm5-persistent-events; do
  echo "#### \`$a\`"
  echo
  echo "| Item | Value |"
  echo "|---|---|"
  echo "| clone fingerprint (sha256 of \`manifest.before.tsv\`) | \`$(cat "$SB/arms/$a/clone-fingerprint.sha256")\` |"
  echo "| template fingerprint it was cloned from | \`$(cat "$SB/template/fingerprint.sha256")\` |"
  echo "| inactive-equivalence result | \`arms/$a/equiv/RESULT.txt\` — $(grep -c '^EQUAL' "$SB/arms/$a/equiv/RESULT.txt") EQUAL, $(grep -c '^DIFFER' "$SB/arms/$a/equiv/RESULT.txt" || true) DIFFER |"
  echo "| instrumented-inactive stdout sha256 | \`$(sha "$SB/arms/$a/equiv/inactive/stdout.txt")\` (rc $(cat "$SB/arms/$a/equiv/inactive/rc.txt")) |"
  echo "| uninstrumented frozen stdout sha256 | \`$(sha "$SB/arms/$a/equiv/uninstr/stdout.txt")\` (rc $(cat "$SB/arms/$a/equiv/uninstr/rc.txt")) |"
  echo "| ACTIVE run rc | \`$(cat "$SB/arms/$a/active/rc.txt")\` (\`arms/$a/active/rc.txt\`) |"
  echo "| ACTIVE run stdout | \`arms/$a/active/stdout.txt\` — $(wc -c < "$SB/arms/$a/active/stdout.txt" | tr -d ' ') bytes, sha256 \`$(sha "$SB/arms/$a/active/stdout.txt")\` |"
  echo "| ACTIVE run stderr | \`arms/$a/active/stderr.txt\` — $(wc -c < "$SB/arms/$a/active/stderr.txt" | tr -d ' ') bytes, sha256 \`$(sha "$SB/arms/$a/active/stderr.txt")\` |"
  echo "| pass ordinals emitted | $(wc -l < "$SB/arms/$a/h507/pass.log" | tr -d ' ') (\`arms/$a/h507/pass.log\`) |"
  echo "| barrier arrivals | $(grep -c '^arrive' "$SB/arms/$a/h507/barrier.log" || true) (\`arms/$a/h507/barrier.log\`) |"
  echo "| controller action log | \`arms/$a/h507/controller.log\` |"
  echo "| controller stderr surface | \`arms/$a/h507/controller.stdouterr\` — $(wc -c < "$SB/arms/$a/h507/controller.stdouterr" | tr -d ' ') bytes |"
  echo "| INCONCLUSIVE / abort markers | $(ls "$SB/arms/$a/h507/" | grep -cE 'INCONCLUSIVE|ABORTED' || true) |"
  echo "| before-manifest | \`arms/$a/manifest.before.tsv\` |"
  echo "| after-manifest | \`arms/$a/manifest.after.tsv\` |"
  echo "| before/after manifest delta | \`arms/$a/manifest.delta.diff\` — $(grep -c '^[+-][^+-]' "$SB/arms/$a/manifest.delta.diff" || true) changed manifest lines |"
  echo "| tmux snapshot probe | \`arms/$a/active/tmux-probe.txt\` (list-sessions / list-panes / list-clients with their rc; no tmux server is started by this arm) |"
  mf=("$SB/arms/$a"/mutations/*.diff)
  if (( ${#mf[@]} )); then
    echo "| mutation byte diffs | $(for f in "${mf[@]}"; do printf '`arms/%s/mutations/%s` ' "$a" "$(basename "$f")"; done) |"
    echo "| mutation byte facts (sha256/size/inode pre+post) | $(for f in "$SB/arms/$a"/mutations/*.bytes.txt; do printf '`arms/%s/mutations/%s` ' "$a" "$(basename "$f")"; done) |"
  else
    echo "| mutation byte diffs | none (no mutation in this arm) |"
  fi
  if [[ -d "$SB/arms/$a/poststate" ]]; then
    echo "| LEAK-COMPARE post-state control | \`arms/$a/poststate/\` — same template, the arm's named mutation applied cold, frozen UNINSTRUMENTED \`ae archive preview\` run once |"
    echo "| post-state control rc | \`$(cat "$SB/arms/$a/poststate/run/rc.txt")\` |"
    echo "| post-state control stdout | \`arms/$a/poststate/run/stdout.txt\` — $(wc -c < "$SB/arms/$a/poststate/run/stdout.txt" | tr -d ' ') bytes, sha256 \`$(sha "$SB/arms/$a/poststate/run/stdout.txt")\` |"
    echo "| post-state control stderr | \`arms/$a/poststate/run/stderr.txt\` — sha256 \`$(sha "$SB/arms/$a/poststate/run/stderr.txt")\` |"
    echo "| post-state control after-manifest | \`arms/$a/poststate/manifest.after.tsv\` |"
  else
    echo "| LEAK-COMPARE post-state control | not part of this arm's spec |"
  fi
  echo
  echo "\`\`\`"
  echo "--- arms/$a/h507/barrier.log ---"
  cat "$SB/arms/$a/h507/barrier.log"
  echo "--- arms/$a/h507/pass.log ---"
  cat "$SB/arms/$a/h507/pass.log"
  echo "--- arms/$a/h507/controller.log ---"
  cat "$SB/arms/$a/h507/controller.log"
  echo "--- arms/$a/active/stderr.txt ---"
  cat "$SB/arms/$a/active/stderr.txt"
  echo "\`\`\`"
  echo
done
cat <<'FTR'
## Recorded construction facts and limits

- No tmux server exists during any arm; `ae archive preview` was invoked
  directly against a cloned session directory. Each run's `tmux-probe.txt`
  records `list-sessions` / `list-panes` / `list-clients` against that run's own
  socket, with their rc.
- Mutations are performed by the controller only, from a separate process, while
  the instrumented `ae` is blocked at `H507_AFTER_FACTS`. The hooks never read,
  hash, or compute over product state.
- Arm clone paths differ from the template build path, so absolute paths
  recorded inside the fixture (`origin`, `work_dir`, `config`, `ae_path`,
  `tmux_server`, event `body_file`) name the template build locations.
- `ae` was invoked as `<binary> archive preview b0tmpl`; the exact argv of every
  run is recorded at the end of each run's `env.allowlisted.txt`.
- The harness scripts that produced every artifact here are copied into
  `harness/`.
FTR
} > "$M"
echo "wrote $M ($(wc -l < "$M" | tr -d ' ') lines)"
