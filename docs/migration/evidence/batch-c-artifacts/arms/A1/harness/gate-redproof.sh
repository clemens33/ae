#!/opt/homebrew/bin/bash
# Red-proof suite for the three-half gate. Every class of defect gets one injection on a
# CANDIDATE copy, and the control copy must stay green. An instrument that cannot report
# red is not an instrument.
set -uo pipefail
S="$(dirname "$0")"
A="${1:-/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts}"
RP=/tmp/aecx/gate-redproof; rm -rf "$RP"; mkdir -p "$RP"
cp -R "$A" "$RP/base"; chmod -R u+w "$RP/base"
# The candidate TREE is a copy; the REPO must stay the REAL one, because that is where
# .gitattributes and HEAD live. A stand-in directory is not a work tree and the clone-
# fidelity check refuses to answer from one — deliberately.
REAL_REPO=/Users/ckriech/projects/clemens33/ae-rust
run() { # <label> <mutator-fn>
    rm -rf "$RP/t"; cp -R "$RP/base" "$RP/t"; chmod -R u+w "$RP/t"
    "$2" "$RP/t"
    local out; out="$(BATCH_C_ARTIFACTS="$RP/t" REPO_ROOT="$REAL_REPO" "$S/manifest-tree-gate.sh" "$RP/t" 2>&1)"
    printf '%-34s -> %s\n' "$1" "$(printf '%s' "$out" | grep '## result' | sed 's/## result: //')"
    printf '%s' "$out" | grep -E 'UNRESOLVED|SCHEMA-FAIL|COVERAGE MISMATCH|HASH MISMATCH' | head -2 | sed 's/^/      /'
}
m_control() { :; }
m_cite_tree()   { printf '\nInjected: `hook-patch/no-such.txt`\n' >>"$1/MANIFEST.md"; }
m_cite_arms()   { printf '\nInjected: `arms/A9-nope/case.txt`\n' >>"$1/MANIFEST.md"; }
m_cite_tpl()    { printf '\nInjected: `templates/GZZ/_meta/nope.txt`\n' >>"$1/MANIFEST.md"; }
m_cite_twd()    { printf '\nInjected: `twd-precursor/a9/run-manifest.txt`\n' >>"$1/MANIFEST.md"; }
m_cite_repo()   { printf '\nInjected: `docs/migration/evidence/no-such.md`\n' >>"$1/MANIFEST.md"; }
m_cite_map()    { printf '\nInjected: `G99/no-such-member`\n' >>"$1/MANIFEST.md"; }
m_cite_wild()   { printf '\nInjected: `arms/A1/*/no-such-artifact.txt`\n' >>"$1/MANIFEST.md"; }
m_cite_caserel(){ printf '\nInjected: `out/no-such-consumer.stdout`\n' >>"$1/MANIFEST.md"; }
m_cite_slashless(){ printf '\nInjected: `NO-SUCH-SUMS.txt`\n' >>"$1/MANIFEST.md"; }
m_delete_listed(){ rm -f "$1/arms/A2/c01-filters-ro/env-tab-selfcheck.txt"; }
m_tamper()      { printf 'tampered\n' >>"$1/arms/A3/c01-dead-ro/case.txt"; }
m_paired_delete(){ local v="$1/arms/A3b/c01-dead-over-stale-ro/case.txt"; rm -f "$v"
                   python3 -c "
import sys
p='$1/arms/A3b/SHA256SUMS.txt'
open(p,'w').writelines([l for l in open(p) if 'c01-dead-over-stale-ro/case.txt' not in l])"; }
m_case_removed() { rm -rf "$1/arms/A3b/c02-stale-over-waitinguser-rw"
                   python3 -c "
import sys
p='$1/arms/A3b/SHA256SUMS.txt'
open(p,'w').writelines([l for l in open(p) if 'c02-stale-over-waitinguser-rw/' not in l])"; }
m_index_gone()  { rm -f "$1/arms/A3b/CASES.tsv"
                  python3 -c "
import sys
p='$1/arms/A3b/SHA256SUMS.txt'
open(p,'w').writelines([l for l in open(p) if 'CASES.tsv' not in l])"; }
m_ledger_edit() { printf 'seq=999\tforged\n' >>"$1/arms/A3b/c01-dead-over-stale-ro/admissibility-ledger.txt"
                  python3 -c "
import hashlib,sys
d='$1/arms/A3b/c01-dead-over-stale-ro/admissibility-ledger.txt'
h=hashlib.sha256(open(d,'rb').read()).hexdigest()
p='$1/arms/A3b/SHA256SUMS.txt'
out=[]
for l in open(p):
    if 'c01-dead-over-stale-ro/admissibility-ledger.txt' in l:
        l=h+'  ./c01-dead-over-stale-ro/admissibility-ledger.txt\n'
    out.append(l)
open(p,'w').writelines(out)"; }
# NEW IN v4 — the class the working-tree checks cannot see.
m_normalized() {
    # A tracked evidence file whose BYTES are changed and whose SUMS entry is updated to
    # match: coverage balances, every recorded hash verifies against the working file, and
    # only a comparison against the committed blob can tell that a clone would differ.
    local f="$1/arms/A4/c01-status-live-live/case.txt"
    printf 'normalized-by-a-filter
' >>"$f"
    python3 -c "
import hashlib,sys
f='$f'; d='$1/arms/A4'
h=hashlib.sha256(open(f,'rb').read()).hexdigest()
p=d+'/SHA256SUMS.txt'; out=[]
for l in open(p):
    if l.endswith('./c01-status-live-live/case.txt\n'):
        l=h+'  ./c01-status-live-live/case.txt\n'
    out.append(l)
open(p,'w').writelines(out)"
}
m_twd_sums() {
    # twd-precursor was invisible to the old hardcoded SUMS loop.
    printf 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  ./phantom.txt
'         >>"$1/twd-precursor/a1/SHA256SUMS.txt"
}

echo "## gate red-proof suite"
run "control (untouched)"            m_control
run "citation: tree base"            m_cite_tree
run "citation: arms base"            m_cite_arms
run "citation: templates base"       m_cite_tpl
run "citation: twd base"             m_cite_twd
run "citation: repo base"            m_cite_repo
run "citation: group/member mapping" m_cite_map
run "citation: empty wildcard"       m_cite_wild
run "citation: case-relative"        m_cite_caserel
run "citation: slash-less file"      m_cite_slashless
run "sums: listed file deleted"      m_delete_listed
run "sums: bytes tampered"           m_tamper
run "schema: file+SUMS line deleted" m_paired_delete
run "schema: case dir + SUMS gone"   m_case_removed
run "schema: case index deleted"     m_index_gone
run "schema: ledger edited + rehashed" m_ledger_edit
run "committed: bytes changed + SUMS updated" m_normalized
run "sums: twd-precursor phantom entry"  m_twd_sums
