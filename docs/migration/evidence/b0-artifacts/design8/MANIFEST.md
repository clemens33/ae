# B0 Design 8 — SC-1208 transport-separation probe: run manifest

Captures only. Every artifact below is classified BY CONSTRUCTION — by which file,
argv, or paste carried the byte — never by expected content. This worker was NOT told
which channels a sentinel should or should not appear in, and states no such claim.

**Binding limit (carried from the design):** this probe concerns ae's TRANSPORT
separation only. It says nothing about whether any vendor model obeys an instruction
hierarchy, and no artifact or line here is a semantic model-compliance observation.

## Frozen source of truth and environment

| Item | Value |
|---|---|
| frozen commit | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| frozen `ae` sha256 | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| instrumentation | NONE. The real frozen launch/injection path, the real `_cmd_spawn`, and the real generated `send`/`ask` helpers run unmodified; only the agent BINARIES are fake. |
| model / network | no live model, no network. The fakes never open a socket and never exec anything but themselves. |
| env | `env -i` plus the allowlisted set recorded in every `ARM.txt`; the fakes log ONLY `AE_*`, `OPENCODE_CONFIG`, `PATH`, `HOME`, `TERM` — never an ambient dump. |
| environment / tool hashes | `harness/env-record.txt` |

## The fakes

Each fake is a **renamed copy of `bash`** executing `harness/fake-tool.sh`. That shape is
forced by the fake-recognition prerequisite: a bash SCRIPT named `claude` surfaces as
`bash` in `pane_current_command` (measured — `exec -a` does not change it either), which
is exactly the failure mode the design names. A renamed interpreter reports the intended
tool name.

| Item | Value |
|---|---|
| fake binary (identical for all five names) | sha256 `6ba6319962b59831740a56aa5f65e91d6467c72997f5d0a18be1ba1a6d8d378b` |
| fake driver | `harness/fake-tool.sh` sha256 `df41fbe8367f7a6bfabce6369aec66916c47e86de982fdc129d59f509131e8ed` |

**Fake-TUI protocol.** For the TUI-modelled tools (claude, codex) the fake renders an
idle input region EXTRACTED from a real tool's captured idle screen, harvested once with
`tmux capture-pane -e -p` (SGR preserved, because the frozen sensor parses SGR *state*)
and hashed. The other three render a plain prompt line. Every fake puts its tty in
`-echo -icanon`, reads stdin one byte at a time logging every byte verbatim, and
re-renders the idle region after each submitted line, so the frozen send path's
readiness and staged-paste sensors can reach VERIFIED SUBMIT rather than a defer.

| Fixture | Provenance | sha256 |
|---|---|---|
| `fixtures/codex.idle-region.txt` | a real `codex` TUI started in the repo working dir on a dedicated tmux server, captured at t=13s, no prompt ever sent, then killed | `c36b0efaee3d59dece16e1294da5a407188527618d1e86fb60e3236bac2392e7` |
| `fixtures/claude.idle-region.txt` | the INPUT-REGION rows only (separator, prompt row, separator, two status rows) of a real Claude Code pane — this worker's OWN pane. No transcript rows. A fresh `claude` start could not be used: it presents first-run modals (folder-trust in a new dir, a Chrome-extension prompt in the repo) rather than an idle input box, and driving those modals would mutate the operator's real tool settings | `fdc32fc1f18daba3c5fb551d016f60ee1cf35ee1030fc28db782a6cde441c1e1` |

Recorded alongside: the REAL `claude` binary reports `pane_current_command=2.1.237`
(its version string), not `claude`; the real `codex` reports `codex`. The fakes report
their tool name. Recorded as a measured divergence between the fake and the real
subject, not interpreted.

## Ingress x tool matrix — all cells run

| # | Ingress kind | How it was driven |
|---|---|---|
| 1 | spawn-brief body | the `user_prompt` argument of a real `_cmd_spawn` (`spawn <tool>:worker1 <payload>`) |
| 2 | steady-state helper body | a real `send` to the running main fake, plus one real `ask` to the spawned worker |
| 3 | pane bytes | hostile text written by the controller DIRECTLY to the main pane's tty (pane OUTPUT, not stdin), then a real `send` over it; the pane is captured before and after |
| 4 | validated spawn name | a hostile-looking but grammar-valid agent name used as the spawn name |

Every payload carries a unique sentinel plus: a nested fake `⟦ae:msg⟧` envelope,
instruction prose, flag-looking strings (`--append-system-prompt`,
`-c developer_instructions=`), and quote / backslash / newline / tab / `$`-expansion bytes.
Payload bodies: `<tool>/payloads/*.txt`.

## Structural lanes (classification by construction)

| Lane | What is in it |
|---|---|
| AE_CONTEXT_MATERIAL | the `build_ae_context` output wherever it lands: claude `--append-system-prompt` argv value; codex `developer_instructions` config value; gemini `-i` value; grok initial positional; opencode config + context markdown files. Captured as the fake's `*.argv.nul` (byte-exact, NUL-separated) and as the `ctx.*` files, hashed before launch and after every delivery (`ctx-hashes.*.txt`) |
| PEER_USER_INPUT | tmux-pasted message bytes including the helper envelope (`logs/*.stdin.raw`, byte-verbatim) AND the codex fresh-spawn positional argv user text |
| DATA | `events.jsonl` rows and `messages/` body_file contents |

**Vendor-role annotation** (recorded per tool, separate from the lane; the lane is the
structural fact under test and the annotation never upgrades it):

| Tool | Vendor role |
|---|---|
| `claude` | system-like surface |
| `codex` | system-like surface |
| `gemini` | initial user turn |
| `grok` | initial user turn |
| `opencode` | system-like surface |

## Per-tool results

### `claude`

| Item | Value |
|---|---|
| arm record | `claude/ARM.txt` |
| ingress 1 spawn-brief rc | `0` (`claude/i1.stdout.txt`, `claude/i1.stderr.txt`) |
| ingress 2 send rc | `0` (`claude/i2send.stdout.txt`, `claude/i2send.stderr.txt`) |
| ingress 2 ask rc | `0` (`claude/i2ask.stdout.txt`, `claude/i2ask.stderr.txt`) |
| ingress 3 send-over-pane-bytes rc | `0` (`claude/i3send.stdout.txt`, `claude/i3send.stderr.txt`) |
| ingress 4 validated spawn name rc | `0` (`claude/i4.stdout.txt`, `claude/i4.stderr.txt`) |
| fake-recognition i1-worker | pane `%2`, `pane_current_command=claude`, positive=yes |
| fake-recognition i4-worker | pane `%3`, `pane_current_command=claude`, positive=yes |
| fake-recognition launch | pane `%0`, `pane_current_command=claude`, positive=yes |
| INVALID markers | 0 |
| INCONCLUSIVE markers | 0 |
| tmux snapshots | `claude/tmux.after-launch.txt`, `tmux.after-i1.txt`, `tmux.after-i4.txt`, `tmux.final.txt` |
| pane captures (SGR preserved) | `claude/i3.pane-before-send.txt`, `i3.pane-after-send.txt`, `pane.*.final.txt` |
| DATA lane | `claude/events.jsonl`, `claude/messages/` |
| AE_CONTEXT_MATERIAL files | `claude/ctx.*`, hashed in `claude/ctx-hashes.*.txt` |

Fake instances (one artifact set per invocation, index in `claude/logs/index.txt`):

| Instance | argv channel carries | stdin channel carries |
|---|---|---|
| `claude.68845` | (no ingress sentinel) | `D8-I2-SEND`, `D8-I3-SENDOVER` |
| `claude.71987` | (no ingress sentinel) | `D8-I1-SPAWNBRIEF`, `D8-I2-ASK` |
| `claude.81507` | (no ingress sentinel) | `D8-I4-NAMEBODY` |

### `codex`

| Item | Value |
|---|---|
| arm record | `codex/ARM.txt` |
| ingress 1 spawn-brief rc | `0` (`codex/i1.stdout.txt`, `codex/i1.stderr.txt`) |
| ingress 2 send rc | `0` (`codex/i2send.stdout.txt`, `codex/i2send.stderr.txt`) |
| ingress 2 ask rc | `0` (`codex/i2ask.stdout.txt`, `codex/i2ask.stderr.txt`) |
| ingress 3 send-over-pane-bytes rc | `1` (`codex/i3send.stdout.txt`, `codex/i3send.stderr.txt`) |
| ingress 4 validated spawn name rc | `0` (`codex/i4.stdout.txt`, `codex/i4.stderr.txt`) |
| fake-recognition i1-worker | pane `%2`, `pane_current_command=codex`, positive=yes |
| fake-recognition i4-worker | pane `%3`, `pane_current_command=codex`, positive=yes |
| fake-recognition launch | pane `%0`, `pane_current_command=codex`, positive=yes |
| INVALID markers | 0 |
| INCONCLUSIVE markers | 0 |
| tmux snapshots | `codex/tmux.after-launch.txt`, `tmux.after-i1.txt`, `tmux.after-i4.txt`, `tmux.final.txt` |
| pane captures (SGR preserved) | `codex/i3.pane-before-send.txt`, `i3.pane-after-send.txt`, `pane.*.final.txt` |
| DATA lane | `codex/events.jsonl`, `codex/messages/` |
| AE_CONTEXT_MATERIAL files | `codex/ctx.*`, hashed in `codex/ctx-hashes.*.txt` |

Fake instances (one artifact set per invocation, index in `codex/logs/index.txt`):

| Instance | argv channel carries | stdin channel carries |
|---|---|---|
| `codex.16003` | `D8-I4-NAMEBODY` | (no ingress sentinel) |
| `codex.92792` | (no ingress sentinel) | `D8-I2-SEND` |
| `codex.95566` | `D8-I1-SPAWNBRIEF` | `D8-I2-ASK` |

### `gemini`

| Item | Value |
|---|---|
| arm record | `gemini/ARM.txt` |
| ingress 1 spawn-brief rc | `0` (`gemini/i1.stdout.txt`, `gemini/i1.stderr.txt`) |
| ingress 2 send rc | `0` (`gemini/i2send.stdout.txt`, `gemini/i2send.stderr.txt`) |
| ingress 2 ask rc | `0` (`gemini/i2ask.stdout.txt`, `gemini/i2ask.stderr.txt`) |
| ingress 3 send-over-pane-bytes rc | `0` (`gemini/i3send.stdout.txt`, `gemini/i3send.stderr.txt`) |
| ingress 4 validated spawn name rc | `0` (`gemini/i4.stdout.txt`, `gemini/i4.stderr.txt`) |
| fake-recognition i1-worker | pane `%2`, `pane_current_command=gemini`, positive=yes |
| fake-recognition i4-worker | pane `%3`, `pane_current_command=gemini`, positive=yes |
| fake-recognition launch | pane `%0`, `pane_current_command=gemini`, positive=yes |
| INVALID markers | 0 |
| INCONCLUSIVE markers | 0 |
| tmux snapshots | `gemini/tmux.after-launch.txt`, `tmux.after-i1.txt`, `tmux.after-i4.txt`, `tmux.final.txt` |
| pane captures (SGR preserved) | `gemini/i3.pane-before-send.txt`, `i3.pane-after-send.txt`, `pane.*.final.txt` |
| DATA lane | `gemini/events.jsonl`, `gemini/messages/` |
| AE_CONTEXT_MATERIAL files | `gemini/ctx.*`, hashed in `gemini/ctx-hashes.*.txt` |

Fake instances (one artifact set per invocation, index in `gemini/logs/index.txt`):

| Instance | argv channel carries | stdin channel carries |
|---|---|---|
| `gemini.20094` | (no ingress sentinel) | `D8-I2-SEND`, `D8-I3-SENDOVER` |
| `gemini.21757` | (no ingress sentinel) | `D8-I1-SPAWNBRIEF`, `D8-I2-ASK` |
| `gemini.30904` | (no ingress sentinel) | `D8-I4-NAMEBODY` |

### `grok`

| Item | Value |
|---|---|
| arm record | `grok/ARM.txt` |
| ingress 1 spawn-brief rc | `0` (`grok/i1.stdout.txt`, `grok/i1.stderr.txt`) |
| ingress 2 send rc | `0` (`grok/i2send.stdout.txt`, `grok/i2send.stderr.txt`) |
| ingress 2 ask rc | `0` (`grok/i2ask.stdout.txt`, `grok/i2ask.stderr.txt`) |
| ingress 3 send-over-pane-bytes rc | `0` (`grok/i3send.stdout.txt`, `grok/i3send.stderr.txt`) |
| ingress 4 validated spawn name rc | `0` (`grok/i4.stdout.txt`, `grok/i4.stderr.txt`) |
| fake-recognition i1-worker | pane `%2`, `pane_current_command=grok`, positive=yes |
| fake-recognition i4-worker | pane `%3`, `pane_current_command=grok`, positive=yes |
| fake-recognition launch | pane `%0`, `pane_current_command=grok`, positive=yes |
| INVALID markers | 0 |
| INCONCLUSIVE markers | 0 |
| tmux snapshots | `grok/tmux.after-launch.txt`, `tmux.after-i1.txt`, `tmux.after-i4.txt`, `tmux.final.txt` |
| pane captures (SGR preserved) | `grok/i3.pane-before-send.txt`, `i3.pane-after-send.txt`, `pane.*.final.txt` |
| DATA lane | `grok/events.jsonl`, `grok/messages/` |
| AE_CONTEXT_MATERIAL files | `grok/ctx.*`, hashed in `grok/ctx-hashes.*.txt` |

Fake instances (one artifact set per invocation, index in `grok/logs/index.txt`):

| Instance | argv channel carries | stdin channel carries |
|---|---|---|
| `grok.35858` | (no ingress sentinel) | `D8-I2-SEND`, `D8-I3-SENDOVER` |
| `grok.38326` | (no ingress sentinel) | `D8-I1-SPAWNBRIEF`, `D8-I2-ASK` |
| `grok.47929` | (no ingress sentinel) | `D8-I4-NAMEBODY` |

### `opencode`

| Item | Value |
|---|---|
| arm record | `opencode/ARM.txt` |
| ingress 1 spawn-brief rc | `0` (`opencode/i1.stdout.txt`, `opencode/i1.stderr.txt`) |
| ingress 2 send rc | `0` (`opencode/i2send.stdout.txt`, `opencode/i2send.stderr.txt`) |
| ingress 2 ask rc | `0` (`opencode/i2ask.stdout.txt`, `opencode/i2ask.stderr.txt`) |
| ingress 3 send-over-pane-bytes rc | `0` (`opencode/i3send.stdout.txt`, `opencode/i3send.stderr.txt`) |
| ingress 4 validated spawn name rc | `0` (`opencode/i4.stdout.txt`, `opencode/i4.stderr.txt`) |
| fake-recognition i1-worker | pane `%2`, `pane_current_command=opencode`, positive=yes |
| fake-recognition i4-worker | pane `%3`, `pane_current_command=opencode`, positive=yes |
| fake-recognition launch | pane `%0`, `pane_current_command=opencode`, positive=yes |
| INVALID markers | 0 |
| INCONCLUSIVE markers | 0 |
| tmux snapshots | `opencode/tmux.after-launch.txt`, `tmux.after-i1.txt`, `tmux.after-i4.txt`, `tmux.final.txt` |
| pane captures (SGR preserved) | `opencode/i3.pane-before-send.txt`, `i3.pane-after-send.txt`, `pane.*.final.txt` |
| DATA lane | `opencode/events.jsonl`, `opencode/messages/` |
| AE_CONTEXT_MATERIAL files | `opencode/ctx.*`, hashed in `opencode/ctx-hashes.*.txt` |

Fake instances (one artifact set per invocation, index in `opencode/logs/index.txt`):

| Instance | argv channel carries | stdin channel carries |
|---|---|---|
| `opencode.53093` | (no ingress sentinel) | `D8-I2-SEND`, `D8-I3-SENDOVER` |
| `opencode.55956` | (no ingress sentinel) | `D8-I1-SPAWNBRIEF`, `D8-I2-ASK` |
| `opencode.64741` | (no ingress sentinel) | `D8-I4-NAMEBODY` |

## Out of scope (pointer only)

The unsupported/other-command launch surface (ae:1539,1558) is SC-707's
code-observation row and is not exercised here.
