#!/usr/bin/env python3
"""Emit b0-artifacts/design8/MANIFEST.md from the D8 run tree."""
import os, sys, hashlib, subprocess, re
SB="/tmp/aeb0"; D8=f"{SB}/d8"; ARMS=f"{D8}/arms"; DEST=sys.argv[1]
TOOLS=["claude","codex","gemini","grok","opencode"]
VENDOR={"claude":"system-like surface","codex":"system-like surface","opencode":"system-like surface",
        "gemini":"initial user turn","grok":"initial user turn"}
def sha(p):
    try: return hashlib.sha256(open(p,'rb').read()).hexdigest()
    except Exception: return "-"
def rd(p):
    try: return open(p,encoding='utf-8',errors='replace').read()
    except Exception: return ""
def one(p): return rd(p).strip() or "-"
def sentinels_in(p, nul=False):
    try:
        b=open(p,'rb').read()
        t=b.replace(b'\0',b'\n').decode('utf-8','replace') if nul else b.decode('utf-8','replace')
    except Exception: return []
    return sorted(set(re.findall(r'D8-I\d-[A-Z]+', t)))
out=[]
def w(s=""): out.append(s)

w("# B0 Design 8 — SC-1208 transport-separation probe: run manifest")
w()
w("""Captures only. Every artifact below is classified BY CONSTRUCTION — by which file,
argv, or paste carried the byte — never by expected content. This worker was NOT told
which channels a sentinel should or should not appear in, and states no such claim.""")
w()
w("**Binding limit (carried from the design):** this probe concerns ae's TRANSPORT")
w("separation only. It says nothing about whether any vendor model obeys an instruction")
w("hierarchy, and no artifact or line here is a semantic model-compliance observation.")
w()
w("## Frozen source of truth and environment")
w()
w("| Item | Value |")
w("|---|---|")
w("| frozen commit | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |")
w(f"| frozen `ae` sha256 | `{one(SB+'/frozen/ae.sha256')}` |")
w("| instrumentation | NONE. The real frozen launch/injection path, the real `_cmd_spawn`, and the real generated `send`/`ask` helpers run unmodified; only the agent BINARIES are fake. |")
w("| model / network | no live model, no network. The fakes never open a socket and never exec anything but themselves. |")
w("| env | `env -i` plus the allowlisted set recorded in every `ARM.txt`; the fakes log ONLY `AE_*`, `OPENCODE_CONFIG`, `PATH`, `HOME`, `TERM` — never an ambient dump. |")
w(f"| environment / tool hashes | `harness/env-record.txt` |")
w()
w("## The fakes")
w()
w("""Each fake is a **renamed copy of `bash`** executing `harness/fake-tool.sh`. That shape is
forced by the fake-recognition prerequisite: a bash SCRIPT named `claude` surfaces as
`bash` in `pane_current_command` (measured — `exec -a` does not change it either), which
is exactly the failure mode the design names. A renamed interpreter reports the intended
tool name.""")
w()
w("| Item | Value |")
w("|---|---|")
w(f"| fake binary (identical for all five names) | sha256 `{sha(D8+'/fakebin/claude')}` |")
w(f"| fake driver | `harness/fake-tool.sh` sha256 `{sha(D8+'/bin/fake-tool.sh')}` |")
w()
w("""**Fake-TUI protocol.** For the TUI-modelled tools (claude, codex) the fake renders an
idle input region EXTRACTED from a real tool's captured idle screen, harvested once with
`tmux capture-pane -e -p` (SGR preserved, because the frozen sensor parses SGR *state*)
and hashed. The other three render a plain prompt line. Every fake puts its tty in
`-echo -icanon`, reads stdin one byte at a time logging every byte verbatim, and
re-renders the idle region after each submitted line, so the frozen send path's
readiness and staged-paste sensors can reach VERIFIED SUBMIT rather than a defer.""")
w()
w("| Fixture | Provenance | sha256 |")
w("|---|---|---|")
w(f"| `fixtures/codex.idle-region.txt` | a real `codex` TUI started in the repo working dir on a dedicated tmux server, captured at t=13s, no prompt ever sent, then killed | `{sha(D8+'/fixtures/codex.idle-region.txt')}` |")
w(f"| `fixtures/claude.idle-region.txt` | the INPUT-REGION rows only (separator, prompt row, separator, two status rows) of a real Claude Code pane — this worker's OWN pane. No transcript rows. A fresh `claude` start could not be used: it presents first-run modals (folder-trust in a new dir, a Chrome-extension prompt in the repo) rather than an idle input box, and driving those modals would mutate the operator's real tool settings | `{sha(D8+'/fixtures/claude.idle-region.txt')}` |")
w()
w("""Recorded alongside: the REAL `claude` binary reports `pane_current_command=2.1.237`
(its version string), not `claude`; the real `codex` reports `codex`. The fakes report
their tool name. Recorded as a measured divergence between the fake and the real
subject, not interpreted.""")
w()
w("## Ingress x tool matrix — all cells run")
w()
w("""| # | Ingress kind | How it was driven |
|---|---|---|
| 1 | spawn-brief body | the `user_prompt` argument of a real `_cmd_spawn` (`spawn <tool>:worker1 <payload>`) |
| 2 | steady-state helper body | a real `send` to the running main fake, plus one real `ask` to the spawned worker |
| 3 | pane bytes | hostile text written by the controller DIRECTLY to the main pane's tty (pane OUTPUT, not stdin), then a real `send` over it; the pane is captured before and after |
| 4 | validated spawn name | a hostile-looking but grammar-valid agent name used as the spawn name |""")
w()
w("Every payload carries a unique sentinel plus: a nested fake `⟦ae:msg⟧` envelope,")
w("instruction prose, flag-looking strings (`--append-system-prompt`,")
w("`-c developer_instructions=`), and quote / backslash / newline / tab / `$`-expansion bytes.")
w("Payload bodies: `<tool>/payloads/*.txt`.")
w()
w("## Structural lanes (classification by construction)")
w()
w("""| Lane | What is in it |
|---|---|
| AE_CONTEXT_MATERIAL | the `build_ae_context` output wherever it lands: claude `--append-system-prompt` argv value; codex `developer_instructions` config value; gemini `-i` value; grok initial positional; opencode config + context markdown files. Captured as the fake's `*.argv.nul` (byte-exact, NUL-separated) and as the `ctx.*` files, hashed before launch and after every delivery (`ctx-hashes.*.txt`) |
| PEER_USER_INPUT | tmux-pasted message bytes including the helper envelope (`logs/*.stdin.raw`, byte-verbatim) AND the codex fresh-spawn positional argv user text |
| DATA | `events.jsonl` rows and `messages/` body_file contents |""")
w()
w("**Vendor-role annotation** (recorded per tool, separate from the lane; the lane is the")
w("structural fact under test and the annotation never upgrades it):")
w()
w("| Tool | Vendor role |")
w("|---|---|")
for t in TOOLS: w(f"| `{t}` | {VENDOR[t]} |")
w()
w("## Per-tool results")
w()
for t in TOOLS:
    a=os.path.join(ARMS,t)
    if not os.path.isdir(a):
        w(f"### `{t}`"); w(); w("NOT RUN."); w(); continue
    w(f"### `{t}`")
    w()
    w("| Item | Value |")
    w("|---|---|")
    w(f"| arm record | `{t}/ARM.txt` |")
    for lab,f in [("ingress 1 spawn-brief","i1"),("ingress 2 send","i2send"),("ingress 2 ask","i2ask"),
                  ("ingress 3 send-over-pane-bytes","i3send"),("ingress 4 validated spawn name","i4")]:
        rc=one(os.path.join(a,f+".rc.txt"))
        w(f"| {lab} rc | `{rc}` (`{t}/{f}.stdout.txt`, `{t}/{f}.stderr.txt`) |")
    recs=sorted(x for x in os.listdir(a) if x.startswith("recognition."))
    for r in recs:
        txt=rd(os.path.join(a,r))
        m=re.search(r"pane=(\S+) intended_tool=(\S+) pane_current_command=(\S*)", txt)
        ok=re.search(r"positively_identifies_as_intended_tool=(\S+)", txt)
        if m: w(f"| fake-recognition {r[len('recognition.'):-4]} | pane `{m.group(1)}`, `pane_current_command={m.group(3)}`, positive={ok.group(1) if ok else '-'} |")
    inv=[x for x in os.listdir(a) if x.startswith("ARM-INVALID")]
    inc=[x for x in os.listdir(a) if x.startswith("INCONCLUSIVE")]
    w(f"| INVALID markers | {len(inv)}{'' if not inv else ' — '+', '.join(inv)} |")
    w(f"| INCONCLUSIVE markers | {len(inc)}{'' if not inc else ' — '+', '.join(inc)} |")
    w(f"| tmux snapshots | `{t}/tmux.after-launch.txt`, `tmux.after-i1.txt`, `tmux.after-i4.txt`, `tmux.final.txt` |")
    w(f"| pane captures (SGR preserved) | `{t}/i3.pane-before-send.txt`, `i3.pane-after-send.txt`, `pane.*.final.txt` |")
    w(f"| DATA lane | `{t}/events.jsonl`, `{t}/messages/` |")
    w(f"| AE_CONTEXT_MATERIAL files | `{t}/ctx.*`, hashed in `{t}/ctx-hashes.*.txt` |")
    w()
    logs=os.path.join(a,"logs")
    if os.path.isdir(logs):
        w(f"Fake instances (one artifact set per invocation, index in `{t}/logs/index.txt`):")
        w()
        w("| Instance | argv channel carries | stdin channel carries |")
        w("|---|---|---|")
        for inst in sorted(set(x.rsplit('.',2)[0] for x in os.listdir(logs) if x.endswith('.argv.nul') or x.endswith('.stdin.raw'))):
            av=os.path.join(logs,inst+".argv.nul"); sr=os.path.join(logs,inst+".stdin.raw")
            w(f"| `{inst}` | {', '.join('`'+x+'`' for x in sentinels_in(av,nul=True)) or '(no ingress sentinel)'} | {', '.join('`'+x+'`' for x in sentinels_in(sr)) or '(no ingress sentinel)'} |")
        w()
w("## Out of scope (pointer only)")
w()
w("The unsupported/other-command launch surface (ae:1539,1558) is SC-707's")
w("code-observation row and is not exercised here.")
open(os.path.join(DEST,"MANIFEST.md"),"w",encoding="utf-8").write("\n".join(out)+"\n")
print("wrote", len(out), "lines")
