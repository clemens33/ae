# Quota history

Each session's watchdog appends evidence under its own
`sessions/<session>/quota-history/YYYY-MM-DD.jsonl`, using the UTC collection
date. The existing quota cadence controls writes: quota off or a zero cadence
writes nothing. A recovery refresh writes too when the cadence is enabled;
it may share a collection timestamp with the normal pass in that cycle.
History supplies display PACE; advisories and checkpoint asks use current quota
facts and do not read history.

The directory is 0700; date files and the nonblocking `.lock` are 0600. Broader
existing permissions refuse access. A failure leaves quota accounting running
and produces at most one stderr note per UTC day. Ending a session deletes its
history with the session directory. Renaming preserves admission by the canonical
meta `session_id`; the producer name is data, never the admission key.

## Version 1 rows

Every line has `v: 1`, `kind` (`sample` or `workload`), `producer`, `incarnation`
(the session UUID), `collected_at` (epoch seconds) and
`scope: {tool, source, account}`. Source and account are canonical local paths;
account is the client home or null, never a vendor account ID.

A sample also has `bucket`, optional `qualifier` and `window_minutes`, `resets_at`,
vendor `observed_at`, `raw` (the original numeric string), `effective` (six decimal
places), `derivation` (`raw`, `resets`, `unlimited`, `spend-cap`) and
`policy: {version: 1, manual_resets, credits, spend_cap}`. Credits is one of
`unreported`, `unlimited`, `available`, `available-unknown`, `exhausted`.
Effective comes from the quota module's existing derivation. Samples are written
only when the series, vendor clock or policy changed since this daemon last
successfully wrote them; a restart may write the same evidence again.

A workload has `observed_at` (or null), `unscoped` and `seats`. Its clock is the
oldest successful eligible seat capture in that scope from the prior cycle.
Each seat has `slot`, `agent`, optional launch UUID, `observed` (`busy`, `idle`,
`unknown`), optional current `declared` state, `model` and `effort`. State older
than 900 seconds is unknown. No capture restamps a missing pane. Only recorded
Claude/Codex identities join seats to scopes; unproven seats contribute to the
unscoped count, repeated on each scope row rather than summed across scopes.
Model and effort require a matching launch; profile labels supply neither.

Optional fields accept null or absence; rendering uses null. Unknown keys and
other-kind fields are inert. Duplicate known keys, unknown versions, invalid
types and invalid bounds refuse a line. No transcript, prompt, token, credential
or conversation ID is stored.

## Bounds and reads

A batch has at most 256 rows and 1 MiB; each rendered line has at most 65536 bytes,
including wide seats and rendered nulls. Any oversized row refuses the batch.
A pass beyond the batch bounds writes nothing; repeated oversized passes leave
a history gap until the pass fits.
A file has at most 16 MiB; a history directory has at most 128 entries, including
the lock and unknown names. Retention deletes only regular date-named files older
than today minus seven days. Future files left by a backward clock step stay;
on-disk storage is today plus seven dates times 16 MiB, plus those future files,
within the entry bound. A partial tail is sealed before appending, never joined
to the next JSON row. Full files are never truncated or rewritten.

The API `quota::history::{parse, render, append, read_fleet}`
includes the read-only `read_fleet`, exposing samples, workload, aligned indices, segments and
skipped counters. The hidden pure `fold` entry also supports fuzzing. A read
visits at most 128 session directories, 128 entries per history directory, newest
eight retained dates per session (an 8 x 16 MiB derived bound), 64 MiB across
the fleet and 65536 admitted rows before deduplication. Truncation is counted.
There is no separate session byte cap. Missing evidence remains absent, never
zero busy. Workload alignment uses both vendor and state clocks within 900 seconds.

Samples deduplicate across producers by canonical scope, window instance, policy
and vendor clock; repeats are never added. Segments are computed per incarnation
and scope before deduplication: window changes precede policy changes, then raw
drops. Drops up to 2.0 percentage points are corrections; the comparison allows
1e-9 pp for floating point subtraction roundoff. Larger drops start a segment.
Independent producers can annotate one canonical sample differently; all distinct
annotations remain. A new source or account starts a separate track silently.

A malformed complete line is skipped and counted, a partial last line is counted,
and valid neighbours survive. A foreign incarnation invalidates its whole file.
Links, nonregular, oversized and unreadable files refuse per file. File opens use
no-follow flags and verify held/name inode identity. As with other local state
doors, a same-user swap of a parent directory during access remains outside that
proof; history is bounded evidence, not an adversarial-user audit log.

## PACE from stored samples

`ae quota`, `ae quota --profiles` and the quota dialog join raw samples by tool,
canonical source, account home, bucket, qualifier and window length. Claude gains
a rate once its watchdogs have collected enough history. Collection gaps can
leave PACE unknown; the reader never invents missing observations or replaces
the displayed endpoint.

Stored samples join existing rollout points. With any matching stored point at
or before the displayed observation, the union looks back a fifth of the window,
clamped to 30 minutes through seven days (33 h 36 min for a weekly window).
Maturity stays at the existing threshold: four hours for weekly, 20 minutes for
a five-hour window. A reset timestamp more than 120 seconds from the displayed
endpoint's resets the run, using the same instance tolerance as rollout PACE;
nearby timestamps never chain that tolerance. Consecutive drops greater than
2.0 percentage points (plus 1e-9 roundoff allowance) also restart the run.
Different sources and accounts never join. Policy segment annotations in
the history report remain data: PACE ignores them because both raw endpoints
are judged under the current policy, including manual resets and credits.
Rollout evidence still comes from the reader's already spaced tail; a boundary
missing from those retained rollout points cannot be recovered by the union.

After run and lookback filtering, selection keeps the oldest eligible point,
the exact displayed endpoint and at most 30 interior points. Interior spacing
is at least the greater of five minutes and lookback / 32; the two endpoints
are exempt from neighbour spacing. Repeated producers never add usage. Stored
points at or after the displayed timestamp cannot displace its value.

Without matching stored evidence, rollout PACE retains its existing 12-hour
lookback and 0.5-point rounding rule. The first matching stored point switches
that window to the union rule, so its displayed rate can change at upgrade.
USED, OBSERVED, status and current quota decisions remain unchanged.

A command or one-shot dialog reads the bounded fleet once. The app Reader holds
compact raw history for 60 seconds across its settings refreshes, including
empty and failed reads; a different canonical state root or backward clock
forces a new read. This adds at most 60 seconds of stored-evidence lag. Current
row trust and policy are judged on every render. Workload and aligned rows are
discarded from this cache; no two-second watchdog accounting tick reads history.
