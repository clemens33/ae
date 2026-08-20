# The evidence gate — canonical copy

Run it from the repo root against ANY evidence tree:

```
bash docs/migration/evidence/gate/manifest-tree-gate.sh docs/migration/evidence/batch-c-artifacts
bash docs/migration/evidence/gate/manifest-tree-gate.sh docs/migration/evidence/batch-h-artifacts
```

**This is the copy to run.** The per-arm `harness/` directories inside each arm hold the
version that arm ran under — a provenance snapshot, deliberately frozen with its captures
— and an older snapshot will report violations the current gate does not, because the
kinds and layouts it knows about have grown since. A reader reproducing a gate result must
run THIS copy; reproducing it from an arm snapshot measures that arm's era, not today's
tree.

Four checks: citation resolution over the tree's own MANIFEST, per-case artifact schema
plus a content-bound case index, SHA256SUMS coverage and verification, and committed-bytes
fidelity (what a fresh clone yields) with the repo prefix DERIVED from the tree under audit.
