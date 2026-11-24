# vendorblock fixtures

Frozen with the acceptance tests. Provenance is the driver's 2026-11-24 probe
(`.local/vendorblock/frames/README.md`). No second probe was run.

| File | What it is |
|---|---|
| `opencode-copilot-quota.raw.txt` | MEASURED. opencode 2.4.1, the `cpsol61h` launch translation, pane 140 columns. The row `     Error: You have exceeded your monthly quota` (five spaces) shares the row with sidebar text. |
| `opencode-copilot-quota-100x30.raw.txt` | MEASURED at 100×30. Two failed turns. No sidebar. The row ends after the sentence. |
| `opencode-copilot-quota-two-turns.raw.txt` | MEASURED. The failure repeats once per turn. |
| `opencode-copilot-quota.toast.raw.txt` | MEASURED. A `Session failed` toast beside the transcript. The toast is not the row that books. |
| `opencode-width-table.txt` | MEASURED at 19 widths. The sidebar, when present, starts at column `L + 4`, where `L` is the trimmed length of the composer rule row (`  ╹▀…`). |
| `muse-402.reconstructed.txt` | RECONSTRUCTED. Muse 1.4.25 answered `hi` on 2026-11-24, so the 402 did not reproduce. The healthy frame's reply row is replaced by the failure row. The words are ae-feedback item 111 (`API error 402`, `Billing verification failed`) plus the 2026-11-18 pane quote's tail `Billing Issue`. The `◆ model failed: ` head is the measured muse row. The request id is a zeroed placeholder, not a captured id. |
| `muse-healthy-hi.raw.txt` | MEASURED negative. The same muse probe, a normal reply, no 402. |

Paths under `/private/tmp/vbprobe-*` are the probe directory. No account name was in a measured frame.
