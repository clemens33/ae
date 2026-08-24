#!/usr/bin/env python3
"""Red-prove published-fixture fingerprint properties in temporary Git clones.

Every seed gets a new clone.  The source checkout is read only.  A seed's
landing check is printed before its derived values are examined.
"""
from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from types import ModuleType


EVIDENCE = Path(__file__).resolve().parent
VERIFIER_NAME = "verify-published-fingerprints.py"


def git(repo: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(
        ["git", "-C", str(repo), *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    if check and result.returncode:
        detail = result.stderr.decode("utf-8", "replace").strip()
        raise RuntimeError(f"git {' '.join(args)} failed: {detail or 'no diagnostic'}")
    return result


def source_repo() -> Path:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    if result.returncode:
        raise RuntimeError("red proof must run inside a Git worktree")
    return Path(result.stdout.decode("utf-8").strip())


def load_verifier(repo: Path) -> ModuleType:
    path = repo / EVIDENCE.relative_to(source_repo()) / VERIFIER_NAME
    spec = importlib.util.spec_from_file_location("published_fingerprint_verifier", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load verifier from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def clone(source: Path, scratch: Path, label: str) -> Path:
    target = scratch / label
    subprocess.run(
        ["git", "clone", "--quiet", "--no-hardlinks", str(source), str(target)], check=True
    )
    git(target, "config", "user.name", "published-fingerprint-redproof")
    git(target, "config", "user.email", "published-fingerprint-redproof@example.invalid")
    return target


def first_regular_file(module: ModuleType, repo: Path, member: str) -> Path:
    for mode, kind, _, full_path in module.parse_ls_tree(
        module.run_git(repo, "ls-tree", "-r", "-z", "HEAD", "--", member)
    ):
        if mode == "100644" and kind == "blob":
            return repo / full_path.decode("utf-8")
    raise RuntimeError(f"no non-executable regular file found in {member}")


def fingerprints(module: ModuleType, repo: Path) -> dict[str, object]:
    return {row.source_path: row for row in module.derive(repo)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def landed(repo: Path, target: Path, staged: bool = False) -> None:
    args = ("diff", "--cached", "--quiet", "--", str(target.relative_to(repo))) if staged else (
        "diff",
        "--quiet",
        "HEAD^",
        "HEAD",
        "--",
        str(target.relative_to(repo)),
    )
    require(git(repo, *args, check=False).returncode == 1, f"seed did not land: {target}")


def assert_movement(name: str, before: object, after: object, tree: bool, canonical: bool) -> None:
    require((before.git_tree_id != after.git_tree_id) == tree, f"{name}: unexpected tree-id movement")
    require(
        (before.canonical_sha256 != after.canonical_sha256) == canonical,
        f"{name}: unexpected canonical-SHA-256 movement",
    )
    print(f"PASS {name} — tree_moved={tree} canonical_moved={canonical}")


def committed_seed(
    source: Path,
    scratch: Path,
    name: str,
    mutate,
    expected_tree: bool,
    expected_canonical: bool,
) -> None:
    repo = clone(source, scratch, name)
    module = load_verifier(repo)
    member = module.published_members(repo, module.fixture_roots(repo))[0]
    before = fingerprints(module, repo)[member]
    target = mutate(repo, module, member)
    git(repo, "add", "--", str(target.relative_to(repo)))
    git(repo, "commit", "-qm", f"redproof {name}")
    landed(repo, target)
    print(f"LANDING VERIFIED {name}")
    after = fingerprints(module, repo)[member]
    assert_movement(name, before, after, expected_tree, expected_canonical)


def nonexec_chmod_seed(source: Path, scratch: Path) -> None:
    repo = clone(source, scratch, "chmod-nonexec")
    module = load_verifier(repo)
    member = module.published_members(repo, module.fixture_roots(repo))[0]
    before = fingerprints(module, repo)[member]
    target = first_regular_file(module, repo, member)
    os.chmod(target, 0o400)
    require((target.stat().st_mode & 0o777) == 0o400, "non-exec chmod did not land")
    require(git(repo, "diff", "--quiet", "HEAD", "--", str(target.relative_to(repo)), check=False).returncode == 0,
            "non-exec chmod unexpectedly changed Git's tracked tree")
    print("LANDING VERIFIED chmod-nonexec")
    after = fingerprints(module, repo)[member]
    assert_movement("chmod-nonexec", before, after, False, False)


def dirty_existing_seed(source: Path, scratch: Path, staged: bool) -> None:
    name = "dirty-staged-existing" if staged else "dirty-unstaged-existing"
    repo = clone(source, scratch, name)
    module = load_verifier(repo)
    member = module.published_members(repo, module.fixture_roots(repo))[0]
    target = first_regular_file(module, repo, member)
    target.write_bytes(target.read_bytes() + b"\nredproof dirty source\n")
    if staged:
        git(repo, "add", "--", str(target.relative_to(repo)))
        landed(repo, target, staged=True)
    else:
        require(git(repo, "diff", "--quiet", "--", str(target.relative_to(repo)), check=False).returncode == 1,
                "unstaged dirty seed did not land")
    print(f"LANDING VERIFIED {name}")
    try:
        module.derive(repo)
    except module.DirtySource:
        print(f"PASS {name} — DIRTY-SOURCE refused derivation")
        return
    raise RuntimeError(f"{name}: derivation emitted a value despite dirty source")


def dirty_new_member_seed(source: Path, scratch: Path) -> None:
    name = "dirty-index-added-member"
    repo = clone(source, scratch, name)
    module = load_verifier(repo)
    root = module.fixture_roots(repo)[0].decode("utf-8")
    member = root + "/redproof-index-added-member"
    target = repo / member / "added.txt"
    target.parent.mkdir()
    target.write_text("redproof staged member\n", encoding="utf-8")
    git(repo, "add", "--", str(target.relative_to(repo)))
    landed(repo, target, staged=True)
    require(
        git(repo, "rev-parse", "--verify", f"HEAD:{member}", check=False).returncode != 0,
        "index-added member unexpectedly has a HEAD tree identity",
    )
    print(f"LANDING VERIFIED {name} — no HEAD:path identity")
    try:
        module.derive(repo)
    except module.DirtySource:
        print(f"PASS {name} — DIRTY-SOURCE guard, not rev-parse failure, refused derivation")
        return
    raise RuntimeError(f"{name}: derivation emitted a value despite indexed new member")


def symlink_specimens(module: ModuleType, repo: Path) -> list[tuple[str, bytes]]:
    found: list[tuple[str, bytes]] = []
    for member in module.published_members(repo, module.fixture_roots(repo)):
        for mode, kind, _, full_path in module.parse_ls_tree(
            module.run_git(repo, "ls-tree", "-r", "-z", "HEAD", "--", member)
        ):
            if mode == "120000" and kind == "blob":
                found.append((member, full_path))
    return found


def symlink_retarget_seed(source: Path, scratch: Path, specimen: tuple[str, bytes]) -> None:
    name = "symlink-retarget"
    repo = clone(source, scratch, name)
    module = load_verifier(repo)
    member, full_path = specimen
    target = repo / full_path.decode("utf-8")
    before = fingerprints(module, repo)[member]
    old_target = os.readlink(target)
    target.unlink()
    os.symlink(old_target + ".redproof-retarget", target)
    git(repo, "add", "--", str(target.relative_to(repo)))
    git(repo, "commit", "-qm", "redproof symlink retarget")
    landed(repo, target)
    print("LANDING VERIFIED symlink-retarget")
    after = fingerprints(module, repo)[member]
    assert_movement("symlink-retarget", before, after, True, True)


def synthetic_symlink_retarget_seed(source: Path, scratch: Path) -> None:
    """Exercise the grammar branch without misreporting a published specimen."""
    repo = clone(source, scratch, "synthetic-symlink-retarget")
    module = load_verifier(repo)
    member = module.published_members(repo, module.fixture_roots(repo))[0]
    target = repo / member / "redproof-synthetic-link"
    before = fingerprints(module, repo)[member]
    os.symlink("synthetic-target-a", target)
    git(repo, "add", "--", str(target.relative_to(repo)))
    git(repo, "commit", "-qm", "redproof synthetic symlink addition")
    landed(repo, target)
    print("LANDING VERIFIED synthetic-symlink-addition")
    with_link = fingerprints(module, repo)[member]
    assert_movement("synthetic-symlink-addition", before, with_link, True, True)
    target.unlink()
    os.symlink("synthetic-target-b", target)
    git(repo, "add", "--", str(target.relative_to(repo)))
    git(repo, "commit", "-qm", "redproof synthetic symlink retarget")
    landed(repo, target)
    print("LANDING VERIFIED synthetic-symlink-retarget")
    after = fingerprints(module, repo)[member]
    assert_movement("synthetic-symlink-retarget", with_link, after, True, True)


def main() -> int:
    source = source_repo()
    module = load_verifier(source)
    specimens = symlink_specimens(module, source)
    print(f"SYMLINK SPECIMENS {len(specimens)}")
    with tempfile.TemporaryDirectory(prefix="p1-published-fingerprints-") as temporary:
        scratch = Path(temporary)
        committed_seed(
            source,
            scratch,
            "content-mutation",
            lambda repo, mod, member: _append_to_first_regular(repo, mod, member),
            True,
            True,
        )
        committed_seed(
            source,
            scratch,
            "path-addition",
            lambda repo, mod, member: _add_path(repo, member),
            True,
            True,
        )
        committed_seed(
            source,
            scratch,
            "path-deletion",
            lambda repo, mod, member: _delete_first_regular(repo, mod, member),
            True,
            True,
        )
        nonexec_chmod_seed(source, scratch)
        committed_seed(
            source,
            scratch,
            "chmod-exec-bit",
            lambda repo, mod, member: _set_first_regular_executable(repo, mod, member),
            True,
            False,
        )
        dirty_existing_seed(source, scratch, staged=True)
        dirty_existing_seed(source, scratch, staged=False)
        dirty_new_member_seed(source, scratch)
        if specimens:
            symlink_retarget_seed(source, scratch, specimens[0])
        else:
            print("SKIP symlink-retarget — no published symlink specimen exists")
        synthetic_symlink_retarget_seed(source, scratch)
    print("RED-PROOFS FRESH — all executed seeds passed")
    return 0


def _append_to_first_regular(repo: Path, module: ModuleType, member: str) -> Path:
    target = first_regular_file(module, repo, member)
    target.write_bytes(target.read_bytes() + b"\nredproof content mutation\n")
    return target


def _add_path(repo: Path, member: str) -> Path:
    target = repo / member / "redproof-path-addition.txt"
    target.write_text("redproof path addition\n", encoding="utf-8")
    return target


def _delete_first_regular(repo: Path, module: ModuleType, member: str) -> Path:
    target = first_regular_file(module, repo, member)
    target.unlink()
    return target


def _set_first_regular_executable(repo: Path, module: ModuleType, member: str) -> Path:
    target = first_regular_file(module, repo, member)
    os.chmod(target, target.stat().st_mode | 0o100)
    return target


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, subprocess.CalledProcessError) as exc:
        print(f"RED-PROOFS FAILED — {exc}", file=sys.stderr)
        sys.exit(1)
