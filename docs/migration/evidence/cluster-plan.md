# Probe cluster plan — closing the 322 (seat-gated; refreshed from the manifest)

Built ONLY from the regenerated ratification-critical.md labels (322 CRITICAL lines,
machine-counted 2026-08-20 post slice-1d: SC-510e/f added; header and lines agree,
manifest↔canonical SET_EXTRA=0/SET_MISSING=0). Rules inherited: dependency closure (a
critical SC row pulls its interpreting D record/mechanism into the same batch unless
independently proven); probe designs are VALUE-BLIND (manipulation/barriers + captured
stdout/stderr/rc/files/tmux; expected values omitted — seats classify); deterministic
per the closure-map gate (no timing races, no live-model queries); every capture is a
CANDIDATE observation until seat acceptance (never a builder oracle before scoped
ratification — P1-start condition 2).

Family spread of the 322 (id-range convention, machine-joined): S9=71, S10=58,
S6=45, S15=25, S3=24, S1=20, S5=18, S14=13, S13=12, S8=8, S7=5, S2=3, S12=3, S4=2,
S11=2, +13 D records (D01-D04 read side; D14b half; D24 negative pointer;
D25/27/30x gates; D28c).

## Global rule — instrumentation admissibility (B0 preflight ruling, binding for EVERY batch)

Frozen code has no hooks, so deterministic cuts use minimal instrumentation under this
contract: an exact 72c7293 copy plus ONE hook-only patch, with the patch and its hash
recorded in the run manifest; the INACTIVE hook must be byte/rc/file/tmux-equivalent
to the unmodified control (any inactive divergence INVALIDATES the run); an ACTIVE
hook only blocks/emits its barrier — the CONTROLLER performs the named writer-shaped
mutation; PATH shims delegate-and-log per the date-shim contract. Inactive equivalence
is proven PER hook/shim fixture, not by one token control: a tmux wrapper that
snapshots rows must match unwrapped output on the same stable topology before its
active barrier is admissible. Equivalence and manifests speak about PRODUCT-VISIBLE
paths only: delegate-and-log shims and hooks necessarily create trace bytes, so
harness artifacts (barrier/log files) are segregated from product-state equivalence
and separately hashed/captured. This section is the normative home; batch designs
CITE it and add only batch-local specifics.

## Batch 0 — bespoke designs (seat-gated FIRST, as ruled)

SC-507b (fingerprint barriers around preview render), SC-511c (frozen consumer
fixtures + explicit compatibility outcomes), D01–D04 concurrency (named mutation
barriers, before/after input fingerprints, tmux snapshots, repeated assertions),
SC-1208 (constructed argv/config/context capture vs delivered user-input artifact).
Four designs, each individually approved by both seats before running.

## Batch C — read-side cluster (UNBLOCKS P1 FIXTURES; runs first after B0)

One deterministic fixture-session build + reader invocations under barriers:
- covers S1 list/status/next rows (SC-016x/017x/019/020x IS), S6 SC-509/510x/511a-b,
  SC-506 (bad-session degradation arm), SC-513x/514 exits, S14 SC-1306a-e snapshot
  cuts, D01–D04 records (with Batch 0's concurrency designs), SC-100/101/102x modes.
- ~55 observations from ONE operation cluster; its artifacts are the golden-corpus
  candidates (fixture dirs + captured outputs, seat-accepted before corpus freeze).

## Batch L — lifecycle destructive cluster (S9's 71)

Ordered runs in isolated AE_HOME sandboxes, one scenario tree per operation:
end/archive (SC-816–826, 818x proofs incl. symlink/claim/validate arms), purge
inversion (810x + 818x), stop matrix (835a–h + 839a-e with a planted C1–C5 failure
each), compact (827–831, 836/837, 500-series stdout/exit contracts — S6's compact rows
ride this cluster), from/lineage (822–825), rename/transfer residue probes (832x/833x,
1303/1304a-d) with crash-cut faults at the census-named boundaries.
Estimated 4 scenario trees; covers S9 + the S6 compact/exit remainder (~100 ids).

## Batch T — daemon/telegram cluster (S10's 57)

The auth trust chain (943/944abc/946–951/960–962) via a fake-updates harness (fixture
updates injected, no live Telegram); watchdog branch semantics (905–919) via a
fixture-session watchdog run with planted pane states; control-surface truths
(902–904, 926–929 IS confirmations); store/failure semantics (939x, 958, 966–971)
with fault hooks. D25/D27/D30c flip-gate lanes close on DR-002/003 citations (already
written — normative lane), leaving only their IS arms here. D28c status probe.

## Batch H — helper/CLI surface cluster (S3's 24 + S15's 25)

One fixture session, scripted helper invocations: signatures/refusals (211a–l probes,
212x IS confirmations), slot routing (209a-d IS), delivery invariants (201–205 with a
planted shell-pane and busy-input arm), say/memo/goal/state surfaces. Env matrix:
S15's numeric defaults + malformed classes + 1410x/1411x/1412x per-variable probes as
one environment-sweep script (each variable one sub-arm, captured independently).

## Batch F — formats/config/platform residue (S4=2, S5=8, S7=5, S8=7, S12=3, S13=12,
S2 remainder, S11=2, S14 remainder)

Small targeted probes: config grammar arms (300x/307), format round-trips (400x/403),
tmux literalization (600/601 with hostile names in a sandbox), adapter IS
confirmations (704x/705/706 against matrix fixtures), identity boundaries (S13 IS arms
— spawn/roster/interpolation probes with hostile names), installer/doctor (1000x),
platform rows where runnable (1102/1103/1104).

## D24 — negative-evidence pointer (no probe)

The scoped absence assertion over the census writer enumeration (CODE kind), per the
census-audit gate ruling; transcribed by the seats at the marks pass.

## Execution shape

Batch 0 designs (seats) → C (first worker, corpus-feeding) → L, T, H in parallel
(separate sandboxes, one worker each) → F sweep. Every batch: worker runs value-blind
per approved design, delivers captures + provenance (commit, environment, rc); seats
classify observations against rows; contradictions become bucket-3/DR reopenings —
measurement never rewrites SHOULD. Protected gate for all batches: the #81
ratification comments cite the resulting observed manifest.
