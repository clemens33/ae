#!/opt/homebrew/bin/bash
set -euo pipefail
shopt -s nullglob
SB=/tmp/aeb0; D8="$SB/d8"
D=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/b0-artifacts/design8
rm -rf "$D"; mkdir -p "$D"/{harness,fixtures}
cp "$D8/bin/fake-tool.sh" "$D8/bin/run-d8.sh" "$D8/bin/mkmanifest8.py" "$D8/bin/assemble8.sh" "$D/harness/"
cp "$D8/fixtures/"*.txt "$D/fixtures/"
{
  echo "# B0 Design 8 environment / tool record"
  echo "captured_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "uname=$(uname -a)"
  echo "frozen_commit=72c729343a0117af2968b66e1c43f89ad25fc0b2"
  echo "frozen_ae_sha256=$(cat "$SB/frozen/ae.sha256")"
  echo "fake_binary_sha256=$(shasum -a 256 "$D8/fakebin/claude" | awk '{print $1}')  (a renamed copy of /opt/homebrew/bin/bash; all five names identical)"
  echo "fake_driver_sha256=$(shasum -a 256 "$D8/bin/fake-tool.sh" | awk '{print $1}')"
  for f in "$D8/fixtures"/*.txt; do echo "fixture $(basename "$f") $(shasum -a 256 "$f" | awk '{print $1}')"; done
  echo "arm_PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
  echo "TZ=UTC LANG=en_US.UTF-8 TERM=xterm-256color"
  for t in /opt/homebrew/bin/bash /opt/homebrew/bin/tmux /usr/bin/git; do
    [[ -e "$t" ]] && echo "tool $t $(shasum -a 256 "$t" | awk '{print $1}')"
  done
  echo "bash=$(/opt/homebrew/bin/bash --version | head -1)"
  echo "tmux=$(/opt/homebrew/bin/tmux -V)"
  echo "real_claude_binary=/Users/ckriech/.local/bin/claude (used ONLY for the idle-screen harvest attempt; no prompt sent)"
  echo "real_codex_binary=/Users/ckriech/.local/bin/codex (used ONLY for the idle-screen harvest; no prompt sent)"
} > "$D/harness/env-record.txt"
for arm in "$D8/arms"/*; do
  [[ -d "$arm" ]] || continue
  a="$(basename "$arm")"
  mkdir -p "$D/$a"
  cp -a "$arm/." "$D/$a/"
  rm -rf "$D/$a/home" "$D/$a/tmuxtmp" "$D/$a/cwd"
done
echo "files=$(find "$D" -type f | wc -l | tr -d ' ') size=$(du -sh "$D" | awk '{print $1}')"
