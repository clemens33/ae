#!/opt/homebrew/bin/bash
set -uo pipefail
DEST=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/twd-precursor
S=/private/tmp/claude-501/-Users-ckriech-projects-clemens33-ae-rust/347d2089-7268-421d-8188-8924e246bbf0/scratchpad
mkdir -p "$DEST/specimens"
for a in a1 a2 a3; do
    C="/tmp/aecx/twd/$a/cap"
    [[ -d "$C" ]] || { echo "MISSING $C"; continue; }
    mkdir -p "$DEST/$a/events" "$DEST/$a/watchdog" "$DEST/$a/panes" "$DEST/$a/tmux" "$DEST/$a/fs-manifests" "$DEST/$a/stamps"
    cp "$C/run-manifest.txt" "$DEST/$a/" 2>/dev/null
    cp "$C"/meta.*.txt "$DEST/$a/" 2>/dev/null
    cp "$C"/ae-launch.* "$DEST/$a/" 2>/dev/null
    cp "$C"/agent-stdin.log "$DEST/$a/" 2>/dev/null
    cp "$C"/producer-view.*.txt "$DEST/$a/" 2>/dev/null
    cp "$C"/events.*.jsonl "$DEST/$a/events/" 2>/dev/null
    cp "$C"/watchdog*.log "$DEST/$a/watchdog/" 2>/dev/null
    cp "$C"/panes.*.txt "$DEST/$a/panes/" 2>/dev/null
    cp "$C"/tmux.*.txt "$DEST/$a/tmux/" 2>/dev/null
    cp "$C"/manifest.*.txt "$DEST/$a/fs-manifests/" 2>/dev/null
    cp "$C"/stamp.*.txt "$DEST/$a/stamps/" 2>/dev/null
    python3 "$S/harness/enumerate.py" "$a" "$C" "$DEST/specimens" > "$DEST/specimens/summary.$a.json"
    ( cd "$DEST/$a" && find . -type f ! -name SHA256SUMS.txt -print0 | sort -z | xargs -0 shasum -a 256 ) > "$DEST/$a/.sums.tmp" 2>/dev/null
    mv "$DEST/$a/.sums.tmp" "$DEST/$a/SHA256SUMS.txt"
done
( cd "$DEST/specimens" && find . -type f ! -name SHA256SUMS.txt ! -name .sums.tmp -print0 | sort -z | xargs -0 shasum -a 256 ) > "$DEST/specimens/.sums.tmp" 2>/dev/null
mv "$DEST/specimens/.sums.tmp" "$DEST/specimens/SHA256SUMS.txt"
echo "PUBLISHED to $DEST"
