#!/usr/bin/env python3
"""Verify the published phase-4 fixture fingerprints without writing files.

Exit status: 0 FRESH, 1 STALE, 2 MALFORMED, 3 DIRTY-SOURCE.
`--render` is an authoring aid: it writes a candidate TSV only to stdout.
"""
from __future__ import annotations

import argparse
import hashlib
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


CORPUS_ROOT_SHA256 = "802c882bca64453e33efce5351e43b5954ddecc3daed6c2b0b6c8833487b4e12"
SOURCE_ROOT = "docs/migration/evidence/batch-c-artifacts/templates"
ARTIFACT_REL = "docs/migration/evidence/p1-phase4-published-fingerprints.tsv"
ALGORITHM_REL = "docs/migration/evidence/p1-phase4-published-fingerprints-algorithm.md"
FORMAT = "p1-phase4-published-fingerprints-v1"
EXPECTED_MEMBERS = 70
CANONICAL_GRAMMAR = "kind NUL payload NUL relative_path NUL; entries sorted by raw relative-path bytes"
TREE_ID_ROLE = "reproducibility anchor and exec-coverage carrier"
CANONICAL_ROLE = "mode-free scratch-comparison grain the runner recomputes"
ROW_HEADER = "source_path\tentry_count\tgit_tree_id\tcanonical_sha256"


class FingerprintError(Exception):
    """Base class for errors that must be reported as a named outcome."""


class Malformed(FingerprintError):
    pass


class DirtySource(FingerprintError):
    pass


@dataclass(frozen=True)
class Fingerprint:
    source_path: str
    entry_count: int
    git_tree_id: str
    canonical_sha256: str


def run_git(repo: Path, *args: str, check: bool = True) -> bytes:
    result = subprocess.run(
        ["git", "-C", str(repo), *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    if check and result.returncode:
        command = "git " + " ".join(args)
        detail = result.stderr.decode("utf-8", "replace").strip()
        raise Malformed(f"{command} failed: {detail or 'no diagnostic'}")
    return result.stdout


def repo_root() -> Path:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    if result.returncode:
        detail = result.stderr.decode("utf-8", "replace").strip()
        raise Malformed(f"not inside a Git worktree: {detail or 'no diagnostic'}")
    return Path(result.stdout.decode("utf-8").strip())


def parse_ls_tree(raw: bytes) -> list[tuple[str, str, str, bytes]]:
    rows = []
    for record in raw.split(b"\0"):
        if not record:
            continue
        try:
            prefix, path = record.split(b"\t", 1)
            mode, kind, object_id = prefix.decode("ascii").split(" ")
        except (UnicodeDecodeError, ValueError) as exc:
            raise Malformed(f"unparseable git ls-tree record: {record!r}") from exc
        rows.append((mode, kind, object_id, path))
    return rows


def fixture_roots(repo: Path, treeish: str = "HEAD") -> list[bytes]:
    rows = parse_ls_tree(run_git(repo, "ls-tree", "-r", "-d", "-z", treeish, "--", SOURCE_ROOT))
    roots = sorted(path for mode, kind, _, path in rows if mode == "040000" and kind == "tree" and path.endswith(b"/fixture-bytes"))
    if not roots:
        raise Malformed("no published fixture-bytes roots exist in HEAD")
    return roots


def published_members(repo: Path, roots: list[bytes], treeish: str = "HEAD") -> list[str]:
    members: list[bytes] = []
    for root in roots:
        root_text = root.decode("utf-8")
        rows = parse_ls_tree(run_git(repo, "ls-tree", "-d", "-z", treeish, "--", root_text + "/"))
        members.extend(path for mode, kind, _, path in rows if mode == "040000" and kind == "tree")
    try:
        decoded = [member.decode("utf-8") for member in sorted(members)]
    except UnicodeDecodeError as exc:
        raise Malformed("published member path is not UTF-8 and cannot appear in the TSV") from exc
    if len(decoded) != len(set(decoded)):
        raise Malformed("published member enumeration contains duplicate paths")
    return decoded


def dirty_source(repo: Path, roots: list[bytes]) -> None:
    paths = [root.decode("utf-8") for root in roots]
    # `status` rather than `diff` sees staged, unstaged, and untracked paths.
    raw = run_git(
        repo,
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--ignored=matching",
        "--",
        *paths,
    )
    if raw:
        items = [entry.decode("utf-8", "backslashreplace") for entry in raw.split(b"\0") if entry]
        raise DirtySource("published fixture path differs from HEAD: " + "; ".join(items))


def blob_bytes(repo: Path, object_id: str) -> bytes:
    return run_git(repo, "cat-file", "blob", object_id)


def fingerprint_member(repo: Path, member: str, treeish: str = "HEAD") -> Fingerprint:
    tree_id = run_git(repo, "rev-parse", "--verify", f"{treeish}:{member}").decode("ascii").strip()
    if len(tree_id) != 40 or any(ch not in "0123456789abcdef" for ch in tree_id):
        raise Malformed(f"{treeish}:{member} did not resolve to a SHA-1 tree identity")
    prefix = member.encode("utf-8") + b"/"
    entries = parse_ls_tree(run_git(repo, "ls-tree", "-r", "-z", treeish, "--", member))
    manifest: list[tuple[bytes, bytes]] = []
    for mode, git_kind, object_id, full_path in entries:
        if git_kind != "blob" or not full_path.startswith(prefix):
            raise Malformed(f"unexpected leaf under {member}: mode={mode} kind={git_kind} path={full_path!r}")
        relative_path = full_path[len(prefix) :]
        if not relative_path or relative_path.startswith(b"/") or b"/../" in relative_path or relative_path == b"..":
            raise Malformed(f"non-normalized member path: {full_path!r}")
        content = blob_bytes(repo, object_id)
        if mode == "120000":
            kind, payload = b"symlink", content
        elif mode in {"100644", "100755"}:
            kind, payload = b"file", hashlib.sha256(content).hexdigest().encode("ascii")
        else:
            raise Malformed(f"unsupported tracked mode {mode} under {member}")
        manifest.append((relative_path, kind + b"\0" + payload + b"\0" + relative_path + b"\0"))
    manifest.sort(key=lambda item: item[0])
    digest = hashlib.sha256(b"".join(entry for _, entry in manifest)).hexdigest()
    return Fingerprint(member, len(manifest), tree_id, digest)


def derive(repo: Path, treeish: str = "HEAD", enforce_source_clean: bool = True) -> list[Fingerprint]:
    roots = fixture_roots(repo, treeish)
    if enforce_source_clean:
        dirty_source(repo, fixture_roots(repo))
    members = published_members(repo, roots, treeish)
    if len(members) != EXPECTED_MEMBERS:
        raise Malformed(f"published-member census is {len(members)}, expected {EXPECTED_MEMBERS}")
    return [fingerprint_member(repo, member, treeish) for member in members]


def working_blob_id(repo: Path, path: str) -> str:
    candidate = repo / path
    if not candidate.is_file():
        raise Malformed(f"algorithm file is absent: {path}")
    object_id = run_git(repo, "hash-object", "--", path).decode("ascii").strip()
    if len(object_id) != 40 or any(ch not in "0123456789abcdef" for ch in object_id):
        raise Malformed("git hash-object did not return a SHA-1 blob identity")
    return object_id


def render(repo: Path, rows: list[Fingerprint]) -> str:
    algorithm_blob = working_blob_id(repo, ALGORITHM_REL)
    source_commit = run_git(repo, "rev-parse", "--verify", "HEAD^{commit}").decode("ascii").strip()
    header = [
        f"# format\t{FORMAT}",
        f"# corpus_root_sha256\t{CORPUS_ROOT_SHA256}",
        f"# algorithm_path\t{ALGORITHM_REL}",
        f"# algorithm_git_blob\t{algorithm_blob}",
        f"# source_root\t{SOURCE_ROOT}",
        "# committed_treeish\tHEAD",
        f"# derived_from_commit\t{source_commit}",
        "# derived_from_commit_role\tvalue-derivation source only; artifact header may be newer",
        f"# member_count\t{EXPECTED_MEMBERS}",
        f"# git_tree_id_role\t{TREE_ID_ROLE}",
        f"# canonical_sha256_role\t{CANONICAL_ROLE}",
        f"# canonical_entry_grammar\t{CANONICAL_GRAMMAR}",
        "# directories\timplied by tracked leaf paths; not manifest entries",
        ROW_HEADER,
    ]
    body = [f"{row.source_path}\t{row.entry_count}\t{row.git_tree_id}\t{row.canonical_sha256}" for row in rows]
    return "\n".join(header + body) + "\n"


def parse_artifact(path: Path) -> tuple[dict[str, str], list[Fingerprint]]:
    try:
        raw = path.read_bytes()
        text = raw.decode("utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise Malformed(f"cannot read UTF-8 artifact {path}: {exc}") from exc
    if not text.endswith("\n"):
        raise Malformed("artifact lacks its required terminal newline")
    header: dict[str, str] = {}
    rows: list[Fingerprint] = []
    saw_columns = False
    for number, line in enumerate(text.splitlines(), 1):
        if not saw_columns:
            if line == ROW_HEADER:
                saw_columns = True
                continue
            if not line.startswith("# "):
                raise Malformed(f"line {number}: expected a header or column row")
            cells = line[2:].split("\t")
            if len(cells) != 2 or not all(cells):
                raise Malformed(f"line {number}: malformed header")
            key, value = cells
            if key in header:
                raise Malformed(f"line {number}: duplicate header {key}")
            header[key] = value
            continue
        cells = line.split("\t")
        if len(cells) != 4:
            raise Malformed(f"line {number}: expected four TSV cells")
        source_path, count_text, tree_id, canonical = cells
        try:
            count = int(count_text)
        except ValueError as exc:
            raise Malformed(f"line {number}: entry_count is not an integer") from exc
        if count < 0 or len(tree_id) != 40 or len(canonical) != 64:
            raise Malformed(f"line {number}: invalid count or digest width")
        if any(ch not in "0123456789abcdef" for ch in tree_id + canonical):
            raise Malformed(f"line {number}: digest is not lowercase hexadecimal")
        rows.append(Fingerprint(source_path, count, tree_id, canonical))
    if not saw_columns:
        raise Malformed("artifact has no required TSV header")
    required = {
        "format": FORMAT,
        "corpus_root_sha256": CORPUS_ROOT_SHA256,
        "algorithm_path": ALGORITHM_REL,
        "source_root": SOURCE_ROOT,
        "committed_treeish": "HEAD",
        "derived_from_commit_role": "value-derivation source only; artifact header may be newer",
        "member_count": str(EXPECTED_MEMBERS),
        "git_tree_id_role": TREE_ID_ROLE,
        "canonical_sha256_role": CANONICAL_ROLE,
        "canonical_entry_grammar": CANONICAL_GRAMMAR,
        "directories": "implied by tracked leaf paths; not manifest entries",
    }
    if set(header) != set(required) | {"algorithm_git_blob", "derived_from_commit"}:
        raise Malformed("artifact header keys do not match the fixed schema")
    for key, value in required.items():
        if header[key] != value:
            raise Malformed(f"artifact header {key!r} has an unexpected value")
    blob = header["algorithm_git_blob"]
    if len(blob) != 40 or any(ch not in "0123456789abcdef" for ch in blob):
        raise Malformed("artifact algorithm_git_blob is not a lowercase SHA-1")
    source_commit = header["derived_from_commit"]
    if len(source_commit) != 40 or any(ch not in "0123456789abcdef" for ch in source_commit):
        raise Malformed("artifact derived_from_commit is not a lowercase SHA-1")
    if len(rows) != EXPECTED_MEMBERS or len({row.source_path for row in rows}) != len(rows):
        raise Malformed("artifact does not contain exactly 70 unique member rows")
    return header, rows


def verify(repo: Path) -> tuple[str, list[str]]:
    # Refuse the run-time fixture source before consuming any published value.
    roots = fixture_roots(repo)
    dirty_source(repo, roots)
    header, expected_rows = parse_artifact(repo / ARTIFACT_REL)
    source_commit = header["derived_from_commit"]
    resolved_source_commit = run_git(repo, "rev-parse", "--verify", f"{source_commit}^{{commit}}")
    if resolved_source_commit.decode("ascii").strip() != source_commit:
        raise Malformed("artifact derived_from_commit does not resolve to its named commit")
    source_rows = derive(repo, source_commit, enforce_source_clean=False)
    actual_rows = derive(repo, enforce_source_clean=False)
    findings: list[str] = []
    try:
        committed_algorithm_blob = run_git(repo, "rev-parse", "--verify", f"HEAD:{ALGORITHM_REL}").decode("ascii").strip()
    except Malformed as exc:
        findings.append(f"algorithm blob cannot be resolved from HEAD: {exc}")
        committed_algorithm_blob = ""
    if header["algorithm_git_blob"] != committed_algorithm_blob:
        findings.append("algorithm blob differs from the artifact pin")
    if header["algorithm_git_blob"] != working_blob_id(repo, ALGORITHM_REL):
        findings.append("working algorithm bytes differ from the artifact pin")
    expected = {row.source_path: row for row in expected_rows}
    source_actual = {row.source_path: row for row in source_rows}
    actual = {row.source_path: row for row in actual_rows}
    findings.extend(compare_rows("artifact does not agree with its declared derivation commit", expected, source_actual))
    findings.extend(compare_rows("source fixture moved since derivation", expected, actual))
    return ("FRESH", []) if not findings else ("STALE", findings)


def compare_rows(label: str, expected: dict[str, Fingerprint], actual: dict[str, Fingerprint]) -> list[str]:
    findings = []
    for source_path in sorted(expected.keys() - actual.keys()):
        findings.append(f"{label}: member deleted: {source_path}")
    for source_path in sorted(actual.keys() - expected.keys()):
        findings.append(f"{label}: member added: {source_path}")
    for source_path in sorted(expected.keys() & actual.keys()):
        if expected[source_path] != actual[source_path]:
            findings.append(
                f"{label}: {source_path} expected "
                f"{expected[source_path].entry_count}/{expected[source_path].git_tree_id}/"
                f"{expected[source_path].canonical_sha256}, got "
                f"{actual[source_path].entry_count}/{actual[source_path].git_tree_id}/"
                f"{actual[source_path].canonical_sha256}"
            )
    return findings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--render", action="store_true", help="print a candidate artifact to stdout")
    args = parser.parse_args()
    try:
        repo = repo_root()
        if args.render:
            sys.stdout.write(render(repo, derive(repo)))
            return 0
        outcome, findings = verify(repo)
    except DirtySource as exc:
        print(f"DIRTY-SOURCE — {exc}")
        return 3
    except Malformed as exc:
        print(f"MALFORMED — {exc}")
        return 2
    if outcome == "FRESH":
        print(f"FRESH — {EXPECTED_MEMBERS} published members agree with committed HEAD")
        return 0
    print(f"STALE — {len(findings)} difference(s)")
    for finding in findings:
        print(f"  {finding}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
