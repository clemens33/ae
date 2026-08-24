# P1 phase 4 published-fixture fingerprints — algorithm v1

This is the named algorithm for
`p1-phase4-published-fingerprints.tsv`.  It replaces the ambiguous historical
`clone_fingerprint` referent with two deliberately different columns over the
published fixture projection.

## Scope and input

The published members are exactly the direct tree children of
`docs/migration/evidence/batch-c-artifacts/templates/*/fixture-bytes/` in the
committed `HEAD` tree.  A member is not a private producer-store member, an arm,
or a working-tree directory.  Directories below a member are implied by the
tracked leaf paths; they are not manifest entries.

Derivation refuses with `DIRTY-SOURCE` when any path below a published
`fixture-bytes` root differs between `HEAD` and the working tree, whether the
change is staged, unstaged, or untracked.  Dirt elsewhere in the repository is
out of scope.  The guard matters even though the value below is derived from
`HEAD`: phase 4 materialises fixtures from the working tree, so publishing a
`HEAD` value while a fixture is locally changed would recreate the old
wrong-referent defect.  Derivation never uses `git write-tree`, the index tree,
working-tree `stat`, or working-tree file contents.  For a member path `P`, its
reproducibility anchor is obtained only from the committed tree:

```text
git rev-parse --verify HEAD:P
```

This yields `git_tree_id`, Git's tree identity.  It intentionally carries
Git's executable-bit distinction (`100644` versus `100755`).

## Canonical SHA-256

For every tracked leaf returned by `git ls-tree -r -z HEAD -- P`, derive a
relative path by removing `P/`.  Paths are already Git's normalized forward
slash paths and are ordered by their raw byte sequence.  The only accepted
leaf modes are `100644`, `100755`, and `120000`.

`canonical_sha256` uses the following mode-free entry grammar.  `NUL` means
one zero byte, and the displayed concatenation is byte concatenation:

```text
entry = kind NUL payload NUL relative_path NUL
kind  = "file" | "symlink"
payload(file)    = lowercase SHA-256 hex of the blob's frozen bytes
payload(symlink) = raw bytes of the symlink-target blob
manifest = entry_1 || entry_2 || ... || entry_n
canonical_sha256 = SHA-256(manifest)
```

The `file`/`symlink` kind comes from Git mode (`120000` is `symlink`; both
`100644` and `100755` are `file`).  Blob bytes and symlink targets come from
Git objects named by `ls-tree`, not from the checkout.  NUL field separators
make target bytes and paths unambiguous.  An empty member has SHA-256 of the
empty manifest.  `entry_count` is `n`.

The canonical column deliberately excludes **all** mode facts.  Therefore a
non-executable chmod (for example `0400` to `0644`) moves neither column;
flipping the executable bit moves `git_tree_id` but not `canonical_sha256`.
Content edits, path additions, path deletions, and symlink retargets move both
columns.  This split is diagnostic, not disagreement: outside the executable
bit, a divergence between the Git tree identity and this canonical accounting
is an implementation defect.

## Artifact and verification

The TSV header pins the frozen corpus root digest and this document's Git blob
identity.  It carries one row per published member:

```text
source_path<TAB>entry_count<TAB>git_tree_id<TAB>canonical_sha256
```

`verify-published-fingerprints.py` re-enumerates all 70 direct members and
recomputes both columns from committed bytes.  It prints exactly one outcome
class: `FRESH` for exact agreement, `STALE` for syntactically valid records that
no longer agree with `HEAD`, `MALFORMED` for an artifact or input that cannot be
parsed, and `DIRTY-SOURCE` when a published fixture path is locally changed.
`DIRTY-SOURCE` is not malformed input: it tells the reader to commit or set
aside the fixture change, rather than hunt for corruption in the artifact.

`redproof-published-fingerprints.py` performs each mutation in a separate
temporary clone.  It verifies that every seed landed before reading the
property result, reports the published symlink-specimen count first, and skips
the retarget seed rather than claiming it ran when that count is zero.
