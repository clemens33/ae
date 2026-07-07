# aewatch contracts

This file is the machine-checkable contract surface for the aewatch sidecar. It
enumerates, as a versioned JSON fixture matrix, every behavior family aewatch must
reproduce from ae's bash watchdog + Telegram bridge. As of the phase-3 closer (s20)
each fixture carries a REAL `expect` (the exact side effects, plus non-effect
assertions for behaviors outside the effect stream) and a `source` anchor to the
implementation it is a contract for; the validator's coverage guards fail if any
behavior family, effect kind, non-effect assertion, or source is missing.

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

Every required behavior family below has at least one representative fixture; the
validator fails if any family is missing.

<!-- AEWATCH_CONTRACTS_JSON_START -->
```json
{
  "schema_version": 1,
  "fixtures": [
    {
      "id": "session.discovery.two-running",
      "description": "discover_sessions reads $AE_HOME/sessions/*/meta (Python-only; no effects)",
      "tags": [
        "session",
        "discovery",
        "two-running"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": ""
            }
          ],
          "tmux_options": {
            "%1": {
              "@ae_agent": "codex:lead"
            }
          }
        },
        {
          "name": "docs",
          "tmux_server": "ae-alt",
          "meta": {
            "session": "docs",
            "session_id": "sess-2",
            "work_dir": "/docs",
            "tmux_server": "ae-alt",
            "watchdog": "true",
            "agent.main": "claude:writer:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "claude:writer",
              "current_command": "claude",
              "capture": ""
            }
          ],
          "tmux_options": {
            "%1": {
              "@ae_agent": "claude:writer"
            }
          }
        }
      ],
      "telegram": {
        "enabled": false,
        "offset": 0,
        "state_tsv": ""
      },
      "source": [
        "aewatch:1806"
      ],
      "expect": {
        "effects": [],
        "files": {},
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": []
      }
    },
    {
      "id": "watchdog.status.branch-side-channel",
      "description": "_watchdog_set_status @ae_watchdog_status + branch side channel via _watchdog_branch_segment",
      "tags": [
        "watchdog",
        "status",
        "branch-side-channel"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": "idle"
            }
          ],
          "tmux_options": {}
        }
      ],
      "telegram": {
        "enabled": false,
        "offset": 0,
        "state_tsv": ""
      },
      "source": [
        "ae:7552",
        "ae:7731"
      ],
      "expect": {
        "effects": [
          {
            "kind": "tmux.set_option",
            "target": "work",
            "option": "@ae_watchdog_status",
            "value": "[watch ◌ starting]"
          },
          {
            "kind": "tmux.unset_option",
            "target": "work",
            "option": "@ae_branch_name"
          },
          {
            "kind": "tmux.set_option",
            "target": "work",
            "option": "@ae_branch_status",
            "value": ""
          }
        ],
        "files": {},
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": []
      }
    },
    {
      "id": "watchdog.nudge.idle-once",
      "description": "stale agent nudged once (_AE_EVENT_ACTION=nudge paste + nudge_count++): tmux.paste + nudge event",
      "tags": [
        "watchdog",
        "nudge",
        "idle-once"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": "idle"
            }
          ],
          "tmux_options": {}
        }
      ],
      "telegram": {
        "enabled": false,
        "offset": 0,
        "state_tsv": ""
      },
      "source": [
        "ae:8034"
      ],
      "expect": {
        "effects": [
          {
            "kind": "tmux.paste",
            "target": "%1",
            "text": "[ae watchdog] nudge",
            "submit": true
          },
          {
            "kind": "event.append",
            "session": "work",
            "event": {
              "actor": "watchdog",
              "action": "nudge",
              "target": "codex:lead",
              "summary": "no recent events, no recent ae activity"
            }
          }
        ],
        "files": {},
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": []
      }
    },
    {
      "id": "watchdog.alert.dead-pane",
      "description": "dead pane: ae_emit_event alert + display-message DEAD alert",
      "tags": [
        "watchdog",
        "alert",
        "dead-pane"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": "idle"
            }
          ],
          "tmux_options": {}
        }
      ],
      "telegram": {
        "enabled": false,
        "offset": 0,
        "state_tsv": ""
      },
      "source": [
        "ae:7809",
        "ae:7810"
      ],
      "expect": {
        "effects": [
          {
            "kind": "tmux.display_message",
            "text": "[ae watchdog] codex:lead is DEAD — process dropped to shell",
            "duration_ms": 10000
          },
          {
            "kind": "event.append",
            "session": "work",
            "event": {
              "actor": "watchdog",
              "action": "alert",
              "target": "codex:lead",
              "summary": "agent process dead — dropped to shell"
            }
          }
        ],
        "files": {},
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": []
      }
    },
    {
      "id": "watchdog.meta.locked-write",
      "description": "flock-guarded recovery: recovered session_id written back into meta under meta.lock",
      "tags": [
        "watchdog",
        "meta",
        "locked-write"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": "idle"
            }
          ],
          "tmux_options": {}
        }
      ],
      "telegram": {
        "enabled": false,
        "offset": 0,
        "state_tsv": ""
      },
      "source": [
        "ae:2572-2582"
      ],
      "expect": {
        "effects": [],
        "files": {
          "$AE_HOME/sessions/work/meta": "recovered session_id back-filled under flock 200>meta.lock (check-then-set sed write)"
        },
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": []
      }
    },
    {
      "id": "watchdog.telegram-supervise.throttled",
      "description": "bridge-revive scheduler every TG_SUPERVISE_SECS; AE_TMUX_SERVER=$tg_srv propagated into `telegram _supervise`; dispatch at _supervise)",
      "tags": [
        "watchdog",
        "telegram-supervise",
        "throttled"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "ae-alt",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "ae-alt",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": "idle"
            }
          ],
          "tmux_options": {}
        }
      ],
      "telegram": {
        "enabled": false,
        "offset": 0,
        "state_tsv": ""
      },
      "source": [
        "ae:7701",
        "ae:8088-8092",
        "ae:4452"
      ],
      "expect": {
        "effects": [
          {
            "kind": "telegram.supervise",
            "tmux_server": "ae-alt"
          }
        ],
        "files": {},
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": []
      }
    },
    {
      "id": "telegram.outbound.include-filter",
      "description": "outbound event forwarded when its action is in TG_INCLUDE and not in TG_EXCLUDE",
      "tags": [
        "telegram",
        "outbound",
        "include-filter"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": "idle"
            }
          ],
          "tmux_options": {}
        }
      ],
      "telegram": {
        "enabled": true,
        "offset": 42,
        "state_tsv": "# session_id\tinode\tbyte_offset\tlast_ts\n"
      },
      "source": [
        "ae:3232"
      ],
      "expect": {
        "effects": [
          {
            "kind": "telegram.send",
            "session": "work",
            "action": "send",
            "text": "[work] send  codex:lead"
          }
        ],
        "files": {},
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": []
      }
    },
    {
      "id": "telegram.inbound.use-target",
      "description": "/use sticky target: a plain inbound message routes to the target agent; reply sent",
      "tags": [
        "telegram",
        "inbound",
        "use-target"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": "idle"
            }
          ],
          "tmux_options": {}
        }
      ],
      "telegram": {
        "enabled": true,
        "offset": 42,
        "state_tsv": "# session_id\tinode\tbyte_offset\tlast_ts\n"
      },
      "source": [
        "ae:3506"
      ],
      "expect": {
        "effects": [
          {
            "kind": "telegram.send",
            "session": "work",
            "action": "reply",
            "text": "delivered to codex:lead"
          }
        ],
        "files": {},
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": []
      }
    },
    {
      "id": "telegram.security.token-redaction",
      "description": "a bot-token-bearing log line REDACTED (_redact) before reaching daemon.log",
      "tags": [
        "telegram",
        "security",
        "token-redaction"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": "idle"
            }
          ],
          "tmux_options": {}
        }
      ],
      "telegram": {
        "enabled": true,
        "offset": 42,
        "state_tsv": "# session_id\tinode\tbyte_offset\tlast_ts\n"
      },
      "source": [
        "ae:3147"
      ],
      "expect": {
        "effects": [
          {
            "kind": "log.write",
            "level": "WARNING",
            "message": "send_message failed (rc=1): bot<redacted> ..."
          }
        ],
        "files": {},
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": [
          "bot<redacted>"
        ]
      }
    },
    {
      "id": "telegram.command-menu.register",
      "description": "setMyCommands registers the full menu (list/use/session/help) ONCE at startup (non-effect)",
      "tags": [
        "telegram",
        "command-menu",
        "register"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": "idle"
            }
          ],
          "tmux_options": {}
        }
      ],
      "telegram": {
        "enabled": true,
        "offset": 42,
        "state_tsv": "# session_id\tinode\tbyte_offset\tlast_ts\n"
      },
      "source": [
        "ae:3314",
        "aewatch:1511"
      ],
      "expect": {
        "effects": [],
        "files": {},
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": [],
        "telegram_commands": [
          {
            "command": "list",
            "description": "Running sessions (name, id, last activity)"
          },
          {
            "command": "use",
            "description": "Redirect plain messages to a session/agent; /use clear resets to the steward"
          },
          {
            "command": "session",
            "description": "Message an agent: /session <name> send|ask <agent> <msg>"
          },
          {
            "command": "help",
            "description": "Show help"
          }
        ]
      }
    },
    {
      "id": "daemon.runtime.bridge-handoff",
      "description": "s19 handoff: marker + fresh heartbeat -> kill ae-telegram every server -> send (bash guard ae:4192, handoff aewatch:3316)",
      "tags": [
        "daemon",
        "runtime",
        "bridge-handoff"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [
        {
          "name": "work",
          "tmux_server": "",
          "meta": {
            "session": "work",
            "session_id": "sess-1",
            "work_dir": "/repo",
            "tmux_server": "",
            "watchdog": "true",
            "agent.main": "codex:lead:uuid"
          },
          "events": [],
          "panes": [
            {
              "pane_id": "%1",
              "agent": "codex:lead",
              "current_command": "codex",
              "capture": "idle"
            }
          ],
          "tmux_options": {}
        }
      ],
      "telegram": {
        "enabled": true,
        "offset": 42,
        "state_tsv": "# session_id\tinode\tbyte_offset\tlast_ts\n"
      },
      "source": [
        "ae:4192",
        "aewatch:3316"
      ],
      "expect": {
        "effects": [
          {
            "kind": "telegram.send",
            "session": "work",
            "action": "send",
            "text": "[work] send  codex:lead"
          }
        ],
        "files": {
          "$AE_HOME/aewatch/bridge-owner": "<pid> <ns> — written before stop-bash; NOT a file.write effect"
        },
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": [],
        "killed_sessions": [
          "ae-telegram"
        ],
        "handoff_order": [
          "bridge-owner-marker",
          "stop-bash",
          "send"
        ]
      }
    },
    {
      "id": "daemon.runtime.singleton-heartbeat",
      "description": "per-AE_HOME singleton flock + heartbeat touched each tick (write_heartbeat) + daemon.log",
      "tags": [
        "daemon",
        "runtime",
        "singleton-heartbeat"
      ],
      "time": {
        "now": "2026-07-05T07:00:00Z",
        "epoch": 1783234800
      },
      "config": {
        "ae_home": "$TMP/ae",
        "ini": "[workspace]\nwatchdog = true\n"
      },
      "sessions": [],
      "telegram": {
        "enabled": false,
        "offset": 0,
        "state_tsv": ""
      },
      "source": [
        "aewatch:2893"
      ],
      "expect": {
        "effects": [
          {
            "kind": "file.write",
            "path": "$AE_HOME/aewatch/heartbeat",
            "redacted": false
          },
          {
            "kind": "log.write",
            "level": "INFO",
            "message": "daemon shutting down (reason=shutdown)"
          }
        ],
        "files": {
          "$AE_HOME/aewatch/aewatch.lock": "flock singleton (one supervisor per AE_HOME)"
        },
        "tmux_options": {},
        "exit_code": 0,
        "log_contains": []
      }
    }
  ]
}
```
<!-- AEWATCH_CONTRACTS_JSON_END -->

## Coverage guards + source provenance (s20)

Each fixture above is now a **representative** contract (not a placeholder): a real
`expect.effects` list, plus non-effect assertions where the behavior is outside
`EFFECT_KINDS` — `expect.telegram_commands` (setMyCommands), and for the s19 bridge
handoff `expect.handoff_order` + `expect.killed_sessions` (the bridge-owner marker is
documented via `expect.files`, never a forged `file.write`). `validate_contracts` fails
if any required family, any `EFFECT_KIND`, a required non-effect assertion, or a
`source` anchor is missing/malformed — and every guard is mutation-proven in
`tests/aewatch_tests/test_46_contracts_coverage.py` (delete the guarded thing -> validate
fails). This matrix stays **documentation + coverage**; the dual-run oracle keeps its
own test-local fixtures.

`source` anchors: the validator machine-checks the anchor SHAPE (`ae:<line>` for a bash
implementation, `aewatch:<line>` for python-only behavior such as the handoff order).
Anchor SEMANTIC freshness — that a cited line still points at the behavior — is a
HUMAN-REVIEWED contract, re-checked when ae/aewatch shift, not machine-proven.
