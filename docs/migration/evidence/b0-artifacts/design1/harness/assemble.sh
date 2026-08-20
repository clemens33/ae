#!/opt/homebrew/bin/bash
set -euo pipefail
shopt -s nullglob
SB=/tmp/aeb0
D=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/b0-artifacts/design1
rm -rf "$D"; mkdir -p "$D"/{harness,template,payloads,arms}
cp "$SB/frozen/ae.sha256" "$D/harness/frozen-ae.sha256"
cp "$SB/instr/ae.sha256" "$D/harness/instrumented-ae.sha256"
cp "$SB/instr/h507.patch" "$D/harness/h507.patch"
cp "$SB/instr/h507.patch.sha256" "$D/harness/h507.patch.sha256"
cp "$SB/env-record.txt" "$D/harness/env-record.txt"
cp "$SB/bin/run-arm.sh" "$SB/bin/manifest.sh" "$SB/bin/build-template.sh" "$SB/bin/harvest.sh" "$SB/bin/harvest2.sh" "$SB/bin/envrec.sh" "$SB/bin/assemble.sh" "$D/harness/"
cp "$SB/template/manifest.tsv" "$D/template/manifest.tsv"
cp "$SB/template/fingerprint.sha256" "$D/template/fingerprint.sha256"
mkdir -p "$D/template/session"
cp -a "$SB/template/.ae/sessions/b0tmpl/meta" "$SB/template/.ae/sessions/b0tmpl/memo.tsv" "$SB/template/.ae/sessions/b0tmpl/events.jsonl" "$D/template/session/"
cp -a "$SB/template/.ae/sessions/b0tmpl/messages" "$D/template/session/messages"
cp -a "$SB/template/.ae/config" "$D/template/config"
cp "$SB/payloads/meta.variant" "$SB/payloads/meta.variant.diff" "$SB/payloads/memo.row" "$D/payloads/"
for i in 1 2 3 4; do cp "$SB/payloads/events.ask.$i" "$D/payloads/"; done
for a in arm1-stable-control arm2-transient-meta arm3-transient-memo arm4-transient-events arm5-persistent-events; do
  mkdir -p "$D/arms/$a/active" "$D/arms/$a/h507" "$D/arms/$a/equiv/inactive" "$D/arms/$a/equiv/uninstr"
  cp "$SB/arms/$a/clone-fingerprint.sha256" "$D/arms/$a/"
  cp "$SB/arms/$a/manifest.before.tsv" "$SB/arms/$a/manifest.after.tsv" "$SB/arms/$a/manifest.delta.diff" "$D/arms/$a/"
  cp "$SB/arms/$a"/active/{stdout.txt,stderr.txt,rc.txt,env.allowlisted.txt,tmux-probe.txt} "$D/arms/$a/active/"
  for f in "$SB/arms/$a"/h507/*.log "$SB/arms/$a"/h507/*.seq "$SB/arms/$a"/h507/controller.stdouterr; do cp "$f" "$D/arms/$a/h507/"; done
  cp "$SB/arms/$a/equiv/RESULT.txt" "$D/arms/$a/equiv/"
  for sub in inactive uninstr; do
    cp "$SB/arms/$a/equiv/$sub"/{stdout.txt,stderr.txt,rc.txt,env.allowlisted.txt,tmux-probe.txt} "$D/arms/$a/equiv/$sub/"
  done
  for f in "$SB/arms/$a"/equiv/*.manifest.after.tsv; do cp "$f" "$D/arms/$a/equiv/"; done
  mf=("$SB/arms/$a"/mutations/*.diff "$SB/arms/$a"/mutations/*.bytes.txt)
  if (( ${#mf[@]} )); then mkdir -p "$D/arms/$a/mutations"; for f in "${mf[@]}"; do cp "$f" "$D/arms/$a/mutations/"; done; fi
  if [[ -d "$SB/arms/$a/poststate" ]]; then
    mkdir -p "$D/arms/$a/poststate/run"
    cp "$SB/arms/$a/poststate/controller.log" "$SB/arms/$a/poststate/manifest.after.tsv" "$D/arms/$a/poststate/"
    cp "$SB/arms/$a"/poststate/run/{stdout.txt,stderr.txt,rc.txt,env.allowlisted.txt,tmux-probe.txt} "$D/arms/$a/poststate/run/"
  fi
done
echo "files: $(find "$D" -type f | wc -l | tr -d ' ')  size: $(du -sh "$D" | awk '{print $1}')"
