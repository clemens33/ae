# aewatch contracts

This file is the machine-checkable contract surface for the aewatch sidecar. It
enumerates, as a versioned JSON fixture matrix, every behavior family aewatch must
reproduce from ae's bash watchdog + Telegram bridge. Phase 2/3 fill each fixture's
`expect` with the exact side effects (events emitted, tmux options set, nudge/send
text, files touched); **phase 1 only enumerates the families** so later phases
cannot silently shrink the surface.

## Source of truth

Prose in this file is explanatory only and is never asserted. The single source of
truth is one JSON object between the literal markers below. The loader
(`extract_contracts_json`) and validator (`validate_contracts`) live in the
`aewatch` script; both the CLI (`aewatch contracts validate`) and the test suite
(`tests/aewatch_tests/test_01_contracts.py`) call that same code.

Validate the committed matrix:

```bash
python3 contrib/aewatch/aewatch contracts validate
```

## Schema (version 1)

- `schema_version`: `1`.
- `fixtures`: list of fixtures, each with:
  - `id`: unique, lowercase dotted/kebab token (e.g. `watchdog.nudge.idle-once`).
  - `description`, `tags`.
  - `time`: `{now, epoch}`.
  - `config`: `{ae_home, ini}` — the ae INI text this fixture runs against.
  - `sessions`: list of session inputs (meta, events, panes, tmux options).
  - `telegram`: `{enabled, offset, state_tsv}`.
  - `expect`: `{effects, files, tmux_options, exit_code, log_contains}` —
    `effects` MUST be present even when empty (the normalized side-effect oracle).

Every required behavior family below has at least one placeholder fixture; the
validator fails if any family is missing.

<!-- AEWATCH_CONTRACTS_JSON_START -->
```json
{
  "schema_version": 1,
  "fixtures": [
    {
      "id": "session.discovery.two-running",
      "description": "discover two running sessions from $AE_HOME/sessions/*/meta",
      "tags": ["session", "discovery"],
      "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
      "config": {"ae_home": "$TMP/ae", "ini": "[workspace]\nwatchdog = true\n"},
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {"session": "work", "session_id": "sess-1", "work_dir": "/repo", "tmux_server": "", "watchdog": "true", "agent.main": "codex:lead:uuid"},
          "events": [{"ts": "2026-07-05T06:20:00Z", "actor": "codex:lead", "action": "state", "summary": "working"}],
          "panes": [{"pane_id": "%1", "agent": "codex:lead", "current_command": "codex", "capture": ""}],
          "tmux_options": {"%1": {"@ae_agent": "codex:lead"}}
        },
        {
          "name": "docs",
          "tmux_server": "ae-alt",
          "meta": {"session": "docs", "session_id": "sess-2", "work_dir": "/docs", "tmux_server": "ae-alt", "watchdog": "true", "agent.main": "claude:writer:uuid"},
          "events": [],
          "panes": [{"pane_id": "%1", "agent": "claude:writer", "current_command": "claude", "capture": ""}],
          "tmux_options": {"%1": {"@ae_agent": "claude:writer"}}
        }
      ],
      "telegram": {"enabled": false, "offset": 0, "state_tsv": ""},
      "expect": {"effects": [], "files": {}, "tmux_options": {}, "exit_code": 0, "log_contains": []}
    },
    {
      "id": "watchdog.status.branch-side-channel",
      "description": "watchdog sets @ae_watchdog_status / @ae_branch_status / @ae_branch_name and restores status-right",
      "tags": ["watchdog", "status", "tmux-options"],
      "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
      "config": {"ae_home": "$TMP/ae", "ini": "[workspace]\nwatchdog = true\n"},
      "sessions": [],
      "telegram": {"enabled": false, "offset": 0, "state_tsv": ""},
      "expect": {"effects": [], "files": {}, "tmux_options": {}, "exit_code": 0, "log_contains": []}
    },
    {
      "id": "watchdog.nudge.idle-once",
      "description": "an idle agent past the stale threshold with no quiet state is nudged exactly once",
      "tags": ["watchdog", "nudge", "quiet-state"],
      "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
      "config": {"ae_home": "$TMP/ae", "ini": "[workspace]\nwatchdog = true\n"},
      "sessions": [],
      "telegram": {"enabled": false, "offset": 0, "state_tsv": ""},
      "expect": {"effects": [], "files": {}, "tmux_options": {}, "exit_code": 0, "log_contains": []}
    },
    {
      "id": "watchdog.alert.dead-pane",
      "description": "a registered agent whose pane vanished raises an alert; a recovered pane clears it",
      "tags": ["watchdog", "alert", "dead"],
      "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
      "config": {"ae_home": "$TMP/ae", "ini": "[workspace]\nwatchdog = true\n"},
      "sessions": [],
      "telegram": {"enabled": false, "offset": 0, "state_tsv": ""},
      "expect": {"effects": [], "files": {}, "tmux_options": {}, "exit_code": 0, "log_contains": []}
    },
    {
      "id": "watchdog.meta.locked-write",
      "description": "watchdog on/off honored (canonical watchdog key, legacy loop fallback); meta writes hold meta.lock",
      "tags": ["watchdog", "meta", "flock"],
      "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
      "config": {"ae_home": "$TMP/ae", "ini": "[workspace]\nwatchdog = true\n"},
      "sessions": [],
      "telegram": {"enabled": false, "offset": 0, "state_tsv": ""},
      "expect": {"effects": [], "files": {}, "tmux_options": {}, "exit_code": 0, "log_contains": []}
    },
    {
      "id": "telegram.outbound.include-filter",
      "description": "outbound events honor include/exclude, forward chat events, track state.tsv byte offsets, retry at-least-once",
      "tags": ["telegram", "outbound"],
      "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
      "config": {"ae_home": "$TMP/ae", "ini": "[telegram]\nenabled = true\n"},
      "sessions": [],
      "telegram": {"enabled": true, "offset": 0, "state_tsv": ""},
      "expect": {"effects": [], "files": {}, "tmux_options": {}, "exit_code": 0, "log_contains": []}
    },
    {
      "id": "telegram.inbound.use-target",
      "description": "getUpdates offset advances at-most-once; allowlist + private-chat check; /use and /use clear set the current target; steward default routing",
      "tags": ["telegram", "inbound"],
      "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
      "config": {"ae_home": "$TMP/ae", "ini": "[telegram]\nenabled = true\n"},
      "sessions": [],
      "telegram": {"enabled": true, "offset": 0, "state_tsv": ""},
      "expect": {"effects": [], "files": {}, "tmux_options": {}, "exit_code": 0, "log_contains": []}
    },
    {
      "id": "telegram.security.token-redaction",
      "description": "token file owner/mode checked; bot token, bot<TOKEN> URLs, and token-file contents redacted in logs; message text never shell-interpolated",
      "tags": ["telegram", "security", "redaction"],
      "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
      "config": {"ae_home": "$TMP/ae", "ini": "[telegram]\nenabled = true\n"},
      "sessions": [],
      "telegram": {"enabled": true, "offset": 0, "state_tsv": ""},
      "expect": {"effects": [], "files": {}, "tmux_options": {}, "exit_code": 0, "log_contains": []}
    },
    {
      "id": "daemon.runtime.singleton-heartbeat",
      "description": "singleton lock under $AE_HOME/aewatch; heartbeat touched per tick; daemon.log rotation; backoff on crash-loop",
      "tags": ["daemon", "runtime", "singleton"],
      "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
      "config": {"ae_home": "$TMP/ae", "ini": ""},
      "sessions": [],
      "telegram": {"enabled": false, "offset": 0, "state_tsv": ""},
      "expect": {"effects": [], "files": {}, "tmux_options": {}, "exit_code": 0, "log_contains": []}
    }
  ]
}
```
<!-- AEWATCH_CONTRACTS_JSON_END -->

## Filling `expect` in later phases

Each fixture above is a **placeholder** with an empty `expect.effects`. Phase 2
(watchdog) and phase 3 (telegram) replace those with the full normalized
side-effect list — every event appended, tmux option set, nudge/send text, and
file touched — so the dual-run parity oracle can diff bash vs aewatch against a
single source of truth.
