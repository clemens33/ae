#!/opt/homebrew/bin/bash
set -euo pipefail
shopt -s nullglob
SB=/tmp/aeb0; D7="$SB/d7"
D=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/b0-artifacts/design7
rm -rf "$D"; mkdir -p "$D"/{harness/shim,harness/bin,fixture/session,alert-specimens,arms}
cp "$D7/bin/"*.sh "$D7/bin/"*.py "$D/harness/bin/" 2>/dev/null || true
cp "$D7/shim/date" "$D/harness/shim/date"
cp "$SB/bin/manifest.sh" "$D/harness/bin/manifest.sh"
{
  echo "# B0 Design 7 environment / tool record"
  echo "captured_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "uname=$(uname -a)"
  echo "frozen_commit=72c729343a0117af2968b66e1c43f89ad25fc0b2"
  echo "frozen_ae_sha256=$(cat "$SB/frozen/ae.sha256")"
  echo "template_fingerprint_sha256=$(cat "$D7/template/fingerprint.sha256")"
  echo "date_shim_sha256=$(shasum -a 256 "$D7/shim/date" | awk '{print $1}')"
  echo "arm_PATH=$D7/shim:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
  echo "TZ=UTC LANG=en_US.UTF-8"
  echo "pinned_consumer_clock_epoch=1787191200"
  for t in /opt/homebrew/bin/bash /opt/homebrew/bin/tmux /usr/bin/git /bin/date /usr/bin/python3 /usr/bin/jq /usr/bin/awk /usr/bin/sed /usr/bin/grep /usr/bin/shasum /usr/bin/stat; do
    [[ -e "$t" ]] && echo "tool $t $(shasum -a 256 "$t" | awk '{print $1}')"
  done
  echo "bash=$(/opt/homebrew/bin/bash --version | head -1)"
  echo "tmux=$(/opt/homebrew/bin/tmux -V)"
  echo "python3=$(/usr/bin/python3 --version 2>&1)"
  echo "jq=$(jq --version)"
} > "$D/harness/env-record.txt"
# fixture
cp "$D7/template/manifest.tsv" "$D7/template/fingerprint.sha256" "$D/fixture/"
cp "$D7/template/.ae/config" "$D/fixture/config"
S="$D7/template/.ae/sessions/b0d7"
cp "$S/meta" "$S/memo.tsv" "$S/events.jsonl" "$D/fixture/session/"
cp -a "$S/messages" "$D/fixture/session/messages"
cp "$D7/alerts/events.pre-extension.jsonl" "$D7/alerts/events.extension.diff" "$D/fixture/"
cp "$D7/build/producers.log" "$D/fixture/producers.log"
cp "$D7/build/date-shim.log" "$D/fixture/date-shim.build.log"
# alert specimens
cp "$D7/alerts/SET-EQUALITY-PROOF.txt" "$D7/alerts/alert.lines.jsonl" "$D/alert-specimens/"
cp "$D7/alerts/source/"* "$D/alert-specimens/"
# arms: everything except live clone trees
for arm in "$D7/arms"/*; do
  [[ -d "$arm" ]] || continue
  a="$(basename "$arm")"
  mkdir -p "$D/arms/$a"
  [[ -f "$arm/ARM.txt" ]] && cp "$arm/ARM.txt" "$D/arms/$a/"
  for fam in "$arm"/*; do
    [[ -d "$fam" ]] || continue
    f="$(basename "$fam")"
    mkdir -p "$D/arms/$a/$f"
    for x in "$fam"/*; do
      [[ -f "$x" ]] && cp "$x" "$D/arms/$a/$f/"
    done
  done
done
echo "files=$(find "$D" -type f | wc -l | tr -d ' ') size=$(du -sh "$D" | awk '{print $1}')"
