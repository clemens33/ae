# Provenance note — commit `25c6a00`

**What happened.** `25c6a00` is titled *"contract: rename identity defects — SC-1303 to
bucket 3 (#103), SC-832d/e (#102)"* and its message describes only contract and surface
changes. It also contains **1305 files of `docs/migration/evidence/t-artifacts/`** — the
entire T-100 capture (six arms answering #100), produced by opus5:lexec and delivered
minutes before the commit was made.

**Cause.** The commit was staged with `git add -A docs/migration`. lexec's T-100 tree had
landed in the working directory between my last status check and the commit, and the
recursive add swept it in. The commit message was written against what I *intended* to
commit, not against what was staged.

**Why it is not being rewritten.** `rust-rewrite` is pushed and shared with several agents
who have it checked out. Rewriting shared history to fix an attribution error costs more
than it repairs. The record is corrected here instead, which is where the first instance
of this was corrected too.

**Attribution, stated plainly.** Everything under `docs/migration/evidence/t-artifacts/` in
`25c6a00` is **lexec's T-100 work**, not part of the contract change the message describes.
It comprises: 6 arms, 1297 files under `T-100/`, the `_harness/` that produced them, 7
checksum files (all verified clean), and `TABLES.txt` — which was independently verified to
regenerate byte-identical from the arm artifacts before acceptance.

---

## This is the SECOND instance, so the remedy is a mechanism and not a resolution

The first was `e3ace55`, recorded in `commit-note-e3ace55.md`: the same `git add -A
docs/migration` swept lexec's 132-file checksum fix into a commit about SC-818e. I wrote
that note, resolved to be careful, and then did it again within the same session.

That is the pattern I told lexec to stop doing three hours earlier — after their fifth
report-vs-tree discrepancy, I wrote to them: *"You did not promise to be more careful — you
removed the step where care was required. Mechanisms survive fatigue in a way intentions
do not."* I then failed to apply it to myself.

**The mechanism, in force from now on for this tree:**

1. **Never `git add -A <dir>` on the shared evidence tree.** Stage explicit paths.
2. When a recursive add is genuinely wanted, **read the staged set before committing**:
   ```
   git diff --cached --name-only | cut -d/ -f1-4 | sort -u
   ```
   and confirm every top-level area shown is one the message describes.
3. A worker's delivery landing in the tree between a status check and a commit is **normal**
   in a concurrent workspace, not exceptional. The staging step must be robust to it, which
   means it cannot rely on my model of what is dirty.

**The general lesson, which is the same one this whole migration keeps teaching:** a check
that runs in the same breath as the action it guards cannot gate it. The duplicate-id
pre-check in `25c6a00`'s own session printed its warning *in the same shell invocation* as
the edit it was meant to prevent, so the edit ran anyway and duplicated `SC-832b`/`SC-832c`.
A guard must be able to *stop* something, or it is decoration.

---

## Third instance (2026-08-21) — and the mechanism above was NOT enough

Near-miss, caught before committing. Another agent had a file **staged in the shared index**
(`A  docs/migration/p1-phase2-gate-completeness.md`) while I staged my own path. A plain
`git commit` would have swept their in-progress work into my commit — the same outcome as the
two `git add -A` incidents, arrived at by a completely different route.

**The remedy recorded above does not cover this.** "Stage explicit paths" assumes *I* am the
only one who stages. In a shared checkout with concurrent agents, the index is shared state,
and my careful `git add src/inventory.rs` sat on top of somebody else's careful
`git add <their file>`. Doing my half correctly was not sufficient, because correctness here is
a property of the whole index rather than of my contribution to it.

**The mechanism that does cover it, in force from now on:**

```
git commit -m "..." -- <path> [<path>...]
```

A **path-limited commit** ignores the rest of the index entirely. It cannot pick up a file
somebody else staged, it does not depend on the index being clean when I arrive, and it does
not require me to know what else is in flight.

**The general lesson, which is the one worth carrying past this repo:** a remedy that assumes
you are the only actor on a piece of shared state is not a remedy — it is a habit that happens
to work while you are alone. The first two instances taught "be explicit about what you add";
the correct lesson was "do not let *what you add* be the thing that decides what gets
committed."
