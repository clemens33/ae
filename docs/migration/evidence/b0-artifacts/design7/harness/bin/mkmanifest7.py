#!/usr/bin/env python3
"""Emit docs/migration/evidence/b0-artifacts/design7/MANIFEST.md from the run tree."""
import os, re, subprocess, sys, hashlib, json

SB="/tmp/aeb0"; D7=f"{SB}/d7"
DEST=sys.argv[1]
ARMS=f"{D7}/arms"

def sha(p):
    try: return hashlib.sha256(open(p,'rb').read()).hexdigest()
    except Exception: return "-"
def rd(p, n=None):
    try:
        t=open(p, encoding='utf-8', errors='replace').read()
        return t if n is None else t[:n]
    except Exception: return ""
def one(p):
    return rd(p).strip() or "-"

out=[]
def w(s=""): out.append(s)

w("# B0 Design 7 — SC-511c frozen-consumer schema-evolution fixtures: run manifest")
w()
w("Captures only. No verdict, classification, or expected-vs-actual claim appears in")
w("this file or in any artifact it indexes.")
w()
w("## STANDING DISCLOSURE (recorded per seat ruling 2026-08-20)")
w()
w("""While looking for the byte source named by this design's `target`/`summary` cohorts
(\"alert events (T-WD precursor bytes)\"), this worker opened
`docs/migration/evidence/twd-precursor.md` and read the whole 96-line file in one
pass — including the `SEAT ANNEX — never included in the worker brief` at its end.
The worker's value-blind rule named b0-design.md's annex, semantic-contract.md and
ownership.md; twd-precursor.md was not on the input list and the annex was not
anticipated. What was thereby read: the T-WD manipulation->reason mapping and a
pointer to `_agents_alert_reasons`' summary-substring classes. Nothing about
SC-507b / SC-511c / SC-1208 rows, no contract values.

**Design 1 (SC-507b) was complete and delivered BEFORE this read.** The seat ruling
(2026-08-20) accepted the disclosure, kept this worker eligible for all Design 7/8
work, and closed the mitigation structurally: producer-derivation already forbids the
worker authoring summary bytes, and under the same ruling the worker produced NO alert
bytes at all — every alert-family specimen here comes from cexec's seat-gated T-WD
harvest, unaltered, with the set-equality proof below.""")
w()
w("## Frozen source of truth and environment")
w()
w("| Item | Value |")
w("|---|---|")
w(f"| frozen commit | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |")
w(f"| frozen `ae` sha256 | `{one(SB+'/frozen/ae.sha256')}` |")
w("| instrumentation | NONE. Design 7 uses no hook patch: every runner is a frozen product surface driven from outside. The only harness insertions are a PATH-first `date` shim and, for telegram, a PATH-first `curl` shim — both delegate-and-log, both recorded below. |")
w(f"| `date` shim | `harness/shim/date` sha256 `{sha(D7+'/shim/date')}` — substitutes ONLY the four frozen now-forms (`date +%s`, `date -u +%FT%TZ`, `date -u +%Y/%m/%d`, `date -u +%Y%m%dT%H%M%SZ`), delegating those substitutions to the real binary via `-r <pinned>`; every other invocation is `exec`'d to the real `date`. Every call is logged with argv + disposition. |")
w(f"| real `date` | `/bin/date` sha256 `{sha('/bin/date')}` |")
w("| pinned consumer clock | `1787191200` (2026-08-20T02:00:00Z) for every family run |")
w("| locale | `TZ=UTC`, `LANG=en_US.UTF-8` — see the measured environment facts below for why NOT `LANG=C` |")
w(f"| environment / tool hashes | `harness/env-record.txt` |")
w()
w("## Measured environment facts (captures, not interpretations)")
w()
w("""1. **`LANG=C` breaks the generated `send`/`ask`/`review` helpers' agent resolution in
   this sandbox.** With `LANG=C` the helpers answer `Error: agent '<a>' not found in
   session '<s>'` for an agent that is present, while `tmux list-panes -s -t <session>
   -F '#{pane_id}|#{@ae_agent}'` under the SAME locale returns that agent's row. Bisected
   over the arm environment one variable at a time (`TZ`, `LANG`, `AE_TMUX_SERVER`,
   `AE_TMUX_SERVER_KIND`, `AE_DATE_SHIM_SUBSTITUTE`); only `LANG` reproduced it. Design 7
   therefore runs at `LANG=en_US.UTF-8`. Design 1 ran at `LANG=C` and never reaches agent
   resolution.
2. **tmux `-t` prefix-matches.** With a session `b0d7x` present, `ae --local b0d7` took the
   session-already-exists branch (`has-session -t b0d7` succeeded against `b0d7x`) and
   created nothing. The cross-session partner session is therefore named `xpartner`, out
   of prefix range of `b0d7`.
3. **`watchdog start` does not carry the caller's `AE_WATCHDOG_*` env into the loop.** It
   launches the loop into a tmux pane; the pane banner reported the DEFAULTS
   (`interval: 60s stale: 15m max nudges: 2`) while the caller had set 2s/1m/1. The
   watchdog family therefore executes the GENERATED watchdog script directly at its `_run`
   verb, where the knobs are honoured (banner: `interval: 2s stale: 1m max nudges: 1`).
   That is execution of the generated program, not function-sourcing.
4. **The telegram daemon initialises an UNSEEN session at EOF** (ae:10133-10138, the
   do-not-replay-history invariant), so a bounded cycle consumes nothing unless its own
   `telegram/state.tsv` is seeded. The controller seeds it at offset 0 with the fixture's
   real `session_id` and the current inode; the action is recorded per run in
   `telegram-seed.txt`, and the daemon's FINAL offset is itself part of the capture.
5. **`ae compact` refuses while spawned agents are present** (\"compact never retires
   someone else's worker\"). The compact runner satisfies that documented precondition
   with the REAL `retire` helper first, captured as `compact-precondition-retire.*`.
6. **A single external `ae stop <name>` emits no stop-request/stop-result**; the fleet path
   (`ae stop all -y`) does. Both the fixture's stop specimens and the stop-verification
   family runner therefore use the fleet form.""")
w()
w("## Alert-family specimens — SET EQUALITY PROOF (binding seat guard)")
w()
w("""Before ANY whole-cohort mutation ran, this worker proved set equality between its
alert-family specimen set and cexec's five-hash enumeration. Full proof:
`alert-specimens/SET-EQUALITY-PROOF.txt`. Summary:""")
w()
w("```")
w(rd(D7+"/alerts/SET-EQUALITY-PROOF.txt"))
w("```")
w()
w("""Nothing under `docs/migration/evidence/batch-c-artifacts/` was written by this worker;
the specimen files were copied out and re-hashed. The five raw lines were appended to the
fixture unaltered, in cexec's `(arm, line_no)` order, as a recorded byte diff
(`fixture/events.extension.diff`).""")
w()
w("## Fixture (producer-derived)")
w()
w("| Item | Value |")
w("|---|---|")
w(f"| template AE_HOME manifest | `fixture/manifest.tsv` |")
w(f"| template fingerprint | `{one(D7+'/template/fingerprint.sha256')}` |")
w("| session | `b0d7` (cross-session partner: `xpartner`) |")
w(f"| events.jsonl | `fixture/events.jsonl` — sha256 `{sha(D7+'/template/.ae/sessions/b0d7/events.jsonl')}` |")
w(f"| events.jsonl BEFORE the alert extension | `fixture/events.pre-extension.jsonl` sha256 `{sha(D7+'/alerts/events.pre-extension.jsonl')}` |")
w("| producers, in order | real `ae --local` launch; `spawn`; a bounded `send` settle probe; `goal` x2; `state` working/waiting-user/blocked/done across two agents; `memo add` x2; `say`; `ask`->`reply` (closed pair); `review` (left open); `ask` (left open); a real cross-session `ask @xpartner:dummy`; real `ae stop all -y` |")
w()
# cohort census
ev=D7+"/template/.ae/sessions/b0d7/events.jsonl"
counts={}; actions={}
n=0
for line in open(ev, encoding='utf-8'):
    line=line.strip()
    if not line: continue
    n+=1; o=json.loads(line)
    for k in o: counts[k]=counts.get(k,0)+1
    actions[o.get('action')]=actions.get(o.get('action'),0)+1
w(f"Fixture size: **{n} lines**. Cohort sizes (a cohort is every specimen line carrying the key):")
w()
w("| Key | Cohort size |")
w("|---|---|")
for k,v in sorted(counts.items(), key=lambda kv:(-kv[1],kv[0])):
    w(f"| `{k}` | {v} |")
w()
w("Action classes present: " + ", ".join(f"`{a}`x{c}" for a,c in sorted(actions.items())) + ".")
w()
w("## Runners (product-layer; exact argv recorded per run in `<label>.invocation.txt`)")
w()
w("""| Family | Frozen runner as executed |
|---|---|
| list/next | `ae list`, `ae list --json`, `ae next` (session resumed, no attached client) |
| watchdog | the GENERATED `watchdog` script executed at its `_run` verb with the documented knobs, SIGTERM-bounded, then `watchdog stop` |
| requests/state | generated `requests all\\|mine\\|inbox`, `state`, plus a real `reply` from the WRONG pane (refusal path) and from the target pane |
| archive/digest | `ae archive preview b0d7` |
| compact | real `ae compact b0d7 --force`, handover answered by the controller driving the REAL `reply` helper from the main pane (`AE_COMPACT_HANDOVER_SECS=45`), after the documented `retire` precondition |
| stop verification | real `ae stop all -y` |
| events-tail | the generated `events-tail` helper: POSITIVE launch barrier (banner + >=1 rendered record, bounded 25s poll), then a capture window closed by SIGTERM (the helper is `tail -f` and never exits) |
| telegram | real `ae telegram start` -> `telegram stop` -> bounded direct daemon cycle, PATH-shimmed `curl` that logs argv, stdin AND the message temp-file body, then exits 7 (never network) |
| aewatch | frozen `contrib/aewatch` `daemon --once --ae-home <sandbox>` with `[telegram] enabled = false` |""")
w()
w("## Arms")
w()
w("""Every family-run gets its OWN fresh clone of the template AE_HOME, fingerprinted
before the mutation. The mutation is whole-cohort: every line carrying the key.
Removal deletes the key/value pair; rename rewrites the key name to `<key>_x`;
the additive arms insert one unknown optional key into EVERY line at the named
object position. Each is applied as a byte-level edit on the raw line (no
re-serialisation, so every unrelated byte survives) and gated by the design's
mutation-validity self-check, run per line:

* removal — `decoded(mutated)` must equal `decoded(control)` minus the named key;
* rename — `decoded(mutated)` must equal `decoded(control)` with the key renamed;
* insert — `decoded(mutated)` must equal `decoded(control)` plus exactly the inserted
  pair, and the inserted key must occupy the requested object position.

A line failing its self-check makes the arm INVALID (`ARM-INVALID.txt`) and no family
runs for it. Self-check reports: `<arm>/<family>/mutation.selfcheck.txt`; byte diffs:
`mutation.bytediff`.""")
w()
w("| Arm | Row id | Mutation | Families | Cohort | Self-check |")
w("|---|---|---|---|---|---|")
def cohort_of(rep):
    m=re.search(r"lines=\d+ cohort=(\d+) selfcheck_failures=(\d+)", rd(rep))
    return (m.group(1), m.group(2)) if m else ("-","-")
for arm in sorted(os.listdir(ARMS)):
    ad=os.path.join(ARMS,arm)
    if not os.path.isdir(ad): continue
    fams=[f for f in sorted(os.listdir(ad)) if os.path.isdir(os.path.join(ad,f))]
    rows=[]
    coh=chk="-"
    for f in fams:
        c,k = cohort_of(os.path.join(ad,f,"mutation.report.txt"))
        if c!="-": coh, chk = c, ("PASS" if k=="0" else f"FAIL({k})")
    label = "SC-511c" if not arm.startswith("ext-") else "SC-511c (empirical-extension lane)"
    at = rd(os.path.join(ad,"ARM.txt"))
    op = (re.search(r"^op=(.*)$", at, re.M) or [None,"-"])[1] if "op=" in at else "-"
    key= (re.search(r"^key=(.*)$", at, re.M) or [None,"-"])[1] if "key=" in at else "-"
    xtra=(re.search(r"^extra=(.*)$", at, re.M) or [None,""])[1] if "extra=" in at else ""
    whr= (re.search(r"^where=(.*)$", at, re.M) or [None,""])[1] if "where=" in at else ""
    if arm == "churn":
        mut = "tmux `set-option -p @ae_agent` on BOTH panes (post clone only); `@ae_slot` and session untouched"
    elif op == "none":
        mut = "none (control)"
    elif op == "remove":
        mut = f"whole-cohort REMOVE of `{key}`"
    elif op == "rename":
        mut = f"whole-cohort RENAME `{key}` -> `{xtra}`"
    elif op == "insert":
        mut = f"whole-cohort INSERT `{key}`=`{xtra}` at object position **{whr}**"
    else:
        mut = f"see `{arm}/ARM.txt`"
    w(f"| `{arm}` | {label} | {mut} | {', '.join(fams)} | {coh} | {chk} |")
w()
w("### Per-family-run artifacts")
w()
w("""Each `<arm>/<family>/` holds: `clone-fingerprint.sha256`, `manifest.before.tsv`,
`manifest.after.tsv`, `manifest.delta.diff`, `events.control.jsonl`,
`events.mutated.jsonl`, `mutation.bytediff`, `mutation.bytes.txt`,
`mutation.report.txt`, `mutation.selfcheck.txt`, `events.after.jsonl`, the family's
`<label>.invocation.txt` / `.stdout.txt` / `.stderr.txt` / `.rc.txt`, its
`date-shim.*.log`, and the `tmux.*.txt` snapshots. Family-specific extras:
`panes.txt` (requests/state), `watchdog.knobs.txt` + `watchdog-run.*` (watchdog),
`curl-shim.log` + `tg.*` + `telegram-seed.txt` (telegram), `aw.*` (aewatch),
`compact.invocation.txt` + `refs.before-compact.txt` (compact),
`churn.controller.txt` + `churn.panes.after.txt` (churn post state).""")
w()
inv=subprocess.run(["find",ARMS,"-name","ARM-INVALID.txt"],capture_output=True,text=True).stdout.split()
inc=subprocess.run(["find",ARMS,"-name","INCONCLUSIVE*"],capture_output=True,text=True).stdout.split()
w(f"**INVALID arms:** {len(inv)}" + ("" if not inv else " — " + ", ".join(x.replace(ARMS+'/','') for x in inv)))
w(f"**INCONCLUSIVE markers:** {len(inc)}" + ("" if not inc else " — " + ", ".join(x.replace(ARMS+'/','') for x in inc)))
w()
w("## Scope notes")
w()
w("""* The consumer matrix per key is b0-design.md's discriminating-consumer column mapped
  onto the design's runner table; the additive arms run every family; the churn arm runs
  the routing/identity families only (requests/state incl. the real reply attempt,
  archive/digest, compact) per the seat ruling of 2026-08-20.
* The churn arm mutates NO file: it is two clones, and on the `post` clone the controller
  rewrites BOTH panes' `@ae_agent` via `tmux set-option -p` while `@ae_slot` and the
  session name stay untouched (the frozen shape at tests/integration@72c7293:1268-1285).
* `body_file` is carried in a separately labelled EMPIRICAL EXTENSION lane
  (`ext-body_file-*`) and is never merged into the stable-key lanes.
* Duplicate-key mutations are deliberately ABSENT: SC-510e/f are Batch C's assignment
  (seat instruction 2026-08-20).
* `ae list --json` carries `last_active_epoch`, which is derived from events.jsonl MTIME
  rather than from any event field, so it varies between runs even under the pinned clock.
  Recorded, not normalised.""")
open(os.path.join(DEST,"MANIFEST.md"),"w",encoding="utf-8").write("\n".join(out)+"\n")
print("wrote", os.path.join(DEST,"MANIFEST.md"), len(out), "lines")
