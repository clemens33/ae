#!/opt/homebrew/bin/bash
# A5 fixture: an AE_HOME whose config the doctor can fully resolve, so the CLEAN arm has
# no failures of its own and every failure in a planted arm is attributable to the one
# planted removal. Produced by a real ae launch so the meta and helper set are producer-
# derived, then the session is stopped so no live topology is implied.
source "$(dirname "$0")/tlib.sh"
run() { echo "  as=$1 helper='${*:2}'" >>"$P"; as_agent "$@"; echo "    rc=$?" >>"$P"; }
t_sandbox a5 "fake:worker"
t_launch ta5 || { echo FAILED; cat "$CAP/ae-launch.err"; exit 1; }
LEAD="$(pane_of fake:lead)"
P="$CAP/prov.txt"; : >"$P"
{ echo "construction: a real 2-agent launch, ordinary traffic, then the real ae stop, so the"
  echo "fixture is a settled on-disk session rather than a live one. The config names the"
  echo "agent alias by ABSOLUTE path, so the doctor's agent: check resolves through the"
  echo "controlled bin dir like every other checklist item."
} >>"$P"
run "$LEAD" state working "A5 doctor fixture traffic"
run "$LEAD" goal "A5 doctor fixture goal"
"$HARNESS_BASH" "$FROZEN_AE" stop ta5 </dev/null >"$CAP/stop.out" 2>"$CAP/stop.err"
echo "  ae stop rc=$?" >>"$P"
mkdir -p "$TSTORE/A5/_meta"
echo "A5/doctor-fixture fingerprint(pre)=$(t_store A5 doctor-fixture "$P")"
t_protect A5 doctor-fixture >/dev/null
t_teardown
echo "TPL-A5 DONE"
