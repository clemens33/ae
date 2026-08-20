# Stage-2 evidence schema inventory

This is a structure-only inventory of:

* `docs/migration/evidence/batch-c-artifacts/` (4,508 files; six arm groups), and
* `docs/migration/evidence/l-artifacts/` (7,085 files; six sections and 126 arm directories).

Combined census: 11,593 files.

There is no `manifest.json` in either tree.  “Manifest” means a TSV snapshot here.
The inventory reports names, layout, serialization, cardinality, and types.  It does not
report captured output values.

## Synthetic-example rule

Every example below is invented.  The vocabulary is deliberately outside the corpus:
`zzz-example-1`, `zzz-example-2`, `zzz-case`, `zzz-arm`, `zzz-step`, `zzz-channel`,
`zzz-command`, `<rc>`, `<PATH>`, and a digest of exactly 64 zeroes
(`0000000000000000000000000000000000000000000000000000000000000000`).  Timestamps in
examples are always the impossible-but-well-formed `2999-12-31T23:59:59Z`; numeric example
fields use `987654321`.

`<ROOT>`, `<SECTION>`, `<ARM>`, `<CASE>`, `<MEMBER>`, `<SESSION>`, `<STEP>`, `<LABEL>`,
`<PID>`, and `<UUID>` are path-pattern placeholders, not recorded values.

## Layout census

| tree | top-level shape | count |
|---|---|---:|
| batch C | `arms/<ARM>/...` | 6 arms; 99 case directories |
| batch C | `templates/<GROUP>/{_meta,fixture-bytes}` | 17 groups, 48 fixture members |
| batch C | `twd-precursor/{a1,a2,a3,...}` | three producer arms plus harness/specimens |
| L | `L-<SECTION>/{arms,harness-snapshot,...}` | 6 sections; 126 arms |
| L | `_admissibility/*`, `_harness/*` | 17 admissibility notes; 55 shared harness files |

The file counts in the tables below are current-tree counts.  A glob row is one file kind
even when its basename contains a step, barrier, PID, UUID, or test label.

The tables group 45 semantic file kinds.  The suffix census below accounts for every file,
including low-volume source, fixture, and launch-snapshot suffixes that do not deserve a
separate parser.

### Complete suffix census

| suffix (last-dot classification) | batch C | L |
|---|---:|---:|
| `.txt` | 1,485 | 2,347 |
| `.stdout` | 960 | 399 |
| `.tmuxtrace` | 957 | 0 |
| `.tsv` | 371 | 1,668 |
| `.jsonl` | 161 | 53 |
| `.stderr` | 128 | 401 |
| extensionless | 117 | 4 |
| `.log` | 114 | 178 |
| `.sh` | 110 | 335 |
| `.py` | 45 | 23 |
| `.flockspy` | 26 | 0 |
| `.patch` | 8 | 18 |
| `.bin` | 5 | 0 |
| `.xtrace` | 4 | 0 |
| `.out` | 3 | 2 |
| `.json` | 3 | 1 |
| `.err` | 3 | 0 |
| `.pre-refresh` | 2 | 0 |
| `.c` | 2 | 0 |
| `.NOTE` | 2 | 0 |
| `.post-refresh` | 1 | 0 |
| `.md` | 1 | 41 |
| `.rc` | 0 | 399 |
| `.invocation` | 0 | 377 |
| `.raw` | 0 | 200 |
| `.nul` | 0 | 195 |
| `.lines` | 0 | 195 |
| `.diff` | 0 | 100 |
| `.od` | 0 | 89 |
| `.stdout-at-cut` | 0 | 16 |
| `.stderr-at-cut` | 0 | 16 |
| `.harvested` | 0 | 12 |
| `.sed` | 0 | 7 |
| `.lock` | 0 | 1 |
| `.firstline` | 0 | 1 |
| `.0after-launch` | 0 | 1 |
| `.1first-run` | 0 | 1 |
| `.2after-first-run-pane-killed` | 0 | 1 |
| `.3second-run` | 0 | 1 |
| `.4after-second-run-pane-killed` | 0 | 1 |
| `.5after-stop` | 0 | 1 |
| `.6after-resume-rewrite` | 0 | 1 |
| **total** | **4,508** | **7,085** |

## JSON and JSON Lines

All JSON objects below are UTF-8 JSON.  JSONL means one JSON value per physical line; an
empty file, a missing final newline, and a malformed/partial final record all occur in the
corpus and are byte-level states that an importer must preserve.
Unless explicitly described as an array, the listed JSON keys are top-level scalars; no
stable nested object schema is present.

### JSON objects

| path pattern | format / count | schema (presence and nesting) | synthetic example |
|---|---|---|---|
| `batch-c-artifacts/twd-precursor/specimens/summary.<ARM>.json` | JSON object; 3 | All eight top-level keys are always present: `arm:string`; `total_specimens:number`; `alert_family_specimens:number`; `source_events_file:string`; `source_events_bytes:number`; `source_events_sha256:string`; `alert_family_actions:array[string]`; `all_actions:array[string]`. Arrays are present even when their cardinality is zero. | `{"arm":"zzz-arm","total_specimens":987654321,"alert_family_specimens":987654321,"source_events_file":"<ROOT>/zzz-events.jsonl","source_events_bytes":987654321,"source_events_sha256":"0000000000000000000000000000000000000000000000000000000000000000","alert_family_actions":["zzz-action"],"all_actions":["zzz-action","zzz-action-2"]}` |
| `l-artifacts/L-COMPACT/arms/sigpipe/sigpipe-record.json` | JSON object; 1 | All eleven top-level keys are present: `producer_argv:array[string]`; `producer_exit_code:number`; `producer_exited_normally:boolean`; `producer_signalled:boolean`; `producer_term_signal:number|null`; `producer_term_signal_name:string|null`; `raw_wait_status:number`; `consumer_read_first_line:string`; `consumer_closed_read_end_after:string`; `consumer_exit_code:number`. The two producer signal fields are nullable. | `{"producer_argv":["zzz-command","zzz-step"],"producer_exit_code":987654321,"producer_exited_normally":true,"producer_signalled":false,"producer_term_signal":null,"producer_term_signal_name":null,"raw_wait_status":987654321,"consumer_read_first_line":"zzz-line","consumer_closed_read_end_after":"2999-12-31T23:59:59Z","consumer_exit_code":987654321}` |

### JSONL records

| path pattern | format / count | record schema (presence) | synthetic example |
|---|---|---|---|
| `batch-c-artifacts/templates/<GROUP>/fixture-bytes/<MEMBER>/sessions/<SESSION>/events.jsonl` | JSONL; 56 | Product event objects. `ts:string`, `action:string`, and `actor:string` are present on valid event objects. `summary:string`, `target:string`, `ref:string`, `body_file:string`, `actor_session:string`, `actor_slot:string`, `target_session:string`, and `target_slot:string` are optional and action/routing dependent. Mutation fixtures may add `zzz_unknown:string` and `another_unknown:number`; unknown keys are therefore allowed. | `{"ts":"2999-12-31T23:59:59Z","action":"zzz-action","actor":"zzz-actor","summary":"zzz-summary","target":"zzz-target","ref":"zzz-ref","body_file":"<PATH>"}` |
| `l-artifacts/L-<SECTION>/arms/<ARM>/*.events*.jsonl` and `.../events.<LABEL>.jsonl` | JSONL; 51 arm captures plus 2 specimen copies | Same product event envelope as the fixture events. `ts`, `action`, `actor` are the stable core; `summary`, `target`, `ref`, `body_file`, and the four session/slot facets are optional. Some captures are intentionally empty. | `{"ts":"2999-12-31T23:59:59Z","action":"zzz-action","actor":"zzz-actor","summary":"zzz-summary","target":"zzz-target"}` |
| `l-artifacts/L-END/specimens/<ARCHIVE-UUID>/events.jsonl` and the compact copied archive | JSONL; 2 | Same product event envelope; the archive copies are byte snapshots, not a new schema. | `{"ts":"2999-12-31T23:59:59Z","action":"zzz-action","actor":"zzz-actor"}` |
| `batch-c-artifacts/twd-precursor/<a1|a2|a3>/events/*.jsonl` | JSONL; 93 | Watchdog/event-tail specimens. Valid objects use `action:string`, `actor:string`, and `ts:string`; `summary:string`, `target:string`, and `body_file:string` are optional. Empty snapshots exist. | `{"ts":"2999-12-31T23:59:59Z","action":"zzz-action","actor":"zzz-actor","summary":"zzz-summary"}` |
| `batch-c-artifacts/twd-precursor/specimens/{specimens,alert-specimens}.<ARM>.jsonl` | JSONL; 6 | Specimen records. All 15 top-level keys are present: `action:string`, `actor:string`, `arm:string`, `byte_len_no_nl:number`, `byte_len_with_nl:number`, `byte_offset:number`, `first_seen_capture_label:string`, `line_no:number`, `raw_line_no_nl:string`, `ref:string`, `sha256_line_no_nl:string`, `sha256_line_with_nl:string`, `summary:string`, `ts:string`. | `{"action":"zzz-action","actor":"zzz-actor","arm":"zzz-arm","byte_len_no_nl":987654321,"byte_len_with_nl":987654321,"byte_offset":987654321,"first_seen_capture_label":"zzz-capture","line_no":987654321,"raw_line_no_nl":"zzz-raw-line","ref":"zzz-ref","sha256_line_no_nl":"0000000000000000000000000000000000000000000000000000000000000000","sha256_line_with_nl":"0000000000000000000000000000000000000000000000000000000000000000","summary":"zzz-summary","ts":"2999-12-31T23:59:59Z"}` |
| `batch-c-artifacts/templates/<GROUP>/fixture-bytes/<MEMBER>/{_a1-...producer-input.jsonl,_d02-...reply.jsonl,append-sentinel.jsonl,rotate-*.jsonl,throttled.events.before.jsonl}` | JSONL; 6 | Two sub-kinds: the producer-input file has six always-present string keys `content`, `cwd`, `id`, `role`, `timestamp`, `type`; the other five use the product event envelope above. | Producer input: `{"content":"zzz-content","cwd":"<PATH>","id":"zzz-id","role":"zzz-role","timestamp":"2999-12-31T23:59:59Z","type":"zzz-type"}` |

## TSV schemas

`<TAB>` below means one literal tab.  The word “empty” means a zero-length field between
tabs; a literal `-` or `ABSENT` is a non-empty sentinel and is not an empty field.

| path pattern | format / count | columns and emptiness | synthetic example |
|---|---|---|---|
| `batch-c-artifacts/arms/<ARM>/CASES.tsv` | headed TSV; 6 | 4 columns: `case_dir`, `ledger_sha256`, `ledger_lines`, `files`; all rows have four non-empty fields. | `zzz-case<TAB>0000000000000000000000000000000000000000000000000000000000000000<TAB>987654321<TAB>987654321` |
| `batch-c-artifacts/arms/<ARM>/ledger.tsv` | headed TSV; 6 | 4 columns: `case`, `rows`, `group`, `member`; no empty cells observed. | `zzz-case<TAB>SC-example<TAB>zzz-group<TAB>zzz-member` |
| `batch-c-artifacts/arms/<ARM>/<CASE>/consumers.tsv` | headed TSV; 99 | 10 columns: `consumer`, `rc`, `stdout_sha256`, `stdout_bytes`, `stderr_sha256`, `stderr_bytes`, `tmuxtrace_sha256`, `tmuxtrace_lines`, `bounded`, `argv`; no empty cells. `-` is used for absent hash/bounded values. | `zzz-step<TAB><rc><TAB>0000000000000000000000000000000000000000000000000000000000000000<TAB>987654321<TAB>-<TAB>987654321<TAB>0000000000000000000000000000000000000000000000000000000000000000<TAB>987654321<TAB>-<TAB>zzz-command` |
| `batch-c-artifacts/arms/<ARM>/harness/case-schema.tsv` | headed TSV; 6 | 2 columns: `kind`, `required`; a leading `#` explanation precedes the header. `required` can contain a directory marker or glob syntax. No empty cells. | `zzz-kind<TAB>zzz-required/` |
| `batch-c-artifacts/PATH-CITES.tsv` | headed TSV; 1 | 5 columns: `citation`, `class`, `resolved_base`, `expansions`, `first_resolution`; 207 data rows, no empty cells. | `zzz-citation<TAB>zzz-class<TAB>zzz-base<TAB>987654321<TAB>zzz-resolution` |
| `batch-c-artifacts/templates/{FINGERPRINTS.tsv,FINGERPRINTS.superseded-pre-locale-fix.tsv}` | headed TSV; 2 | 6 columns: `group`, `member`, `fingerprint_pre_protection`, `fingerprint_protected`, `session`, `files`; all six fields non-empty. | `zzz-group<TAB>zzz-member<TAB>0000000000000000000000000000000000000000000000000000000000000000<TAB>0000000000000000000000000000000000000000000000000000000000000000<TAB>zzz-session<TAB>987654321` |
| `batch-c-artifacts/{arms/<ARM>/<CASE>/{manifest.before.tsv,manifest.after.tsv,manifest.at-barrier-*.tsv},templates/<GROUP>/_meta/*.modes.tsv,arms/<ARM>/manifest.s*.tsv}` | positional TSV; 251 | 5 columns, no header: `entry_type`, `mode`, `sha256-or-dash`, `link_target-or-dash`, `relative_path`. No empty cells; `-` represents a non-applicable digest/link. | `file<TAB>0755<TAB>0000000000000000000000000000000000000000000000000000000000000000<TAB>-<TAB>./zzz-example-1` |
| `l-artifacts/L-<SECTION>/arms/<ARM>/{*.aehome.tsv,*.sessions.tsv,*.sessions-full.tsv,*.workdir.tsv,*.sessiondir.tsv,*.linktarget.tsv,*.staging-*.tsv,*.final-archive.tsv,*.manifest.tsv,...}` | manifest-style TSV; 1,559 (1,406 headed snapshots plus 153 absence/payload variants) | Standard form starts with `# manifest-root<TAB><ROOT>` then `#path<TAB>type<TAB>mode<TAB>nlink<TAB>size<TAB>link<TAB>sha256`; data rows have 7 columns: `path`, `type`, `mode`, `nlink`, `size`, `link`, `sha256`. Standard rows have no empty cells; `-` is the non-applicable link/digest sentinel. Absence variants use exactly 2 columns: `ABSENT<TAB><ROOT>`. Three archived `memo.tsv` payloads are zero-byte files, not headered tables. | `# manifest-root<TAB><ROOT>`<br>`#path<TAB>type<TAB>mode<TAB>nlink<TAB>size<TAB>link<TAB>sha256`<br>`. <TAB>dir<TAB>0755<TAB>987654321<TAB>987654321<TAB>-<TAB>-` |
| `l-artifacts/L-<SECTION>/arms/<ARM>/barrier-order.tsv` | positional TSV; 50 | 2 columns: ordinal and barrier/marker label. No header and no empty cells. | `987654321<TAB>zzz-channel` |
| `l-artifacts/L-<SECTION>/arms/<ARM>/hook-trace.tsv` and `trace-channels.txt`-style TSV blocks | positional TSV; 58 `hook-trace.tsv` plus one plain-text trace-channel file | 4 columns: ordinal, barrier/channel name, process id, monotonic timestamp. No empty cells in the TSV rows. Trace-channel text adds a two-column legend after the four-column firing list. | `987654321<TAB>zzz-channel<TAB>987654321<TAB>2999-12-31T23:59:59Z` |
| `l-artifacts/L-COMPACT/residual-rc/rc-table.tsv` | headed TSV with comments; 1 | Header says 4 columns: `arm`, `step`, `rc`, `invocation`. Most rows have four fields; an unescaped tab in two recorded invocation strings creates rows with 5 or 7 physical fields. One blank separator line exists. Treat the invocation as the tail, not as a safely quoted TSV field. No zero-length data cells were observed. | `zzz-arm<TAB>zzz-step<TAB><rc><TAB>zzz-command` |

## Plain text, byte captures, and source snapshots

| path pattern | format / count | schema / cardinality | synthetic example |
|---|---|---|---|
| `l-artifacts/L-<SECTION>/arms/<ARM>/ARM.txt` | tab-separated key/value text; 127 (126 arm records plus the compact residual-rc record) | One `key<TAB>value` per line. Keys vary by section. Union includes `arm`, `section`, `roster_ids`, `construction`, `fixture`, `topology`, `transport`, `hook_patch_version`, `binary`/`binary.sha256`, `session`/`session_uuid`, `parent_uuid`, `template_uuid`, `op`, `op_rc`, `launch_rc`, `end_rc`, `stop_rc`, `resume_rc`, `push_rc`, `pull_rc`, `barrier_bound_sec`, `handover_bound_sec`, `cut_barrier`, `mutation_target`, `planted_claim`, `policy`, `class`, `direction`, `ordered_pair`, `flock`, `shims`, `trace`, `captures`, `note`, and section-specific rc/bound fields. Values are scalar text; some keys are absent in a section. | `arm<TAB>zzz-arm`<br>`section<TAB>zzz-section`<br>`op_rc<TAB><rc>` |
| `batch-c-artifacts/arms/<ARM>/<CASE>/admissibility-ledger.txt` | tab-separated `key=value` event ledger; 99 | Every line begins with `seq`, `utc`, `epoch`, and `event` tokens; event-specific tokens add arm/case/template, clone fingerprints, artifact names/digests, barrier/consumer labels, rc, line counts, and flags. Token count varies by event. A few D-arm annotations are free text after the key/value tokens. | `seq=987654321<TAB>utc=2999-12-31T23:59:59Z<TAB>epoch=987654321<TAB>event=zzz-event<TAB>artifact=zzz-file<TAB>artifact_sha256=0000000000000000000000000000000000000000000000000000000000000000` |
| `batch-c-artifacts/arms/<ARM>/<CASE>/case.txt` | plain text key/value plus prose; 99 | First lines carry case identity, row/template references, clone fingerprints, socket note, frozen-binary references, UTC bounds, manifest-diff counts, and topology flags. Later lines are human-readable notes. Keys are not a fixed record schema. | `arm=zzz-arm case=zzz-case clone_mode=zzz-mode`<br>`manifest_diff_lines=987654321`<br>`note=zzz-note` |
| `batch-c-artifacts/arms/<ARM>/<CASE>/{env.txt,env-tab-selfcheck.txt,tmux.before.txt,tmux.after.txt,manifest.diff.txt}` | plain text; 99 each for the standard case files (manifest diff can be empty) | `env.txt` is an `env -i` style one-variable-per-line capture. `env-tab-selfcheck.txt` is labelled probe metadata plus byte/field checks. tmux files are raw command output. `manifest.diff.txt` is a unified diff and may be zero bytes. | `LANG=zzz-locale`<br>`tab_survived=zzz-boolean`<br>`<CAPTURED-DIFF-BYTES-OMITTED>` |
| `batch-c-artifacts/arms/<ARM>/<CASE>/out/<LABEL>.stdout`, `.stderr`, `.tmuxtrace` | plain text; 958 stdout, 128 stderr, 957 tmux traces | stdout/stderr are unparsed command streams and can be empty. tmuxtrace is a one-line-or-more key/value plus delegated-argv trace; it can contain process id, effective server variables, argc, and argv tokens. | `pid=987654321<TAB>AE_TMUX_SERVER=<PATH><TAB>argc=987654321<TAB>zzz-command` |
| `l-artifacts/L-<SECTION>/arms/<ARM>/<STEP>.invocation`, `.rc`, `.stdout`, `.stderr` | plain text; 377 invocation, 399 rc, 399 stdout, 401 stderr | Invocation records preserve argv/environment construction; rc records preserve one operation status; stdout/stderr preserve separate byte streams. Each can be empty where the operation did not reach that capture point; `.rc` is not guaranteed to be numeric text because harness failures and cut records are represented too. | `argv=zzz-command zzz-step`<br>`<rc>`<br>`zzz-output` |
| `l-artifacts/L-<SECTION>/arms/<ARM>/*.stdout-at-cut`, `*.stderr-at-cut`, `*.od`, `*.stream-sizes.txt` | plain text; 16 stdout-at-cut, 16 stderr-at-cut, 89 od dumps | At-barrier stream snapshots, octal/character dumps, and size tables. `.od` is a rendering of bytes, not a second semantic format; empty source streams produce empty dumps. | `<OFFSET>  z z z - b y t e s` |
| `l-artifacts/L-<SECTION>/arms/<ARM>/fake/fake.<PID>.{argv.nul,stdin.raw,stdin.lines}` | binary/raw text; 195 NUL-delimited argv captures, 200 raw stdin captures, 195 line views | `.argv.nul` is NUL-separated argv; `.stdin.raw` is the exact stdin byte stream; `.stdin.lines` is a line-oriented rendering. All three may be empty. | `zzz-arg<NUL>zzz-arg-2<NUL>` |
| `l-artifacts/L-<SECTION>/arms/<ARM>/{tmux-argv.log,tmux-argv.op.log,flock-spy*.log,consumer-inproc.txt,preflight-tab.txt,stop-results.txt,...}` | plain text traces; role-specific counts (126 tmux argv logs, 21 operation tmux logs, 125 preflight tabs, and other named captures) | These are not one fixed table. They are line-oriented key/value traces, command argv lines, byte probes, or JSON-looking lines embedded in text. Import by path role and preserve bytes; do not parse a generic `.txt` as one schema. | `source=zzz-source<TAB>field=zzz-field` |
| `batch-c-artifacts/twd-precursor/<ARM>/{events,fs-manifests,panes,stamps,tmux,watchdog}/*` | plain text snapshot channels; 93 each for events/fs-manifests/panes/stamps/tmux, 86 watchdog | File names carry a capture label. Contents are raw channel snapshots; the events subdirectory is the JSONL kind above. The other channels are opaque text and may be empty. | `zzz-channel<TAB>zzz-snapshot-bytes` |
| `batch-c-artifacts/templates/<GROUP>/_meta/*` | mixed TSV/text/JSONL metadata; 133 files | 48 `*.modes.tsv` use the 5-column positional manifest schema; 22 `*.mutation.txt` are free-form mutation records; 9 date-shim logs are line-oriented invocation records; remaining files are text notes or one JSONL payload. | `mutation=zzz-mutation<TAB>before=zzz-state<TAB>after=zzz-state` |
| `batch-c-artifacts/templates/<GROUP>/fixture-bytes/<MEMBER>/{config,sessions/<SESSION>/{meta,events.jsonl,messages/*}}` | fixture payload tree; 48 `config`, 56 `meta`, 56 `events.jsonl`, 44 message directories, 111 message files | `config` is INI-like section/key text. `meta` is one `key=value` scalar per line; common keys include `mode`, `origin`, `session`, `session_id`, `work_dir`, `layout`, `config`, `main_pane`, `ae_path`, `ae_version`, `tmux_server`, `tmux_server_kind`, `watchdog`, and `agent*`/`agent_bin*`; unknown-key fixtures exist. Message files are opaque text. | `config`: `[workspace]` then `main = zzz-example-1`<br>`meta`: `session_id=zzz-session` |
| `l-artifacts/L-END/specimens/<ARCHIVE-UUID>/<digest.md,events.jsonl,memo.tsv,messages/,meta>` and compact copied archive | archive specimen tree; 2 section specimens plus 1 compact copy | `digest.md` is plain Markdown/text; `events.jsonl` uses product events; `memo.tsv` is zero bytes in all three copies; `messages/` is a directory (empty in these specimens); `meta` is `key=value` text with archive keys such as `archive_version`, `archive_id`, `archive_id_origin`, `archived_at`, `source_session`, `source_session_id`, `source_mode`, `source_origin`, `source_layout`, `source_ae_version`, `source_goal`, `parent_archive_id`, git fields, `event_count`, event bounds, `handover_count`, `memo_topic_count`, `pending_request_count`, and agent fields. | `archive_id=zzz-archive`<br>`event_count=987654321` |
| `batch-c-artifacts/{MANIFEST.md,hook-patch/*}` and `l-artifacts/L-<SECTION>/MANIFEST.md` | Markdown/text; 1 batch manifest, 6 L manifests, 4 batch hook-patch files | Markdown manifests are section documents with headings, prose, and tables; they are not machine records. Hook patch directories contain README, unified diff, generator, and checksum files. | `# zzz-section manifest`<br>`| field | value |` |
| `l-artifacts/{_admissibility/equiv-*.txt,L-<SECTION>/ADMISSIBILITY-SHA256SUMS.txt,L-<SECTION>/HARNESS-SHA256SUMS.txt}` | plain text/checksum lists; 17 equivalence notes, 6 + 6 checksum ledgers | Equivalence notes are prose plus labelled comparison records. The two checksum families use the checksum schema below. | `equivalence=zzz-comparison<TAB>verdict=<verdict>` |
| `batch-c-artifacts/arms/D/<CASE>/out/*.{flockspy,xtrace}`, `twd-precursor/{a1,a2,a3}/ae-launch.{out,err}`, D helper `*.{pre-refresh,post-refresh}`, and template `*.NOTE` | plain text diagnostics; 26 `.flockspy`, 4 `.xtrace`, 3 `.out`, 3 `.err`, 2 `.pre-refresh`, 1 `.post-refresh`, 2 `.NOTE` | No shared record schema. `.flockspy` is empty or key/value process-lock tracing (`pid`, `ppid`, `enter`/`leave`, `rc`, `argv`); `.xtrace` is shell xtrace text; launch files are raw stdout/stderr; refresh files are helper snapshots; `.NOTE` files are labelled mutation annotations. | `pid=<PID> ppid=<PID> enter=2999-12-31T23:59:59Z argv=zzz-command` |
| `l-artifacts/L-COMPACT/arms/sigpipe/producer.stdout.firstline` | plain text; 1 | One captured first-line rendering; preserve as opaque text and byte length. | `zzz-line` |
| `l-artifacts/L-<SECTION>/harness-snapshot/fixtures/stop-result.*.harvested` and matching `_harness`/arm fixture paths | JSON object stored under a non-JSON suffix; 12 | All five keys are present: `action:string`, `actor:string`, `summary:string`, `target:string`, `ts:string`. | `{"action":"zzz-action","actor":"zzz-actor","summary":"zzz-summary","target":"zzz-target","ts":"2999-12-31T23:59:59Z"}` |
| `l-artifacts/L-<SECTION>/arms/<ARM>/**/*.diff` | unified-diff text; 100 | Diff files may be zero bytes. Non-empty files use ordinary unified-diff headers and hunks; the importer should preserve bytes, not parse captured values as schema fields. | `--- <PATH>`<br>`+++ <PATH>`<br>`@@ -1 +1 @@` |
| `l-artifacts/L-COMPACT/arms/<ARM>/recovery-exec-selected/<...>/sessions/.lifecycle.<LABEL>.lock` | zero-byte lock sentinel; 1 | Empty file; presence and relative path carry the state. | *(empty file)* |
| `l-artifacts/L-END/arms/launch-rerun/launch.main.sh.{0after-launch,1first-run,2after-first-run-pane-killed,3second-run,4after-second-run-pane-killed,5after-stop,6after-resume-rewrite}` | plain-text shell-script snapshots; 7 | Script bytes at lifecycle checkpoints; no record schema. Preserve each suffix as a checkpoint label. | `#!/bin/sh`<br>`echo zzz-example-1` |

### Checksums and source files

| path pattern | format / count | schema | synthetic example |
|---|---|---|---|
| `**/SHA256SUMS.txt` | GNU-style checksum text; 13 batch + 128 L | One record per line: 64-hex digest, whitespace, relative path. The digest is a value, but the importer should treat it as opaque text. | `0000000000000000000000000000000000000000000000000000000000000000  zzz-example-1` |
| `l-artifacts/L-<SECTION>/{ADMISSIBILITY-SHA256SUMS.txt,HARNESS-SHA256SUMS.txt}` | same checksum text; 6 of each | Same one-line checksum record; section-level scope differs from arm-level `SHA256SUMS.txt`. | `0000000000000000000000000000000000000000000000000000000000000000  zzz-example-2` |
| `batch-c-artifacts/**/{*.sh,*.py,*.patch,*.c}` and `l-artifacts/**/{*.sh,*.py,*.patch,*.sed}` | source text; batch 110/45/8/2, L 335/23/18/7 | Shell, Python, C, patch, and sed source snapshots. They have language syntax, not a corpus record schema; preserve mode and bytes where the parent manifest records them. | `#!/bin/sh\necho zzz-example-1` |
| `batch-c-artifacts/templates/date-shim/date`, `**/flock-spy`, `**/tmux-shim`, and other extensionless helpers | executable/plain text; 117 batch extensionless files and 4 L extensionless files | Script or helper bytes; no shared record schema. | `#!/bin/sh\nexit <rc>` |
| `batch-c-artifacts/templates/fixture-bytes/**/{backslash,cr,newline,quote,tab}.bin` | byte fixture; 5 | Opaque bytes (the `file` probe identifies these as ASCII/text bytes, not native binary objects). Newline/CR/TAB/backslash/quote cases are intentionally byte-sensitive. | `<bytes: 7a 7a 7a 00>` |

## Cross-section inconsistencies and importer traps

1. **No JSON manifest dialect exists.** The requested-looking `manifest.json` name is absent. Batch C uses five positional TSV columns; L uses a comment preamble plus seven headed manifest columns, with separate two-column `ABSENT` records.
2. **Manifest column order and meaning split.** Batch C is `entry_type, mode, sha256-or-dash, link_target-or-dash, relative_path`; L is `path, type, mode, nlink, size, link, sha256`. Do not reuse one decoder for both.
3. **Case index split.** Batch C has per-arm `CASES.tsv`, per-arm `ledger.tsv`, per-case `admissibility-ledger.txt`, and `consumers.tsv`. L has no `CASES.tsv`, `ledger.tsv`, or `consumers.tsv`; each L arm has `ARM.txt` plus operation records.
4. **Admissibility split.** Batch C records admissibility per case. L records section-level equivalence notes under `_admissibility/` and section checksum ledgers named `ADMISSIBILITY-SHA256SUMS.txt`.
5. **Arm metadata split.** L `ARM.txt` is a tab-separated key/value record with section-specific keys. Batch C has no `ARM.txt`; `case.txt` is key/value plus prose and carries template/clone metadata instead.
6. **Operation-result split.** Batch C puts captures under `out/<LABEL>.stdout|stderr|tmuxtrace` and indexes them in `consumers.tsv`. L uses sibling `<STEP>.invocation|rc|stdout|stderr` files and, in some sections, `.od`, stream-size, and at-cut siblings.
7. **RC table is not safely rectangular.** L’s one `rc-table.tsv` declares four columns, but two invocation strings contain unescaped tabs and therefore produce 5- or 7-field physical rows. A parser must preserve/join the invocation tail.
8. **Empty/absent representation differs.** Batch TSVs use `-` for non-applicable digest/link values and permit zero-byte text diffs/events. L manifest snapshots use `-`, while absent archive/worktree states use a two-field `ABSENT` row; archived `memo.tsv` payloads are zero bytes.
9. **Event envelope has section variants.** Batch T-WD event snapshots omit the session/slot routing facets used by routed product events. Fixture and L product events allow optional `ref`, `body_file`, and actor/target session/slot facets; mutation fixtures add unknown keys. Empty, partial-tail, and no-final-newline records occur.
10. **Trace-channel naming is L-only.** `barrier-order.tsv`, `hook-trace.tsv`, `trace-channels.txt`, and per-barrier `.stdout-at-cut`/`.stderr-at-cut` have no Batch C equivalent; Batch C instead has `PATH-CITES.tsv`, `case-schema.tsv`, and template fingerprint tables.
11. **Checksum layering differs.** Batch C has arm/template/hook/T-WD `SHA256SUMS.txt` files only. L adds section-root `ADMISSIBILITY-SHA256SUMS.txt` and `HARNESS-SHA256SUMS.txt` alongside arm checksums and specimen checksums.
12. **Naming is not shell-safe everywhere.** L contains one event-capture basename with shell metacharacters (an intentional hostile-name case); Batch C fixture/member names are ordinary path components. Import by directory entry, never by shell interpolation.
13. **Payload metadata split.** Batch fixture `meta` describes a live session and uses agent/layout/tmux keys. L archive `meta` uses archive/source/git/handover counters. Both are `key=value` text, but they are different schemas.
14. **Diagnostic suffix split.** Batch C uses `.flockspy`, `.xtrace`, `.NOTE`, and refresh checkpoint suffixes for helper/lock diagnostics. L uses `.firstline`, `.harvested`, `.diff`, and numbered launch-checkpoint suffixes. These are path-role-specific opaque captures, not one cross-section suffix dialect.

## Value-leak self-check and residual

Checks performed on this document before handoff:

* searched the document for long hexadecimal runs and absolute user-home paths;
* searched every document token against the two evidence trees for verbatim matches;
* replaced all examples with the synthetic vocabulary above and re-ran both searches;
* manually checked every example digest, path, timestamp, rc, session name, state word, and
  branch-like token.

The corpus-token search necessarily reports legitimate structural matches: field names,
suffixes, section names, schema keywords, path placeholders, the mode exemplar `0755`, and
the literal sentinels `-`/`ABSENT` described as syntax. Those are not captured output values.
The long-hex search finds only the exact 64-zero stand-in declared above; the absolute-path
search is empty.
No captured digest, pane byte, rc, absolute capture path, session identifier, goal, branch,
or timestamp is reproduced here.

Not checked: semantic equivalence of recorded outputs, whether a checksum is correct, or
whether a free-form text capture contains a value that is not delimited as a token. This
document is a schema inventory, not a redaction proof for the source corpus.
