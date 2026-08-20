#!/opt/homebrew/bin/bash
# Red-proof for the clone-fidelity check, in an ISOLATED scratch repository so the live
# repo's index is never touched. Reproduces the exact ce8965e shape: a CRLF evidence file
# under core.autocrlf=input, where the working bytes and the stored blob disagree while the
# recorded hash matches the working file.
set -uo pipefail
S="$(dirname "$0")"
R=/tmp/aecx/cbrp; rm -rf "$R"; mkdir -p "$R/repo/ev/arm/case"
cd "$R/repo"
git init -q .; git config user.email p@p; git config user.name p
git config core.autocrlf input
printf 'ordinary line\n'            > ev/arm/case/plain.txt
printf 'pty log line one\r\nline two\r\n' > ev/arm/case/scripted-client.log
( cd ev/arm && find . -type f ! -name SHA256SUMS.txt -print0 | sort -z | xargs -0 shasum -a 256 ) > /tmp/aecx/cbrp.sums
mv /tmp/aecx/cbrp.sums ev/arm/SHA256SUMS.txt
echo "## A. CONTROL — no attributes yet, but nothing committed either"
echo "## A. INJECTION — CRLF file, autocrlf=input, NO -text attribute"
python3 "$S/committed-bytes-check.py" "$R/repo/ev" "$R/repo" "ev" 2>&1 | grep -E 'A_would_normalize|NORMALIZE-FAIL|oid ' | sed 's/^/   /'
echo
echo "## A. FIX — the same tree with the -text attribute the live repo now carries"
printf 'ev/** -text\n' > .gitattributes
python3 "$S/committed-bytes-check.py" "$R/repo/ev" "$R/repo" "ev" 2>&1 | grep -E 'A_would_normalize' | sed 's/^/   /'
echo
echo "## B. INJECTION — commit the tree, then corrupt one recorded hash so the HEAD blob disagrees"
git add -A >/dev/null 2>&1; git commit -qm evidence >/dev/null 2>&1
python3 - <<'PY'
p="/tmp/aecx/cbrp/repo/ev/arm/SHA256SUMS.txt"
out=[]
for l in open(p):
    if l.endswith("./case/plain.txt\n"):
        l = "0"*64 + "  ./case/plain.txt\n"
    out.append(l)
open(p,"w").writelines(out)
PY
python3 "$S/committed-bytes-check.py" "$R/repo/ev" "$R/repo" "ev" 2>&1 | grep -E 'B_head_mismatch|HEAD-BYTES-FAIL|recorded |HEAD blob|working ' | sed 's/^/   /'
echo
echo "## B. CONTROL — restore the recorded hash"
( cd "$R/repo/ev/arm" && find . -type f ! -name SHA256SUMS.txt -print0 | sort -z | xargs -0 shasum -a 256 ) > /tmp/aecx/cbrp.sums
mv /tmp/aecx/cbrp.sums "$R/repo/ev/arm/SHA256SUMS.txt"
python3 "$S/committed-bytes-check.py" "$R/repo/ev" "$R/repo" "ev" 2>&1 | grep -E 'A_would_normalize|B_head_mismatch' | sed 's/^/   /'
