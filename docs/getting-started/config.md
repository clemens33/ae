# Configuration

Run `ae init` to discover supported harness executables on `PATH` and propose `~/.ae/config`, with a harness, model and effort picked per role.
It never runs a harness or checks login/model access. A first session launch still auto-creates
the broad default when the file is absent. Per-project overrides go in `.ae/config` inside your
project directory (the project file shadows the global file key by key).

## Example

```toml
[clients]
claude = claude
codex = codex
grok = grok

[profiles]
opus5 = "claude --permission-mode bypassPermissions --model claude-opus-5 --effort xhigh"
gpt56sol = "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh"
fable5 = "claude --permission-mode bypassPermissions --model fable --effort xhigh"
gpt6astra = "codex --yolo -m gpt-6-astra -c model_reasoning_effort=xhigh"
gpt56luna = "codex -m gpt-5.6-luna -c model_reasoning_effort=xhigh -a never"
grok46 = "grok --always-approve -m grok-4.6 --effort high"
muse = "muse"

[prices]
sol_discount = gpt-5.6-sol,1.25,2.5,0.125,10

[roster]
lead = fable5
colead = gpt6astra
orchestrator = gpt56luna

[workspace]
main = lead
workers = colead
layout = lead-pair
watchdog = true
quota = on
quota_every_secs = 300
idle_nudge_secs = 300
done_confirmations = 2
# auto_upgrade = on

[prompt]
instructions = "Always write tests. Prefer TypeScript."
```

## `[clients]`

A client names one installed CLI instance: its executable plus, optionally, its account/config
directory. Profiles build flags and permissions on that name. The default entries are no-op
aliases such as `claude = claude`; this keeps the executable choice in one place without changing
existing profile commands.

The value is one executable word followed by at most one `config_home=<path>` assignment. The
path must be absolute or rooted at `$HOME`/`${HOME}`. ae expands it once when it resolves the
profile; other variables are refused because a client path must not depend on a pane's ambient
environment. Only Claude Code and Codex have verified account-directory variables:

| Tool | `config_home` sets | Default store |
|---|---|---|
| Claude Code | `CLAUDE_CONFIG_DIR` | `$HOME/.claude` |
| Codex | `CODEX_HOME` | `$HOME/.codex` |

`config_home` is refused for Grok, agy, Muse, OpenCode, and Gemini until their account-directory
contracts are verified.

## `[profiles]`

Register any CLI tool as a profile — a reusable launch recipe. The value is the shell command to launch it. ae extracts the executable name from the command and verifies it's on `PATH` during `ae doctor`.

### OpenCode model variants

An OpenCode profile can request a model variant by ending the model word in `#<variant>`, for example
`opencode --agent build --model opencode-go/glm-5.3-flash#max`. ae passes the whole model word to
OpenCode in the inline config layer and does not parse, validate or rewrite what follows the `#`. It is
a request, not an enforcement: ae cannot see or set the variant a running OpenCode selects.

Write the whole profile value in double quotes, as in
`go = "opencode --agent build --model p/m#max"`. Outside a fully double-quoted value the config reader
treats `#` as the start of a comment and drops the rest of the line, so the model would lose its suffix.

What OpenCode 2.2.5 does with the request was read from its source at commit `9bbb8ce1`, not measured
by ae, and later releases may differ. A new session takes the saved per-model variant preference if one
exists, an explicit Default included, before the agent model's variant and then the root model's
variant, and drops a variant the model's catalog does not list. An existing session first uses an
in-memory selection, then its stored model variant when the session's agent is absent or equals the
current agent. The saved preference is shared by the OpenCode sessions that use the same preference store, and OpenCode's
`/variants` command writes it; ae never does.

A recorded session id that ae can use resumes with `--session <id>` and the same inline layer, and
ae never learns whether OpenCode still has that session; an absent or unusable id starts fresh, and so
does a changed model. `ae doctor` names what the current profile requests on its `agent:<profile>` row
and says the effective variant cannot be verified; see [ae doctor](../reference/commands.md#ae-doctor).

### Multiple identities of one CLI

One binary can serve several logins/subscriptions. Give each non-default account a client, then
use that client name as the executable word in its profiles:

```toml
[clients]
cc = claude
cc-mic = claude config_home=$HOME/.claude-mic
codex = codex
codex-mic = codex config_home=${HOME}/.codex-mic

[profiles]
fable5 = "cc --permission-mode bypassPermissions --model fable --effort xhigh"
fablemic = "cc-mic --permission-mode bypassPermissions --model fable --effort xhigh"
solmic = "codex-mic --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh"
```

For a one-off seat on another account, skip the duplicate profile and say so at launch instead:
`ae myproject --lead fable5@cc-mic` runs the `fable5` flags against the `cc-mic` account. The
`@` spelling works on `--lead`, `--colead` and `--seat`, and on `spawn --using` once you approve
the pair (below). Both sides must be the same known tool, and the recorded client is
write-once for the session: a flagless resume honors it by itself, while an explicit
re-pairing repeats its exact label. See
[Commands](../reference/commands.md) for the refusal rules.

### `[client_pairs]`: which pairs an agent may spawn

A launch is yours, so the `@` spelling there needs no permission. A spawn is an agent's, so
`spawn --using fable5@cc-mic` runs only for a pair you listed, in the GLOBAL config:

```toml
[client_pairs]
fable5 = cc-mic            # fable5 may run on cc-mic
opus55x = cc-mic, cc-p
```

The key is a profile, the value the clients it may run on, comma-separated. Only the global file
is read: a project `.ae/config` cannot approve a pair or redefine an approved name. A row ae
cannot read in full approves nothing — an empty list, an empty entry (`a,,b`), a name that is not
a client label — and a profile keyed twice approves nothing rather than letting the later row
win. An entry naming a profile or client that does not exist is inert. Remove a line to retire a
route for new spawns; a seat already recorded on it keeps resuming. The `spawn_instructions`
text is the place to tell your agents which spellings you allow: written policy is never turned
into a permission, and an unlisted pair is refused with where to approve it and the clients that
profile is approved for. See [Spawn on another account](../reference/helpers.md#spawn-on-another-account).

A client row also takes `manual_resets=<0-9>`, in any order beside `config_home=`:

```toml
[clients]
codex = codex manual_resets=1
codex-mic = codex config_home=${HOME}/.codex-mic manual_resets=0
```

It is the one fact no client reports: how many manual window resets that subscription has in hand.
`ae quota` spreads the window percentage over `1 + n` windows in its `EFFECTIVE` column, and the
watchdog advisory and delegation guidance judge that number instead of the raw window, so a 95%
window with one reset declared no longer pushes work off a client that is not constrained. An
unusable value is ignored with one visible note rather than refusing the config.

The count is yours to maintain: ae does not decrement it when a reset is used, so update the row
after each manual reset. A positive count is a claim about headroom that only holds while the
declaration is current; `0` declares that there is none.

Each distinct config home isolates local login state, settings, and conversation files, so one
workspace can mix work and personal identities seat by seat. It does not create an independent
provider quota: two homes may authenticate the same account, while two client labels may share one
home. `ae quota` groups profiles by the resolved client home and lists the client labels sharing
each scope. A client no profile names is read as its own scope, so its declaration is never dead
config — the row is there before you launch anything against it. Bind the profiles to different `[roster]` names and run the tool's login flow once per
new home.

**Claude default-state trap:** never set `CLAUDE_CONFIG_DIR` (directly or through `config_home`) to
the default `$HOME/.claude` directory. Without the variable, Claude Code reads its account state
from `$HOME/.claude.json`; with the variable set, it reads
`$CLAUDE_CONFIG_DIR/.claude.json` instead. The default client must stay `claude = claude` with no
`config_home`, or the same-looking path selects a different state file and can appear logged out.

The config home becomes seat identity on first start: ae records its canonical path and whether the
tool-specific variable was explicit or the default was derived from an unset variable. It uses that
recorded value and mode for resume probes, execution, Codex session-id capture, and optional history
purge. For a default-derived store, ae also records the effective `HOME`; this keeps the tool's login
and state files aligned even when the default store is a symlink. Changing `cc-mic` from directory A
to B while a session is retained therefore prints:

```text
ae: seat <slot>: config now points claude at <B>; the retained conversation lives in <A>, resuming there — end the session to adopt <B>
```

Stopping and resuming keeps A. End that session and start a new one to adopt B. A client entry
also avoids shell functions and renamed wrapper binaries: ae expands the client label to the real
executable before it classifies and launches the tool.

## `[prices]`

Override an exact model's API reference price without extending the INI grammar:

```toml
[prices]
sol_discount = gpt-5.6-sol,1.25,2.5,0.125,10
```

The key is an arbitrary config-safe alias because model ids may contain dots. The value is
`model,input,cache_write,cache_read,output`; rates are USD per one million tokens with at most six
decimal places. Project rows overlay global rows by alias. If two surviving aliases name the same
model, `ae usage` refuses with exit 2 and names both aliases rather than choosing one silently.

An override wins over the bundled exact model row. Bundled rows also match an exact `-YYYYMMDD`
release suffix; broader prefixes do not match. These are API-equivalent reference prices for the
base service tier, base context window and five-minute cache writes. Long-context surcharges and
priority or flex tiers are not modelled. Subscription access incurs no additional charge from an
`ae usage` report: ae reads local transcripts offline and is not a billing system.

## `[roster]`

Bind names to profiles: `name = profile`. The NAME is the agent's identity — it's
what you address with `send`/`ask`/`spawn`, and what pane titles, borders, and
`ae list` show; the profile is metadata (`ae list` shows it alongside the name),
and the same profile can back more than one name. Every seat in
`[workspace] main`/`workers` must be bound here — ae refuses the launch
otherwise and lists every violation. A name bound here but not seated is legal:
`ae <session> use <name>` starts it as main instead of the configured one. Spawn
on demand with `spawn <name> --using <profile>`. The optional orchestrator seat
also uses this global roster: add `orchestrator = <profile>` before running
`ae orchestrator`; its dedicated local overlay no longer chooses the profile.
Old seat files that still carry `[profiles]`/`[roster]` are ignored for identity;
`[workspace]` and `[prompt]` still overlay.

## `[workspace]`

| Key       | Description                                          | Default       |
|-----------|------------------------------------------------------|---------------|
| `main`    | `[roster]` name for the standing main seat. Under `lead-pair` this is a *technical* lifecycle anchor (reboot handover, non-retirable), not a rank | `lead` |
| `workers` | Comma-separated `[roster]` names launched at startup. Under `lead-pair` the FIRST worker is the colead seat — an equal leadership peer of the lead (interchangeable, same level). Recommended default: the colead ONLY — builders/reviewers are spawned on demand per slice and retired when done | `colead` |
| `layout`  | `lead-pair` (lead and colead each get 50% of the leads' window, other workers share the next one), `lead-solo` (lead alone in its window, workers in the next one), `vertical` (side-by-side splits), `horizontal` (stacked splits) | `lead-pair`   |
| `copy`    | Working directory mode (see below)                   | `local`       |
| `watchdog`    | Auto-start the watchdog (`true` / `false`)            | `true`        |
| `quota` | Whether ae acts on vendor quota at all (`on` / `off`); absent means `on`. When `off`, agents are never told about quota, the watchdog books no quota advisory, sends no checkpoint ask and renders no quota throttle line, and the settings menu carries no quota entry. While `on`, a scope entering `low` or worse asks every seat on it, once, to write a durable checkpoint before its subscription runs dry. `ae quota` works identically in both states. `off` wins over `quota_every_secs`; the auto reseat chooser then reads no vendor numbers and moves in declared order | `on` |
| `quota_every_secs` | Watchdog cadence in seconds for the quota observation, rounded to whole watchdog cycles (`0` disables it) | `300` |
| `idle_nudge_secs` | Continuous positively observed empty-input time before the watchdog reminds the seat (`0` disables) | `300` |
| `done_confirmations` | Delivered proof challenges a later `done`/`waiting-agent`/`blocked` must answer (`waiting-user` never challenged; a wait proof with no news since is next challenged after two `idle_nudge_secs` periods; `0` disables; range `0`–`9`) | `2` |
| `auto_reseat` | Which seats the watchdog may move off a vendor usage limit (`off` / `on` / `all`); absent means `off`. `on` moves fixed non-main and spawned seats, `all` moves the main seat too; an orchestrator session's seats never move. Global config only | `off` |
| `auto_reseat_sessions` | Comma-separated sessions auto reseat acts in; absent means every session. An entry that is not a session name, or is named twice, is ignored with a note | every session |
| `auto_reseat_grace_secs` | Whole seconds a usage-limit or headroom episode waits before its seat may move. `0` is legal: the first cycle past the latch is due, the client-input hold retires, and a Draft or Busy frame still holds | `600` |
| `auto_reseat_at` | The judged percentage of a seat's OWN account at which auto reseat moves it before any limit (`50`-`100`, or `off`). Only a usable window proven at least one day long counts, the weekly one, never the 5-hour session window, judged as `ae quota`'s `EFFECTIVE` column. An unusable value means `95`, with one note the watchdog journals once per start. Global config only | `95` |
| `auto_reseat_lead_at` | The headroom threshold of the lead pair alone: `main` and `worker.0` of a `layout = lead-pair` session (`50`-`100`, or `off`), in place of `auto_reseat_at` for those two seats. Every other seat, the main seat of any other layout and the orchestrator keep `auto_reseat_at`. Absent means the lead pair uses `auto_reseat_at`, exactly as without the row; an unusable value (out of range, written twice or unreadable) does too, with one note the watchdog journals once per start. `off` means headroom never moves a lead seat; a drawn vendor usage limit or an urgent start still moves it, and a seat so moved can still return home once its origin reads under `off`'s band of 90. The value may sit below `auto_reseat_at` and works with `auto_reseat_at = off`. It changes the bar, not who may move: `auto_reseat = on` still leaves the main seat alone. A threshold set or raised does not close a headroom episode already open (it stays due while the account reads within 5 points of the threshold). Needs `auto_reseat` on. Global config only | `auto_reseat_at` |
| `auto_reseat_lead_floor` | A capability floor for the lead pair alone: `main` and `worker.0` of a `layout = lead-pair` session. A whole number `0`-`255`, digits only. Absent means no floor and the part-A rule stands: a lead never takes a derived sibling. With a floor set, an automatic move of a lead seat that CHANGES the model (a derived sibling, a row you declared under `[auto_reseat]`, or the return route) needs the candidate's tier under `[model_tiers]` to be at least the floor; the pass bar replaces the blanket sibling bar. A candidate with no tier, an unreadable model, tiers that disagree, or a floor ae cannot use (not a whole number, empty, written twice, out of range) is refused with the model and the missing piece named, so a typo never relaxes the floor. A derived twin, a declared row on the seat's own model and a move back to the profile a proven chain left first need no tier, but a twin or a same-model row only while its model pin is readable: an unpinned twin is judged and refused as `model unknown`. Every other seat and a manual `ae reseat --using` ignore the floor. Needs `auto_reseat` on. Global config only | none |
| `auto_effort` | `on` lets the watchdog step a lead-pair seat's reasoning effort down ONE level, live, once a usable window of its own account at least a day long judges `80`% (`ae quota`'s `EFFECTIVE`), and back up once every window that moved it reads under `75`% again or has passed its recorded reset, while no long window of that account judges `80`%: an in-place restart that keeps the pane, profile, account, model and conversation. Only the main seat and the colead, only claude and codex; never a spawned seat or the orchestrator. The target is always the profile's own effort: another effort is `ae reseat <session> <agent> --using <profile>`, and a change made in the pane yields the episode and is never restored over. While headroom auto reseat is on with the threshold that applies to that seat (`auto_reseat_lead_at` for the lead pair when set, else `auto_reseat_at`) at most `80`, no new step down starts for that seat (one note per watchdog start, naming the row). `off` or absent changes nothing, and a resume then drops a recorded move. Global config only | `off` |
| `opencode_go_key_file` | Path of a file holding an OpenCode Console key with usage access, separate from the key opencode uses for model calls (`~/`, `$HOME/` or absolute), which lets `ae quota` show the Go subscription's windows. Absent means ae reads no Go quota; see [OpenCode Go quota](#opencode-go-quota). Global config only | none |
| `copilot_token_file` | Path of a private file holding your GitHub token (`~/`, `$HOME/` or absolute). Opts into the display-only premium allowance; see [GitHub Copilot quota](#github-copilot-quota). Global config only | none |
| `muse_quota` | `on` lets `ae quota` ask Muse for its allowance with one model call (about 32K input tokens), held for 30 minutes; `off` or absent asks nothing; see [Muse quota](#muse-quota). Global config only | `off` |
| `orchestrator` | Mark this session as the fleet overview seat (`true`); grants its panes the `relay` helper into other sessions' chats | `false`       |
| `sweep` | Persist this orchestrator's changed-overview minimum spacing in seconds (`0` disables; positive values below `60` become `60`) | `AE_WATCHDOG_SWEEP_SEC`, then `120` |
| `auto_upgrade` | Let an installed ae quietly check for and apply strictly newer releases (`on` / `off`); global config only | `on` |
| `fleet_order` | The order your sessions are drawn in on the fleet strip, and the sessions pinned to the top of the app's sidebar, as a comma-separated list of session names; global config only | creation order |
| `restore` | Let bare `ae` resume the sessions a crashed tmux server held (`on` / `off`); global config only | `on` |
| `auto_trust` | Answer claude's and agy's folder-trust modal for a seat's OWN recorded folder, once per tool start, and dismiss codex's Warnings panel with one Escape per panel episode (`on` / `off`); global config only | `on` |
| `palette` | `darcula` (the JetBrains dark), `a` (neutral dark), `b` (warmer neutrals) | `darcula` |
| `icons`   | `off` draws the ASCII fallback instead of the glyph set | `on`          |
| `theme`   | `off` leaves your own status line, pane borders and menu styles alone | `on`  |
| `motion`  | `off` freezes the working `●` at its accent colour   | `on`          |
| `chat`    | `off` launches without the [chat](../chat.md) as the session's first window; `app` runs [`ae app`](../app.md) in that window instead; any value other than `on` / `off` / `app` opens the chat and prints one note. The orchestrator's own session never gets one | `on` |

Set `orchestrator = true` only in the dedicated overview seat. It authorizes
`relay`, whose chat ask the target seat reads as the human's (`human:chat`).

`fleet_order` is how you choose where a session sits on the second status
line, instead of taking the order tmux happened to create the sessions in —
which reshuffles every time you restart the fleet:

```toml
[workspace]
fleet_order = aedev, thinking, infra
```

The sessions you name come first, in the order you named them; everything else
falls in behind them in creation order, as before. The orchestrator stays
pinned at the front either way. Naming a session that is not running right now
costs nothing — that is the point, so the fleet comes back the same way after a
restart. The match is exact and case-sensitive.

This is machine policy like `auto_upgrade`, so ae reads it only from
`~/.ae/config` and a project's `.ae/config` cannot reorder your fleet. Edit it
while sessions are running: each session's watchdog re-reads the file and
redraws its strip within one cycle, so nothing needs relaunching. An entry that
is not a legal session name, one you listed twice, or one matching no session ae
has a record of is skipped silently on the bar — `ae doctor` names it once on
the `workspace.fleet_order` row. With no `fleet_order` set, the strip is exactly
what it always was.

In [`ae app`](../app.md) the same list pins: the sidebar puts the orchestrator
first, then the sessions you named, then every other running session by when
you last used it, newest first.

The same order decides two more things: which session a client is handed to when
the one it is watching is killed, and how the running rows of the fleet picker
(`<prefix> a`) are sorted underneath attention — the picker still puts whatever
needs you first, and an orchestrator you named there is simply first among rows
of equal attention rather than pinned as it is on the strip.

`auto_upgrade` is machine policy, so ae reads it only from `~/.ae/config`;
a project's `.ae/config` cannot override it. Absence means `on`. Any explicit
value other than `on` or `off`, including an empty value, is invalid and leaves
automatic upgrades disabled until corrected. Checkout builds never
auto-upgrade, and `AE_NO_AUTOSTART=1` suppresses scheduling along with the other
companions. `ae version` and `ae doctor` report policy plus last check/result;
they never trigger a check.

### GitHub Copilot quota

Copilot quota is opt-in. Choose the GitHub account that holds your Copilot subscription and
create a file yourself, for example using GitHub CLI (replace `<account>`):

```sh
mkdir -p ~/.credentials
umask 077
gh auth token --user <account> > ~/.credentials/copilot
chmod 600 ~/.credentials/copilot
```

Name it in the GLOBAL config; a project `.ae/config` entry never votes:

```toml
[workspace]
copilot_token_file = ~/.credentials/copilot
```

ae never runs `gh` or reads another tool's authentication store. The file must be regular,
private (no group/other permissions), nonempty printable ASCII, at most 4 KiB. Its contents
go only into the authorization header, never argv, output or the shared cache's key identity.

`ae quota` asks `GET https://api.github.com/copilot_internal/user`. This is GitHub's internal,
undocumented, unversioned endpoint and may break. Light polling reads your own quota; it
invokes no model and spends 0 premium requests. One shared private cache
(`quota/github-copilot.json`) and zero-wait lock limit the fleet to one attempt per 300 seconds,
including failures. Token rotation makes the next attempt due; a failed refresh retains the
last good answer with its original age and a named failure note. `quota = off` prevents fetches.

Profiles running OpenCode with `--model` or `-m github-copilot/<model>` share one group.
With none configured, the group still appears with `-` in PROFILES. Only the closed plan names
`individual`, `business` and `enterprise` are displayed; anything else says `plan unknown`.
USED is `(entitlement - remaining) / entitlement * 100` when both counts are valid integers;
only when both are absent does it use `100 - percent_remaining`. Partial or invalid counts,
missing fields and unknown shapes yield notes, never invented numbers. `unlimited: true`
reads `unlimited` in CREDITS. RESETS comes from `quota_reset_date_utc` (an RFC 3339 instant), else from
`quota_reset_date` (an RFC 3339 instant or a bare `YYYY-MM-DD`, read as 00:00:00Z of that day). The `_utc`
field wins when both parse, and a date the calendar lacks counts as unreadable. The monthly WINDOW has
no fixed length and supplies no PACE history or weekly quota-ladder input.

Unlike Go, Copilot has no watchdog refresh or advisory feed. The settings quota dialog reads
the held snapshot, shows its age and tells you to run `ae quota` to refresh. Copilot quota
does not drive automatic reseating or effort changes. A rejected token names its HTTP code
and `copilot_token_file`; transport, timeout, redirect, size and parse failures are named.

### OpenCode Go quota

OpenCode stores no quota where ae can read it, so Go quota is opt-in. The key is an OpenCode
Console key with usage access, separate from the key opencode uses for model calls. Put it in a
file only you can read, and name that file in the GLOBAL config (`~/`, `$HOME/`, `${HOME}/` or an
absolute path; a project `.ae/config` entry is ignored):

```toml
[workspace]
opencode_go_key_file = ~/.config/opencode/go-usage.key
```

```sh
chmod 600 ~/.config/opencode/go-usage.key
```

ae never reads opencode's own `auth.json` or database, never an environment variable for the
key, and never writes the key anywhere: not to argv, a journal, a log line, an error or the
cache. A key file that is not a regular file, is readable by group or others, is empty, holds
non-printable bytes or is longer than 4 KiB is refused with a note naming the file, never its
contents.

Only profiles whose model is `opencode-go/<model>` (`--model` or `-m`) get Go windows. Every
other OpenCode profile, OpenRouter included, stays `unsupported`. `ae quota` shows three
windows: `rolling` (5 hours), `weekly` (7 days) and `monthly`, whose `WINDOW` cell is `-`
because a month has no fixed length; `EFFECTIVE` and `PACE` come from the same derivation as
every other scope. A window the vendor marks `rate-limited` reads at least 100%.

ae asks `GET https://opencode.ai/zen/go/v1/usage` at most once per 300 s across the whole fleet:
the answer goes into a shared `0600` cache under the state root (`quota/opencode-go.json`), and
every reader — `ae quota`, the watchdog, the settings dialog — reads that cache. `ae quota` may
refresh it inline; the watchdog refreshes it on a background thread, one at a time, only for a
quota-aware session. With `[workspace] quota = off` nothing asks the network at all; a cache
already there is still shown. Rotating the key file makes the next refresh due at once, and
answers kept for the old key are dropped. A failed refresh keeps the last good answer and adds
one note: a 401 names the key file (or, when the vendor's answer carries the code
`api_key_expired`, says the key expired and to renew it in the OpenCode Console), a 403 says the
key has no Go subscription, a 429 says the vendor throttled the call, and a timeout, a redirect,
an oversized or unreadable answer are each named.

The endpoint is undocumented: if OpenCode changes its shape, the windows go away with a note
rather than showing a wrong number. Named gap: an OpenCode seat's quota scope is not joined to
the seat (`quota::recorded_identity` reads none for opencode), so the per-seat checkpoint ask
and throttle line do not cover it; the lead-pair advisories still fire.

### Muse quota

Muse keeps no allowance anywhere ae can read for free: its session logs omit it and the one
place it is reported is the usage record of a model call. So Muse quota is opt-in and costs
tokens. Put the switch in the GLOBAL config (a project `.ae/config` entry is ignored, so a
checkout cannot spend your account):

```toml
[workspace]
muse_quota = on
```

Only `ae quota` ever asks. It runs `muse exec --json --preset empty --reasoning-effort minimal
--no-session-log --no-foreign-personal-context` with a one-line prompt, as the account of the
caller's `HOME`, in a fresh empty private directory under the temp directory (`ae-muse-cwd.<pid>.<n>`,
removed when the call returns; a crash can leave one behind and nothing sweeps it: muse refuses to
start in a directory that holds `TMPDIR`), with a 90 s deadline and a 256 KiB output cap;
a call is about 32K input tokens and takes 13 to 60 s. A call killed at the deadline leaves muse's
own `tbh-process-lifetime-memory-*` directory in `TMPDIR`; ae does not delete vendor directories. The settings quota dialogs never ask: they show the
held answer and say `muse quota not asked yet — run ae quota` until one exists. The watchdog,
the advisories and auto reseat neither ask nor read it, so Muse raises no advisory (a known gap,
as for agy). `ae quota` itself asks nothing with `[workspace] quota = off`, with no `muse` on
`PATH` (named, and not held), or on a state root it cannot write to.

The answer, a failed or timed-out one included (those cost tokens too, and a failure names its
class: `exited N`, `output not parsed`, `output not UTF-8` or `output over 256 KiB`), is held for 30 minutes
in `quota/muse.json` under the state root, shared by every ae process: a second `ae quota`
shows the held numbers with the vendor's own age, so they read `stale` after 15 minutes, and
only the process that wins the zero-wait lock `quota/.muse.lock` asks. Delete `muse.json` to
force a new ask. Any other value than exactly `on` or `off` is ignored with a note.

Rows are the shared account bucket: a `window` row, whose length the vendor does not state
(so `WINDOW` is `-`), and a `weekly` one (7 days), each with its used percentage and reset;
the subscription tier, when the vendor names one, labels them. `EFFECTIVE` and `PACE` come
from the same derivation as every other scope, so a `manual_resets` declaration on the Muse
`[clients]` label applies. Every Muse profile that keeps the caller's `HOME` is one scope.
Named gap: if Muse changes the record's shape the rows go away with a note rather than
showing a wrong number.

### Restoring after a tmux crash

Outside tmux, bare `ae` first resumes the sessions a crashed tmux server held,
then opens the app on a terminal, else attaches (`ae --no-attach` prints the
attach command instead). A session
qualifies when it has not been stopped through `ae` since its latest launch, had
a watchdog running, its recorded server is proven gone, and its watchdog beat is
within 15 minutes of the newest beat among the qualifying candidates on that
same recorded server. Each candidate is re-checked under its own lifecycle
lock, so a concurrent `ae <name>`, `ae stop` or second bare `ae` leaves exactly
one incarnation. Sessions still live are left alone; a candidate skipped during
a restore run is named on stderr as `ae: skipped <name>: <reason>`, and a failed
attempt as `ae: restore failed <name>:`.

`auto_trust` answers TWO prompts. The first is the measured folder-trust modal of claude or
agy, drawn whole, naming exactly the folder the seat was started in (symlinks
resolved). It presses at most one arrow onto "Yes, I trust this folder" and one
Enter per tool start, inside a `spawn` brief or launch-turn wait, else through
the session's watchdog. Every answer is journaled (`auto-trust` and its
`auto-trust-*` steps) and said once on the chat. Anything else — another prompt,
another folder, a damaged journal — is named as before and left to you.
The second is codex's Warnings panel, which replaces its input box (F2 opens it,
and so does a click on codex's `⚠ N warning · f2 to view` footer): once it has
been named for 60 s with no input from you on that pane, ae presses one Escape
per panel episode, journaled as `auto-dismiss` and its `auto-dismiss-*` steps.
An unmeasured panel body is only named.
Absence and `on` answer, the exact lowercase `off` does not, and any other value
answers nothing, said once by the watchdog. A claude or agy launch with no
watchdog pastes no turn, so nothing answers its modal.

`restore` is machine policy like `auto_upgrade`: ae reads it only from
`~/.ae/config`. Only the exact lowercase `off` disables it; absence and `on`
restore silently. Any other value (`OFF`, `no`, an empty value) or an unreadable
line still restores and says so once per run, beginning `ae: restore: `.

Known limits: a session killed by hand with `tmux kill-session` looks like a
crash; a lone old unstopped session is its own cohort; a recorded server whose
state ae cannot prove is reported and skipped on every bare run until it is;
when a server gives no session listing, each candidate on it is proven by its
own tmux call; a saved record whose metadata is unreadable, or whose recorded
server selector is missing or ambiguous, before the launch's preflight can count
as a failed attempt (exit 1, nothing written) rather than a skip.

### Auto reseat

When a seat is proven stuck on its vendor usage limit, the watchdog can move it
in place to another profile — the same move `ae reseat` makes by hand, decided
from the journal and the live quota readings. The five knobs above are
global-config only: a project overlay never steers spend, and the switch is
read on every decision, so turning it off stops the next move. An
`auto_reseat`, `auto_reseat_sessions` or `auto_reseat_grace_secs` value ae
cannot use turns the whole path off with its one note, beginning
`auto reseat stays off: `. An unusable `auto_reseat_at` does NOT: the threshold
falls back to `95`, the watchdog journals that once per start as an
`auto-reseat-notice`, and limit and headroom moves both go on. An unusable
`auto_reseat_lead_at` does not either: the lead pair uses `auto_reseat_at`, with
its own note (`...; auto_reseat_at used`).

A seat can also move BEFORE its limit, to keep headroom in its subscription:
once a usable window of the seat's own account, proven at least one day long,
judges at or past `auto_reseat_at`, a headroom episode opens, and after the grace the seat moves
at the first cycle that proves its frame IDLE — a running turn, a draft, a
human's input, a prompt only the human may answer, or a frame ae cannot model
holds it, and a hold spends no attempt. The episode stays due while the account
reads within 5 points of the threshold, and a new one can open only after it has
read strictly more than 5 points below it, so jitter on the line moves nothing
twice. A headroom move takes only a candidate with room: one with a usable
long window, or a critical one, at or past the threshold is passed over, and a profile the seat left on
headroom waits for a reading after the move that proves it below the threshold,
or a passed reset. A seat on its vendor limit is the limit path's alone, which
moves as it always did. With `quota = off`, or a seat whose account ae cannot
read, nothing opens.

The `[auto_reseat]` map names where each profile may move, in order:

```toml
[auto_reseat]
sol6x = opus55x-mic, opus55x, spark13cm
```

The key is the seat's current profile; the value is its candidates,
comma-separated, best first. An entry ae cannot use is ignored with a note of
its own: a key that is not a profile name, a key with no candidate list, a
candidate that is not a profile name, the profile itself, or a repeat; a
profile left with no usable candidate is ignored too. A profile keyed twice
keeps its later row.

Every listed seat also gets the profiles of its OWN family, after its row — and
with no usable row, those alone. A TWIN is the seat's setup on another account:
the same command word for word, the account variable aside, on a config home ae
can prove is a different one. A SIBLING is the same tool and flags pinned to
another model, on any account. The family is ordered least disruptive first
(#252): a KEPT candidate (same account, a command that differs only in the model
or effort the tool's resume is measured to take) can continue the conversation;
then one on another proven account (claude carries the conversation, other
tools seed); then the rest, which seed. Inside a step twins come before
siblings, each in `[profiles]` order. A declared candidate always comes before a
derived one, even when its account is worse off — to prefer the family, change
the row. Among derived ones a candidate at critical comes after every other,
then the step decides, then the better-off account. A profile whose model routes
through the tool's separately billed plan (today `opencode-go/`) is never
derived; name it in the row to allow it. The seat's own profile, a profile listed already,
and one that runs the same setup on a store ae proves is the same as one listed
before it add nothing; an account ae cannot name is never proof of either, and
on a tool with no account variable only a byte-identical command is the same
setup. Another tool, another spelling of the executable, a reordered argv,
another effort or permission, or a model flag ae cannot read is not family: name
such a profile in the row. Records and notices spell a derived pick `<profile>,
derived twin` or `<profile>, derived sibling`; a declared one reads as it always
did. The family is read from the seat's RECORDED profile and client, so a model
the human switched to in the pane does not travel, and a seat launched on
another client of its profile is never moved back to that profile.

A lead-pair seat — `main` and `worker.0` of a `layout = lead-pair` session —
never takes a derived SIBLING: auto reseat does not swap the model of a lead on
its own, whatever the sibling's account or step. Its siblings stay on the list
and are passed over with `a lead seat never changes model unasked`; a move that
finds nothing else refuses with `no usable candidate` and names each sibling and
that reason, and the seat stays where it is. A declared candidate (your row) and
a derived twin — the same setup and model on another proven account — still move
a lead as they move any seat while no floor is set (with a floor, see below: a
twin needs a readable model pin), so to let a lead fall back to a weaker or
another model, name that profile under `[auto_reseat]`. Every other seat is unchanged,
and so is a manual `ae reseat --using`.

**The lead floor.** Declared rows are the human's own choice, but a row can
still put a lead on a model too weak for the work, and a sibling is useful only
when it is strong enough. `auto_reseat_lead_floor = <n>` together with
`[model_tiers]` (below) lets you say how strong: with a floor set, every
automatic move of a lead seat that would CHANGE the model must reach it. The
rule covers a derived sibling, a row you declared under `[auto_reseat]` and the
return route, and it replaces the blanket sibling bar above: a sibling whose
model is at or over the floor is available again. A candidate passes when its
tier is at least the floor. Not covered, and never refused by the floor:

- a derived TWIN whose model pin is READ (the same setup and the same model, so
  nothing is lost). An unpinned twin takes the tool's default model, which can
  differ per account plan, so it is judged like any candidate and refused as
  `model unknown` (two unread pins are never "equal");
- a declared candidate that runs the seat's own model on another profile, when
  both models are read and equal; a model ae cannot read on either side is judged;
- the move back to the profile a proven chain of automatic moves left first (the
  recorded origin). With no chain, or one ae cannot prove, nothing is exempt, and
  the pair guard still applies;
- every other seat, and a manual `ae reseat --using`, whatever the tiers say.

A floor ae cannot use refuses every other candidate of a lead, so these
exemptions are the only moves left until you fix it.

The floor fails closed. A candidate is refused, with the model named, when:

| Case | The record says |
|---|---|
| its tier is under the floor | `<profile>, derived sibling, model <m> (tier 2 is below the lead floor 3)` |
| no `[model_tiers]` row names its model | `<profile>, declared in [auto_reseat] <seat profile>, model <m> (no tier for its model; set one under [model_tiers])` |
| its profile pins no model ae can read | `<profile>, model unknown (its model cannot be read, so it has no tier)` |
| rows equally specific for its model disagree | `<profile>, model <m>, [model_tiers] one = 1, two = 3 (its model matches tier rows that disagree)` |
| the floor is written and unusable | `<profile>, model <m>, auto_reseat_lead_floor = "high" (the lead floor is unusable)` |

A refused move ends as `no usable candidate` in the class you already know, the
seat stays where it is, and nothing is stopped. Tier every model your lead
profiles pin, plus every model a row of yours may reach: the floor never guesses
a tier, so an untiered model is never chosen for a lead. The same floor is
checked once more after the choice, just before the seat is stopped: if you
raise it or fix a tier while a move is being prepared, the move refuses with
class `floor-changed` and `nothing was stopped`, and the next cycle judges again.
A project overlay never relaxes the floor: both the key and `[model_tiers]` are
global config only.

**Is every profile covered?** A profile whose declared row and derived family
all run on ONE account has no exit when that account's window is spent: its
seats just sit on the limit. With `auto_reseat` on, `ae doctor` prints one
`reseat-cover` row — `OK` when every profile has a candidate off its own account,
otherwise one `WARN` per account naming the uncovered profiles and the exact
line to add under `[auto_reseat]` (profiles ae cannot place on an account get a
row of their own, so one account can show two rows). `ae spawn` and `ae reseat --using` print the
same warning as ONE stderr line when the profile you pick has no such exit, and
go on. With `auto_reseat = off` the row reads `skipped (auto_reseat off)` and
nothing warns. A model of the same tool is a sibling but not an exit: it shares
the account's allowance. A profile ae cannot place on an account is reported as
unproven, never as covered. The row and the warnings read a profile, not a seat:
with no lead floor set, a lead-pair seat cannot use a derived sibling, so a
profile whose only exit is a sibling on another account reads as covered here but
not for the lead pair. With a floor set a sibling at or over it is usable again,
but the row does not judge tiers per seat, so it can still over-promise for a
lead whose sibling is under the floor.

`ae doctor` also prints a `reseat-floor` row: `OK` with the floor and the number of tier rows (or `no lead floor set`), or one `WARN` per unusable floor value, ignored `[model_tiers]` row or conflicting rows; it reads `skipped (auto_reseat off)` with the switch off.

Names show in pane borders and are how agents address each other. Each window
keeps its first agent's name as its stable tmux routing name; later splits do
not rename it.

The status bar has two lines. The first is this session: windows led by their
live marks and named agents, then the branch, goal,
shortened path and watch segment. One agent is `0:✓lead`; multiple agents are
`0:[✓lead ●colead]`. The selected window and current fleet row use the
palette's selection colours.
The second is the **fleet strip** — every ae session on this tmux server in the order it
was created, each with its live mark and clickable to switch to it. Every pane
also carries a border title: `<name> · <mark> <reason>`.

Right-click a session tab in the fleet strip to open its context menu. **Flip
lead/colead panes** swaps the clicked session's current window when it has exactly
two panes and is not zoomed; otherwise ae leaves it alone and explains why. The
tmux keys `prefix {` and `prefix }` provide the same pane swap directly.
Click the quiet `≡` menu glyph (`=` with `icons = off`) or the matching `+N`
overflow count with either mouse button, or press `prefix a`, to open the fleet
picker. Its title starts with `ae session`, then counts
running sessions and those needing you; up to 30
attention-ordered rows show name, mark, state, branch and goal. Choosing a row
drops the invoking client into that session's lead pane. On tmux 3.4 use the
row shortcut keys; tmux 3.5 and newer also supports mouse selection.

The bottom-right three-cell range (` ⚙ `, or ` * ` with `icons = off`) opens
settings against that corner of the client that clicked it. Both surrounding
spaces are clickable and highlight with the glyph. The range is always
version-free; the menu title alone shows the running `ae` version when the
session reports a valid one. While the menu is open, selection colours
highlight this button only. The settings menu shows one live quota entry
opening a centred per-window dialog (absent entirely when `[workspace] quota = off`),
then the recorded orchestrator role and
exactly one of Start, Resume or Pause when that action is safe; otherwise it
shows why the control is unavailable. Very small clients may show a quota
overflow notice instead of the entry; `ae quota` remains the full view.

The marks are the **watchdog's verdict**, never a claim about what an agent is "doing"
(it cannot see that):

| Mark | ASCII | Means |
|---|---|---|
| `✖` | `x` | the process behind the pane is gone |
| `⚠` | `!` | waiting on you, blocked, on its usage limit, or throttled |
| `●` | `*` | working according to the latest liveness verdict |
| `◔` | `~` | waiting on another agent — quiet, no human needed |
| `✓` | `+` | declared done or paused |
| `◌` | `?` | stale, or a fact ae could not establish |
| `·` | `-` | positively idle at an empty modeled input, no agent, or no verdict yet |

While somebody is attached, a Working verdict shows a pulsing `●` in its place.
The pulse is the cached verdict, not terminal motion; silence past the liveness
window changes the next watchdog verdict to stale. A positively recognized
empty Claude Code or Codex input is idle immediately and does not pulse.

Session attention is keyed by the agents in session meta, so an agent whose
pane vanished still holds its slot as `⚠` rather than quietly disappearing.
Live agents and their marks appear in the window list, so attention maps onto
the windows you already scan. Everything is watchdog-published and disappears
when the watchdog is stopped.

> **ae draws its own sessions, at session and window scope.** It sets `status-format[0]`
> and `[1]` on its sessions, and the pane-border and menu styles on their windows. Your
> global tmux config is untouched, and nothing outside an ae session changes. tmux's `Z`
> zoom flag is kept in the window list; the `*` and `-` flags are replaced by
> named agents and marks.
> `theme = off` turns all of that off and still publishes every `@ae_*` value, so your own
> `status-right` can read ae's facts in your own layout.

### `[model_tiers]`: what each model is worth to a lead

One row per model, `alias = <model>, <tier>`. The tier is a whole number from `0`
to `255`, and a higher number is a stronger model. Only the lead floor
(`auto_reseat_lead_floor`) reads it, and only for the lead pair. Tiers belong to
MODELS, not to profiles, so every profile that pins a model reads the same row.

```ini
[workspace]
auto_reseat_lead_floor = 3

[model_tiers]
large = opus, 4
mid   = claude-sonnet-5-5, 3
small = haiku, 1
```

- The alias is a config key (letters, digits, `-` and `_`, starting with a letter or `_`). It only names the
  row, because a model id can hold dots and slashes that a key cannot. The model
  runs to the LAST comma, the tier follows it.
- A row matches a profile's `--model` pin the way the tool matches a model: for
  Claude `opus` covers every Opus version, `claude-opus-5-5` only that version.
  When rows that name a version match, only they decide, so a family row never
  admits a model that its version rows disagree on.
- A later row with the same alias replaces the earlier one. Rows for one model
  that give different tiers all stay, and a lead refuses that model by name: the
  disagreement is never settled by order.
- A row ae cannot read (no comma, an unusable model, a tier that is not a whole
  number) is ignored with a note. `ae doctor` prints it as a `reseat-floor`
  WARN, and the watchdog journals an unusable floor once per start.
- The section is global config only, like the floor key.

## Copy modes

How agents access your code:

| Mode | Flag | What it does |
|------|------|------|
| `local` | *(default)* | Agents work directly in your project directory. Simple and fast. |
| `full` | `--copy` | Full copy of the project. Use for complex features where agents need an isolated workspace. |
| `worktree` | `--worktree` | Git worktree. Lightweight branch isolation backed by git. |

## `[prompt]`

Custom instructions transported beside the ae workspace context through each harness's supported launch channel. Per-project `.ae/config` overrides the global one.

```toml
[prompt]
instructions = "Always cite the source file you used."
```

A long policy is easier as a multi-line block. The opener line is exactly
`instructions = """`; a line that is exactly `"""` closes the block. Between them
every line is raw text — a `[section]` header, a `key = value` line and a `#`
comment are all just text, and no escapes are interpreted. A raw line that is
exactly `"""` cannot appear inside the block, because that line closes it; no
escape exists. The single-line form above stays valid and unchanged, and the
block is accepted for `prompt.instructions` and `prompt.spawn_instructions`
alone.

```toml
[prompt]
instructions = """
SPEND POLICY: keep replies short.
Cite the file for every claim.
"""
```

Every harness receives the value on its own channel — a system-prompt flag, a
developer-instructions value, an initial user turn, or a generated config file —
so the newlines and quotes reach the agent's system prompt verbatim.

### Text only a seat that may spawn needs

`instructions` lands whole on every seat. A long policy that only a seat choosing
profiles needs, such as spend and profile rules, goes in `spawn_instructions`
instead; it takes the same single-line and `"""` block forms.

```toml
[prompt]
instructions = "Cite the file for every claim."
spawn_instructions = """
SPEND POLICY: chores go to the cheap profile.
"""
```

- The lead pair (`main`, and `worker.0` of a lead-pair layout) and a seat whose slot ae
  cannot classify get the whole text, after `instructions`.
- Every other seat gets one line instead, naming the `## Spawn instructions` section of
  the session's `workspace.md`, which holds the same text. A worker that may spawn reads
  that section before it does.
- The two keys resolve apart. Each follows the same layering: the last config file that
  declares the key wins, even when empty, so a local `spawn_instructions = ""` masks a
  global text.
- `instructions` keeps its meaning: with no `spawn_instructions` in any config, every
  seat's context and `workspace.md` are unchanged.
- A running seat holds the text of its own launch; `workspace.md` is rewritten at the next
  launch, rename, spawn or doctor repair. Edit the config, then restart the seats that
  should see the change.

## Watchdog defaults

The watchdog reads its tunables from environment variables (set them in the session shell before `ae <name>`, or via your shell rc):

| Variable | Default | Meaning |
|---|---|---|
| `AE_WATCHDOG_INTERVAL_SEC` | 60 | Cycle length in seconds |
| `AE_WATCHDOG_STALE_MIN` | 15 | Idle minutes before a nudge |
| `AE_WATCHDOG_MAX_NUDGES` | 2 | Nudges before escalating to alert |
| `AE_WATCHDOG_THROTTLE_ALERT_CYCLES` | 5 | Cycles of continuous upstream throttle before alert |
| `AE_WATCHDOG_TG_SUPERVISE_SEC` | 120 | Telegram-bridge revive cadence in seconds (`0` disables) |
| `AE_WATCHDOG_SWEEP_SEC` | 120 | Orchestrator changed-overview minimum-spacing fallback when `[workspace] sweep` is absent or invalid (`0` falls back to the normal watchdog; positive values below `60` become `60`) |
| `AE_WATCHDOG_SWEEP_RETRY_SEC` | 30 | After an UNDELIVERED sweep nudge, retry this soon instead of waiting a full `AE_WATCHDOG_SWEEP_SEC` (clamped to it; floor — lands on the next poll) |
| `AE_WATCHDOG_SWEEP_RETRY_MAX` | 6 | Fast retries allowed before falling back to normal cadence and raising one `meta-agent unreachable` alert |

The legacy `AE_LOOP_*` names are still honoured as fallbacks for each tunable. To turn the watchdog off for a single session, run `~/.ae/sessions/<name>/watchdog stop` once. The setting persists across resume.

Quota observation is configured by `[workspace] quota_every_secs`, not an environment fallback.
Launch persists the value in session meta so rename and resume keep the same cadence. It accepts
unsigned integer seconds only; `0` disables quota advisories. `[workspace] quota = off` disables
the advisories (and the injected quota guidance, the throttle line, and the settings quota entry)
regardless of the cadence; `ae quota` itself is unaffected in both states.

The cadence's due counter advances on whole watchdog cycles, so the
effective observation period is the requested seconds rounded up to that grid.

Idle reminders use `[workspace] idle_nudge_secs`, also persisted at launch and
validated as unsigned integer seconds. The default is 300; `0` disables idle
reminders, done challenges included, without disabling legacy stale detection for unmodeled frames.
`done_confirmations` is persisted too; bad input falls back to 2 with a note. A resumed session
keeps its recorded value, so config changes apply on relaunch; a session without the row records 2.

## Model tiers (recommended profiles)

ae names roles here, never concrete profiles. Which profile fills each role comes
from your spend policy (`[prompt] instructions`) and the Available profiles list in
`workspace.md` (from your `[profiles]`); the profiles seeded into a new config are
install-time defaults, not a statement of which models are current.

- **STRONG DEV (build slices):** a strong build profile.
- **BRAINPOWER (lead/colead seats, plans, rulings, hard debugging):** a judgment-tier profile.
- **CHORES/tests/simple slices:** a chore-tier profile; it also runs the orchestrator seat.
- **REVIEWER:** a profile from a different provider than the builder (rule 11) when usage allows.

**OpenCode Go seats (only if listed in workspace.md Available profiles or
configured [profiles]):** such a profile ranks by the model it serves, not by the
client. A strong served model is a peer of the strong build profiles for build
slices and judgment work; a fast served model is the fast alternative for bounded
build slices and review lanes. A shared OpenCode scope proves nothing about
provider — declare the served provider for rule 11. Chores still go to the
chore-tier profile; simple work never takes a judgment-tier profile.

## Where state lives

```
~/.ae/
├── config                          # global config (this file)
├── sessions/<name>/
│   ├── meta                        # session metadata (read-only mostly)
│   ├── events.jsonl                # event log (audit trail)
│   ├── memo.tsv                    # shared session memory
│   ├── workspace.md                # in-session reference for agents
│   ├── send, ask, review, reply…   # session helpers (regenerated on resume)
│   └── ...
└── worktrees/<name>/               # for --worktree mode
```

Nothing in your project directory changes.
