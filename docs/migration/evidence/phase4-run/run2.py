#!/usr/bin/env python3
"""Fail-closed diagnostic runner for Phase 4 Run 2.

This is deliberately a *run instrument*, not a product repair.  It separates
scheduled P1 rows from child executions, fingerprints every materialised
fixture before and after the portable permission step, and never turns an
unsupported comparison into a pass.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path


REPO = Path(__file__).resolve().parents[4]
RUN = Path(__file__).resolve().parent / "run2-results"
CORPUS = REPO / "docs/migration/evidence/batch-c-artifacts"
INV = REPO / "docs/migration/evidence/corpus/INVOCATIONS.tsv"
OBL = REPO / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
TEMPLATES = CORPUS / "templates"
AE = RUN / "build" / "release" / "ae"
PRODUCT_SUCCESSOR = "acb4f540e9d7fb0d5a70880f7aec883ffccb36bd"

INPUTS = {
    "contract": ("docs/migration/semantic-contract.md", "896d08ea3ac753095c04af17dfba92cd9d15fb38"),
    "invocations": ("docs/migration/evidence/corpus/INVOCATIONS.tsv", "035c5fab48cf04229daa9285457922d90563fabe"),
    "obligations": ("docs/migration/evidence/corpus/OBLIGATIONS.tsv", "44e06c29cc078e6933298139d204413966419d81"),
    "phase1_gate": ("docs/migration/p1-phase1-gate.md", "8e3c9ec0b031f4947260d4e0327bad562a10fdcd"),
    "phase2_gate": ("docs/migration/p1-phase2-gate.md", "29db943aa85319534301332052105ba16df03b4d"),
    "phase3_gate": ("docs/migration/p1-phase3-gate.md", "8cccbe44787d4ea6007ad9cf9d1cc83a3d03936c"),
    "open_choice_register": ("docs/migration/p1-phase4-open-choices.tsv", "2da4fb86933a6b8edee15fd61596d6f53fa6c550"),
    "comparison_projection": ("docs/migration/p1-phase4-comparison-projection.md", "c15087aa57a4f24e4ca773df6cafb60097492454"),
    "agent_health_manifest": ("docs/migration/p1-phase4-agent-health-manifest.tsv", "6927a58b30d0583def63fe491248b695b1b6f754"),
    "published_fingerprints": ("docs/migration/evidence/p1-phase4-published-fingerprints.tsv", "ad3dbb5d02df7d6879ff4536002496b1492862de"),
    "published_fingerprint_verifier": ("docs/migration/evidence/verify-published-fingerprints.py", "54dcc46251c5ea128e556b42cf309e123622869c"),
    "published_fingerprint_redproof": ("docs/migration/evidence/redproof-published-fingerprints.py", "1f604a4b3e75d00847e271d0188e46286fe1cdd2"),
    "contract_reconciliation": ("docs/migration/evidence/p1-phase4-contract-obligation-reconciliation.md", "6bf2e7f86c82ba15eb8479cff3b139ce708f15bd"),
    "contract_reconciliation_verifier": ("docs/migration/evidence/verify-contract-obligation-reconciliation.py", "4cc3eac1d4624062937bc86e65a57c889d6c5a30"),
    "contract_reconciliation_redproof": ("docs/migration/evidence/redproof-contract-obligation-reconciliation.py", "12d9af2ad01bf0ca73c9257a82afacd73397869c"),
    "open_choice_reconciliation": ("docs/migration/evidence/p1-phase4-open-choice-reconciliation.md", "a2232860608455e87cde22b3f37faf61084cc3c0"),
    "open_choice_reconciliation_verifier": ("docs/migration/evidence/verify-open-choice-reconciliation.py", "c78a1e84802551f167d49443f9ce08bd1cc90336"),
    "open_choice_reconciliation_redproof": ("docs/migration/evidence/redproof-open-choice-reconciliation.py", "6d66cd98cdeea9aeca6a2bb37e3f1ea63f90d19e"),
    "open_choice_occurrences": ("docs/migration/evidence/p1-phase4-open-choice-occurrences.tsv", "29c80d6bcd40b27d726157791abb6919655fd479"),
    "phase4_gate": ("docs/migration/p1-phase4-gate.md", "f31ece2ac40ed47077ab07f559ad8ab5ad97f6b0"),
}


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            digest.update(block)
    return digest.hexdigest()


def git(*args: str, check: bool = True) -> str:
    result = subprocess.run(["git", *args], cwd=REPO, capture_output=True, text=True)
    if check and result.returncode:
        raise RuntimeError(f"git {' '.join(args)}: {result.stderr.strip()}")
    return result.stdout.strip()


def git_rc(*args: str) -> int:
    return subprocess.run(["git", *args], cwd=REPO, capture_output=True).returncode


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def record(kind: str, command: list[str], result: subprocess.CompletedProcess[bytes]) -> None:
    RUN.mkdir(parents=True, exist_ok=True)
    extra = " ".join(command)
    with (RUN / "exec.log").open("a", encoding="utf-8") as sink:
        sink.write(f"{utc()}\t{kind}\trc={result.returncode}\t{extra}\n")


def checked(kind: str, command: list[str], cwd: Path | None = None, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(command, cwd=cwd or REPO, env=env, capture_output=True)
    record(kind, command, result)
    return result


def load_tsv(path: Path) -> list[dict[str, str]]:
    lines = [line.split("\t") for line in path.read_text(encoding="utf-8").splitlines() if line]
    header = lines[0]
    return [dict(zip(header, row)) for row in lines[1:]]


def case_values(case_file: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for token in case_file.read_text(encoding="utf-8").replace("\n", " ").split():
        if "=" in token:
            key, value = token.split("=", 1)
            values[key] = value
    return values


def recorded_environment(path: Path) -> dict[str, str]:
    env: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if "=" in line and not line.startswith("#"):
            key, value = line.split("=", 1)
            if key.isupper() or key == "PATH":
                env[key] = value
    return env


def published_corpus_digest() -> str:
    for line in (REPO / INPUTS["published_fingerprints"][0]).read_text(encoding="utf-8").splitlines():
        if line.startswith("# corpus_root_sha256\t"):
            return line.split("\t", 1)[1]
    raise RuntimeError("published fingerprint artifact has no corpus root digest")


def input_manifest(binary_sha256: str | None = None) -> list[str]:
    rows = [f"run_started_utc\t{utc()}", f"product_successor_commit\t{PRODUCT_SUCCESSOR}", f"runner_commit\t{git('rev-parse', 'HEAD')}", f"corpus_root_sha256\t{published_corpus_digest()}"]
    for name, (path, expected) in INPUTS.items():
        actual = git("rev-parse", f"HEAD:{path}")
        if actual != expected:
            raise RuntimeError(f"FIXED-INPUT-MISMATCH {name}: {actual} != {expected}")
        rows.append(f"{name}\t{path}\t{actual}")
    if binary_sha256 is not None:
        rows.append(f"successor_binary_sha256\t{binary_sha256}")
    return rows


def calibrate_c13() -> None:
    calibration = RUN / "calibration"
    calibration.mkdir(parents=True, exist_ok=True)
    echo = checked("c13-calibration", ["/bin/echo", "ae-p4-run2-calibrate"])
    (calibration / "echo.stdout").write_bytes(echo.stdout)
    (calibration / "echo.stderr").write_bytes(echo.stderr)
    socket = calibration / "tmux.sock"
    checked("c13-calibration", ["tmux", "-S", str(socket), "new-session", "-d", "-s", "calibrate", "sleep 2"])
    panes = checked("c13-calibration", ["tmux", "-S", str(socket), "list-panes", "-t", "calibrate", "-F", "#{pane_id}"])
    (calibration / "tmux.stdout").write_bytes(panes.stdout)
    (calibration / "tmux.stderr").write_bytes(panes.stderr)
    checked("c13-calibration", ["tmux", "-S", str(socket), "kill-server"])
    if echo.returncode or panes.returncode or not panes.stdout.strip():
        raise RuntimeError("C13-CALIBRATION-FAILED")


def run_verifier(phase: str, label: str, relative: str) -> None:
    result = checked("verifier", [sys.executable, str(REPO / relative)])
    root = RUN / phase
    (root / f"{label}.stdout").parent.mkdir(parents=True, exist_ok=True)
    (root / f"{label}.stdout").write_bytes(result.stdout)
    (root / f"{label}.stderr").write_bytes(result.stderr)
    (root / f"{label}.rc").write_text(f"{result.returncode}\n", encoding="utf-8")
    if result.returncode:
        raise RuntimeError(f"PREFLIGHT-FAILED {label} rc={result.returncode}")


def build_successor() -> str:
    result = checked("successor-build", ["cargo", "build", "--release", "--locked", "--target-dir", str(RUN / "build")])
    if result.returncode or not AE.is_file():
        raise RuntimeError("SUCCESSOR-BUILD-FAILED")
    digest = sha256_file(AE)
    write(RUN / "SUCCESSOR-BINARY-SHA256.txt", f"{digest}\n")
    return digest


def preflight() -> None:
    RUN.mkdir(parents=True, exist_ok=True)
    if (RUN / "RUN-MANIFEST.txt").exists():
        raise RuntimeError("REFUSE-REUSE existing run manifest")
    if git("status", "--porcelain", "--untracked-files=all"):
        raise RuntimeError("RUN-REFUSED worktree is not clean at pin time")
    calibrate_c13()  # instrumentation before the first identity observation
    if git_rc("diff", "--quiet", PRODUCT_SUCCESSOR, "HEAD", "--", "src") != 0:
        raise RuntimeError("PRODUCT-TREE-MOVED relative to pre-slice successor")
    for label, relative in (
        ("corpus", "docs/migration/evidence/corpus/verify-corpus.py"),
        ("invocations", "docs/migration/evidence/corpus/verify-invocations.py"),
        ("obligations", "docs/migration/evidence/corpus/verify-obligations.py"),
        ("c3", "docs/migration/evidence/verify-contract-obligation-reconciliation.py"),
        ("c8", "docs/migration/evidence/verify-open-choice-reconciliation.py"),
        ("published-fingerprints", "docs/migration/evidence/verify-published-fingerprints.py"),
    ):
        run_verifier("preflight", label, relative)
    census_manifest_shapes(expected_fingerprints())
    binary_sha256 = build_successor()
    rows = input_manifest(binary_sha256)
    write(RUN / "RUN-MANIFEST.txt", "\n".join(rows) + "\n")


def expected_fingerprints() -> dict[str, tuple[int, str, str]]:
    rows: dict[str, tuple[int, str, str]] = {}
    path = REPO / "docs/migration/evidence/p1-phase4-published-fingerprints.tsv"
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#") or line.startswith("source_path\t"):
            continue
        member, count, tree, canonical = line.split("\t")
        if member in rows:
            raise RuntimeError(f"duplicate published member {member}")
        rows[member] = (int(count), tree, canonical)
    if len(rows) != 70:
        raise RuntimeError(f"PUBLISHED-FINGERPRINTS malformed member count {len(rows)}")
    return rows


def census_manifest_shapes(expected: dict[str, tuple[int, str, str]]) -> Counter[str]:
    """Classify every published member by its recorded no-mutation manifest grain."""
    manifests: dict[str, list[Path]] = {member: [] for member in expected}
    for case_file in CORPUS.rglob("case.txt"):
        values = case_values(case_file)
        before = case_file.parent / "manifest.before.tsv"
        template = values.get("template")
        if not template or not before.is_file():
            continue
        group, member = template.split("/", 1)
        source_key = str((TEMPLATES / group / "fixture-bytes" / member).relative_to(REPO))
        if source_key not in manifests:
            raise RuntimeError(f"manifest census has unpublished template {source_key}")
        manifests[source_key].append(before)
    rows, counts = [], Counter()
    for member, (published_entries, _tree, _canonical) in sorted(expected.items()):
        sources = manifests[member]
        line_counts = sorted({len(path.read_bytes().splitlines()) for path in sources})
        if not sources:
            shape = "NO-RECORDED-MANIFEST"
        elif all(count == published_entries for count in line_counts):
            shape = "PUBLISHED-SHAPED"
        elif all(count > published_entries for count in line_counts):
            shape = "STORE-SHAPED"
        else:
            shape = "MIXED-SHAPE"
        counts[shape] += 1
        rows.append(f"{member}\t{published_entries}\t{len(sources)}\t{','.join(map(str, line_counts)) or '-'}\t{shape}")
    if sum(counts.values()) != 70:
        raise RuntimeError(f"C14 manifest-shape census covers {sum(counts.values())}, expected 70")
    write(RUN / "C14-MANIFEST-SHAPE-CENSUS.tsv", "member\tpublished_entries\tmanifest_count\tmanifest_line_counts\tclassification\n" + "\n".join(rows) + "\n")
    write(RUN / "C14-MANIFEST-SHAPE-SUMMARY.txt", "\n".join(f"{key}\t{counts[key]}" for key in sorted(counts)) + "\n")
    return counts


def fingerprint_tree(root: Path) -> tuple[str, str, int]:
    """Canonical content identity plus executable-bit projection, no directories."""
    entries: list[tuple[bytes, bytes, bytes]] = []
    for base, dirs, files in os.walk(root, followlinks=False):
        # A symlink to a directory is a leaf under Git; ordinary directories are
        # implied by their descendants and must not enter either published identity.
        for name in files + [name for name in dirs if (Path(base) / name).is_symlink()]:
            path = Path(base) / name
            rel = os.fsencode(str(path.relative_to(root)))
            mode = path.lstat().st_mode
            if stat.S_ISLNK(mode):
                kind, payload, executable = b"symlink", os.fsencode(os.readlink(path)), b"0"
            elif stat.S_ISREG(mode):
                kind, payload = b"file", sha256_file(path).encode("ascii")
                executable = b"1" if mode & 0o111 else b"0"
            else:
                raise RuntimeError(f"unsupported scratch entry {path}")
            entries.append((rel, kind + b"\0" + payload + b"\0" + rel + b"\0", kind + b"\0" + executable + b"\0" + rel + b"\0"))
    entries.sort(key=lambda item: item[0])
    canonical = hashlib.sha256(b"".join(item[1] for item in entries)).hexdigest()
    executable = hashlib.sha256(b"".join(item[2] for item in entries)).hexdigest()
    return canonical, executable, len(entries)


def git_executable_projection(member: str) -> tuple[str, str]:
    raw = subprocess.run(["git", "ls-tree", "-r", "-z", "HEAD", "--", member], cwd=REPO, capture_output=True, check=True).stdout
    entries: list[tuple[bytes, bytes]] = []
    prefix = member.encode("utf-8") + b"/"
    for record in raw.split(b"\0"):
        if not record:
            continue
        left, path = record.split(b"\t", 1)
        mode, kind, _oid = left.decode("ascii").split(" ")
        if kind != "blob" or not path.startswith(prefix):
            raise RuntimeError(f"unexpected Git member entry {record!r}")
        rel = path[len(prefix):]
        label = b"symlink" if mode == "120000" else b"file"
        executable = b"1" if mode == "100755" else b"0"
        entries.append((rel, label + b"\0" + executable + b"\0" + rel + b"\0"))
    entries.sort(key=lambda item: item[0])
    tree_id = git("rev-parse", f"HEAD:{member}")
    return tree_id, hashlib.sha256(b"".join(item[1] for item in entries)).hexdigest()


def make_readonly(root: Path) -> None:
    if not root.is_symlink():
        os.chmod(root, root.stat().st_mode & ~0o222)
    for base, dirs, files in os.walk(root, followlinks=False):
        for name in dirs + files:
            path = Path(base) / name
            if not path.is_symlink():
                os.chmod(path, path.stat().st_mode & ~0o222)


def make_writable(root: Path) -> None:
    """Release only a disposable copy so it can be removed after its proof."""
    if not root.is_symlink():
        os.chmod(root, root.stat().st_mode | stat.S_IWUSR)
    for base, dirs, files in os.walk(root, followlinks=False):
        for name in dirs + files:
            path = Path(base) / name
            if not path.is_symlink():
                os.chmod(path, path.stat().st_mode | stat.S_IWUSR)


def prove_readonly(source: Path, env: dict[str, str], record_path: Path) -> None:
    nested = next((path for path in source.rglob("*") if path.is_dir() and not path.is_symlink()), None)
    existing = next((path for path in source.rglob("*") if path.is_file() and not path.is_symlink()), None)
    if nested is None or existing is None:
        raise RuntimeError(f"READONLY-PROOF-UNAVAILABLE {source}")
    original, mode = existing.read_bytes(), existing.stat().st_mode & 0o777
    attempts = [
        ("create-root-alternate-spelling", source / "." / "p4-run2-create-root", "pathlib.Path(sys.argv[1]).write_bytes(b'probe')"),
        ("create-nested", nested / "p4-run2-create-nested", "pathlib.Path(sys.argv[1]).write_bytes(b'probe')"),
        ("overwrite-existing", existing, "pathlib.Path(sys.argv[1]).write_bytes(b'probe')"),
        ("unlink-existing", existing, "pathlib.Path(sys.argv[1]).unlink()"),
    ]
    lines: list[str] = []
    for label, candidate, action in attempts:
        command = [sys.executable, "-c", f"import pathlib,sys; {action}", str(candidate)]
        result = checked("readonly-proof", command, env=env)
        lines.append(f"{label}\t{candidate}\trc={result.returncode}\t{result.stderr.decode('utf-8', 'replace').strip()}")
        if result.returncode == 0:
            # This only operates on the disposable source/scratch copy. Restore
            # unexpected overwrite/unlink effects before surfacing the failure.
            if candidate == existing:
                existing.parent.chmod(existing.parent.stat().st_mode | stat.S_IWUSR)
                existing.write_bytes(original)
                os.chmod(existing, mode)
            elif candidate.exists():
                candidate.unlink()
            raise RuntimeError(f"READONLY-PROOF-FAILED {label} {candidate}")
    write(record_path, "\n".join(lines) + "\n")


def tab_oracle(env: dict[str, str], instrument_root: Path, record_path: Path, expected_c_locale: bool) -> None:
    socket = instrument_root / "tab-oracle.sock"
    create = checked("tab-oracle", ["tmux", "-S", str(socket), "new-session", "-d", "-s", "tab", "/bin/sh", "-c", "sleep 5"], env=env)
    if create.returncode:
        raise RuntimeError("TAB-ORACLE-CREATE-FAILED")
    checked("tab-oracle", ["tmux", "-S", str(socket), "set-option", "-t", "tab", "@ae_agent", "x:y"], env=env)
    query = checked("tab-oracle", ["tmux", "-S", str(socket), "list-panes", "-t", "tab", "-F", "#{@ae_agent}\t#{pane_current_command}"], env=env)
    checked("tab-oracle", ["tmux", "-S", str(socket), "kill-server"], env=env)
    fields = query.stdout.rstrip(b"\n").split(b"\t")
    write(record_path, f"raw_hex\t{query.stdout.hex()}\nfields\t{len(fields)}\nlocale\t{env['LANG']}/{env['LC_ALL']}\n")
    expected = [b"x:y_sleep", b""] if expected_c_locale else [b"x:y", b"sleep"]
    if query.returncode or fields != expected:
        raise RuntimeError("TAB-ORACLE-FAILED")


def state_manifest(root: Path) -> bytes:
    """Complete post-policy scratch state: path, kind, mode, content/target."""
    lines: list[tuple[bytes, str]] = []
    seen: set[bytes] = set()
    def add(path: Path, rel: str) -> None:
        raw = os.fsencode(rel)
        if raw in seen:
            raise RuntimeError(f"duplicate scratch manifest path {rel}")
        seen.add(raw)
        mode = oct(path.lstat().st_mode & 0o777)[2:]
        if path.is_symlink():
            line = f"symlink\t{mode}\t-\t{os.readlink(path)}\t{rel}"
        elif path.is_dir():
            line = f"dir\t{mode}\t-\t-\t{rel}"
        elif path.is_file():
            line = f"file\t{mode}\t{sha256_file(path)}\t-\t{rel}"
        else:
            raise RuntimeError(f"unsupported scratch entry {path}")
        lines.append((raw, line))
    add(root, ".")
    for base, dirs, files in os.walk(root, followlinks=False):
        for name in dirs + files:
            path = Path(base) / name
            rel = "./" + str(path.relative_to(root))
            add(path, rel)
    return ("\n".join(line for _raw, line in sorted(lines)) + "\n").encode("utf-8")


def effective_environment(case_dir: Path, home: Path, instrument_root: Path) -> tuple[dict[str, str], list[str]]:
    recorded = recorded_environment(case_dir / "env.txt")
    required = ("PATH", "TZ", "LANG", "LC_ALL", "TERM", "AE_TMUX_SERVER")
    missing = [key for key in required if not recorded.get(key)]
    if missing:
        raise RuntimeError(f"unbound effective environment: missing {','.join(missing)}")
    env = dict(recorded)
    ae_home = home / ".ae"
    remaps = [
        f"HOME={home} (fresh scratch)",
        f"AE_HOME={ae_home} (fresh scratch)",
        f"AE_TMUX_SERVER={instrument_root / 'successor.tmux.sock'} (isolated runner transport socket)",
        f"TMUX_TMPDIR={instrument_root / 'tmux-tmp'} (isolated runner transport tmpdir)",
    ]
    env.update({
        "HOME": str(home),
        "AE_HOME": str(ae_home),
        "AE_TMUX_SERVER": str(instrument_root / "successor.tmux.sock"),
        "TMUX_TMPDIR": str(instrument_root / "tmux-tmp"),
    })
    return env, remaps


def invoke(tokens: list[str], env: dict[str, str], cwd: Path) -> tuple[int, bytes, bytes]:
    command = [str(AE), *tokens]
    try:
        result = subprocess.run(command, cwd=cwd, env=env, capture_output=True, timeout=20)
        record("successor", command, result)
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired as error:
        result = subprocess.CompletedProcess(command, 124, error.stdout or b"", (error.stderr or b"") + b"\ntimeout\n")
        record("successor-timeout", command, result)
        return result.returncode, result.stdout, result.stderr


def parse_json(raw: bytes) -> object | None:
    def unique(pairs: list[tuple[str, object]]) -> dict[str, object]:
        out: dict[str, object] = {}
        for key, value in pairs:
            if key in out:
                raise ValueError(f"duplicate JSON key {key}")
            out[key] = value
        return out
    try:
        return json.loads(raw.decode("utf-8"), object_pairs_hook=unique)
    except (UnicodeDecodeError, ValueError, json.JSONDecodeError):
        return None


def unknown_in_human(raw: bytes) -> bool:
    for line in raw.decode("utf-8", "replace").splitlines():
        if line.startswith("  "):
            continue
        fields = line.split("\t")
        if len(fields) >= 2 and fields[1] == "unknown":
            return True
    return False


def score_obligation(row: dict[str, str], stdout: bytes) -> tuple[str, str]:
    if row["support"] == "UNSCORABLE":
        return "UNSCORABLE", "fixed support is UNSCORABLE"
    stream, oid, locus, wanted = row["stream"], row["obligation_id"], row["locus"], row["to"]
    if stream == "stdout" and oid == "SC-017m":
        ok = unknown_in_human(stdout)
        return ("PASS" if ok else "FAIL"), "unknown human session row"
    if stream != "digest":
        return "NOT-EVALUATED", f"runner lacks a {stream} predicate for {oid}/{locus}"
    document = parse_json(stdout)
    if not isinstance(document, dict):
        return "FAIL", "expected JSON digest unavailable"
    sessions = document.get("sessions")
    if oid == "SC-509d":
        got = document.get("schema_version")
        ok = not isinstance(got, bool) and got == 2
        return ("PASS" if ok else "FAIL"), f"schema_version={got!r}"
    if oid == "SC-017o":
        got = document.get("inventory_complete")
        ok = type(got) is bool
        return ("PASS" if ok else "FAIL"), f"inventory_complete={got!r}"
    if not isinstance(sessions, list):
        return "FAIL", "sessions is not an array"
    if oid == "SC-017l":
        values = [item.get("status") for item in sessions if isinstance(item, dict)]
        ok = bool(values) and all(value == wanted for value in values)
        return ("PASS" if ok else "FAIL"), f"statuses={values!r}"
    if oid == "SC-017m":
        ok = any(isinstance(item, dict) and item.get("status") == "unknown" for item in sessions)
        return ("PASS" if ok else "FAIL"), "unknown JSON session row"
    if oid == "SC-509b":
        values = [item.get("degraded") for item in sessions if isinstance(item, dict)]
        ok = any(value is True for value in values)
        return ("PASS" if ok else "FAIL"), f"degraded={values!r}"
    if oid == "SC-509c":
        match = re.fullmatch(r"sessions\[([^]]+)\]\.agents\[([^]]+)\]\.reason", locus)
        if not match:
            return "FAIL", f"unparseable reason locus {locus}"
        name, ref = match.groups()
        session = next((item for item in sessions if isinstance(item, dict) and item.get("name") == name), None)
        agents = session.get("agents") if isinstance(session, dict) else None
        agent = next((item for item in agents or [] if isinstance(item, dict) and item.get("ref") == ref), None)
        got = agent.get("reason") if isinstance(agent, dict) else None
        return ("PASS" if got == wanted else "FAIL"), f"reason={got!r} wanted={wanted!r}"
    return "NOT-EVALUATED", f"unsupported observed predicate {oid}/{stream}/{locus}"


def comparison_rows(row_key: str, state: str, base_rc: str, rc: int | None, base_out: bytes, out: bytes, base_err: bytes, err: bytes, no_mutation: str) -> list[tuple[str, str, str]]:
    if state != "EXECUTED":
        return [(kind, state, "no successor capture") for kind in ("rc", "stdout", "stderr", "no-mutation")]
    rc_verdict = "PASS" if str(rc) == base_rc else "FAIL"
    # Exact bytes are decisive only when equal. Differences need the complete fixed
    # projection and directional relation; this runner reports that gap explicitly.
    stdout_verdict = "PASS" if out == base_out else "NOT-EVALUATED"
    stderr_verdict = "PASS" if err == base_err else "NOT-EVALUATED"
    return [("rc", rc_verdict, "exact scalar"), ("stdout", stdout_verdict, "exact bytes or projection gap"), ("stderr", stderr_verdict, "exact bytes or projection gap"), ("no-mutation", no_mutation, "scratch identity")]


def validate_population(invocations: list[dict[str, str]]) -> None:
    expected_surfaces = {"ae list": 743, "ae ls": 116, "helper:requests": 168, "helper:events-tail": 38}
    actual_surfaces = Counter(row["surface"] for row in invocations)
    keys = [(row["case"], row["consumer"]) for row in invocations]
    if len(invocations) != 1065 or actual_surfaces != expected_surfaces or len(keys) != len(set(keys)):
        raise RuntimeError(f"C2-POPULATION-FAILED rows={len(invocations)} surfaces={dict(actual_surfaces)} distinct={len(set(keys))}")


def run() -> None:
    if not (RUN / "RUN-MANIFEST.txt").exists():
        raise RuntimeError("RUN-REFUSED no preflight manifest")
    if not AE.is_file():
        raise RuntimeError(f"RUN-REFUSED missing successor binary {AE}")
    expected = expected_fingerprints()
    invocations = [row for row in load_tsv(INV) if row["phase"] == "P1"]
    validate_population(invocations)
    obligations = load_tsv(OBL)
    if not obligations:
        raise RuntimeError("obligation table is empty")
    by_key: dict[tuple[str, str], list[dict[str, str]]] = {}
    for obligation in obligations:
        by_key.setdefault((obligation["case"], obligation["consumer"]), []).append(obligation)
    runs, comparisons, vectors = [], [], []
    for index, row in enumerate(invocations, 1):
        case_dir = CORPUS / Path(row["case"]).parent
        key = (str(Path(row["case"]).parent), row["consumer"])
        values = case_values(case_dir / "case.txt")
        state, reason, rc, stdout, stderr, no_mutation = "", "", None, b"", b"", "NOT-EVALUATED"
        base_out = (case_dir / "out" / f"{row['consumer']}.stdout").read_bytes() if (case_dir / "out" / f"{row['consumer']}.stdout").exists() else b""
        base_err = (case_dir / "out" / f"{row['consumer']}.stderr").read_bytes() if (case_dir / "out" / f"{row['consumer']}.stderr").exists() else b""
        if row["surface"].startswith("helper:"):
            state, reason = "NOT-EXECUTED", "product surface not implemented at the pinned pre-slice successor"
        elif values.get("clone_mode") == "live":
            state, reason = "FIXTURE-ABORT", "no materialisable fixed fixture bytes"
        elif not values.get("template"):
            state, reason = "FIXTURE-ABORT", "case lacks template binding"
        else:
            group, member = values["template"].split("/", 1)
            source = TEMPLATES / group / "fixture-bytes" / member
            source_key = str(source.relative_to(REPO))
            scratch = RUN / "scratch" / f"{index:04d}"
            home, ae_home = scratch / "home", scratch / "home" / ".ae"
            instrument_root = RUN / "instrument" / f"{index:04d}"
            readonly_source = RUN / "read-only-source" / f"{index:04d}"
            fixture = RUN / "fixtures" / f"{index:04d}.tsv"
            if source_key not in expected:
                state, reason = "FIXTURE-ABORT", "template not a published fingerprint member"
            elif not all((case_dir / name).is_file() for name in ("env.txt", "env-tab-selfcheck.txt")):
                state, reason = "FIXTURE-ABORT", "per-invocation environment binding absent"
            elif values.get("state_transform") not in (None, "published-member+bound-environment+permission-policy"):
                state, reason = "FIXTURE-ABORT", "NEW-UNMAPPED-INPUT state_transform is outside member/environment/permission policy"
            else:
                want_count, want_tree, want_canonical = expected[source_key]
                source_canonical, source_exec, source_count = fingerprint_tree(source)
                tree_id, expected_exec = git_executable_projection(source_key)
                if (source_count, source_canonical, tree_id, source_exec) != (want_count, want_canonical, want_tree, expected_exec):
                    state, reason = "FIXTURE-ABORT", "published source member identity mismatch"
                else:
                    # Bind every fixed case artifact before fixture materialisation.
                    env, remaps = effective_environment(case_dir, home, instrument_root)
                    binding = {
                        "case_txt_sha256": sha256_file(case_dir / "case.txt"),
                        "env_sha256": sha256_file(case_dir / "env.txt"),
                        "tab_selfcheck_sha256": sha256_file(case_dir / "env-tab-selfcheck.txt"),
                        "normalised_argv": row["normalised_argv"],
                    }
                    legacy_before = case_dir / "manifest.before.tsv"
                    if legacy_before.is_file():
                        binding["legacy_manifest_before_sha256_inapplicable"] = sha256_file(legacy_before)
                    shutil.copytree(source, readonly_source, symlinks=True)
                    source_copy_canonical, source_copy_exec, source_copy_count = fingerprint_tree(readonly_source)
                    make_readonly(readonly_source)
                    source_ro_canonical, source_ro_exec, source_ro_count = fingerprint_tree(readonly_source)
                    instrument_root.mkdir(parents=True, exist_ok=True)
                    (instrument_root / "tmux-tmp").mkdir(parents=True, exist_ok=True)
                    prove_readonly(readonly_source, env, RUN / "readonly-source" / f"{index:04d}.tsv")
                    shutil.copytree(readonly_source, ae_home, symlinks=True)
                    make_writable(readonly_source)
                    shutil.rmtree(readonly_source)
                    pre_canonical, pre_exec, pre_count = fingerprint_tree(ae_home)
                    make_readonly(ae_home)
                    postperm_canonical, postperm_exec, postperm_count = fingerprint_tree(ae_home)
                    fixture_rows = [
                        *(f"binding_{key}\t{value}" for key, value in binding.items()),
                        f"source_count\t{source_count}", f"source_tree\t{tree_id}", f"source_canonical\t{source_canonical}", f"source_exec\t{source_exec}",
                        f"source_copy_count\t{source_copy_count}", f"source_copy_canonical\t{source_copy_canonical}", f"source_copy_exec\t{source_copy_exec}",
                        f"source_readonly_count\t{source_ro_count}", f"source_readonly_canonical\t{source_ro_canonical}", f"source_readonly_exec\t{source_ro_exec}",
                        f"scratch_pre_count\t{pre_count}", f"scratch_pre_canonical\t{pre_canonical}", f"scratch_pre_exec\t{pre_exec}",
                        f"scratch_postperm_count\t{postperm_count}", f"scratch_postperm_canonical\t{postperm_canonical}", f"scratch_postperm_exec\t{postperm_exec}",
                        *(f"remap\t{remap}" for remap in remaps),
                    ]
                    write(fixture, "\n".join(fixture_rows) + "\n")
                    if (source_copy_count, source_copy_canonical, source_copy_exec) != (want_count, want_canonical, expected_exec) or (source_ro_count, source_ro_canonical, source_ro_exec) != (want_count, want_canonical, expected_exec) or (pre_count, pre_canonical, pre_exec) != (want_count, want_canonical, expected_exec) or (postperm_count, postperm_canonical, postperm_exec) != (want_count, want_canonical, expected_exec):
                        state, reason = "FIXTURE-ABORT", "C14 scratch published identity mismatch"
                    else:
                        prove_readonly(ae_home, env, RUN / "readonly-scratch" / f"{index:04d}.tsv")
                        expected_c_locale = env["LANG"] == "C" and env["LC_ALL"] == "C"
                        tab_oracle(env, instrument_root, RUN / "tab-oracle" / f"{index:04d}.tsv", expected_c_locale)
                        scratch_before = state_manifest(home)
                        (RUN / "scratch-manifests").mkdir(parents=True, exist_ok=True)
                        (RUN / "scratch-manifests" / f"{index:04d}.before.tsv").write_bytes(scratch_before)
                        tokens = row["normalised_argv"].split()
                        if tokens and tokens[0] == "ae":
                            tokens = tokens[1:]
                        rc, stdout, stderr = invoke(tokens, env, home)
                        capture = RUN / "captures" / f"{index:04d}-{row['consumer']}"
                        capture.mkdir(parents=True, exist_ok=True)
                        (capture / "stdout").write_bytes(stdout)
                        (capture / "stderr").write_bytes(stderr)
                        write(capture / "rc", f"{rc}\n")
                        write(capture / "argv", " ".join(tokens) + "\n")
                        scratch_after = state_manifest(home)
                        (RUN / "scratch-manifests" / f"{index:04d}.after.tsv").write_bytes(scratch_after)
                        no_mutation = "PASS" if scratch_after == scratch_before else "FAIL"
                        state, reason = "EXECUTED", "fresh C14 fixture and pre/post scratch state manifest"
        runs.append((index, row["case"], row["consumer"], row["surface"], state, reason))
        comparisons.extend((index, row["case"], row["consumer"], kind, verdict, detail) for kind, verdict, detail in comparison_rows(f"{index}", state, row["rc"], rc, base_out, stdout, base_err, stderr, no_mutation))
        for obligation in by_key.get(key, []):
            if obligation["support"] == "UNSCORABLE":
                verdict, detail = "UNSCORABLE", "fixed support is UNSCORABLE"
            elif state != "EXECUTED":
                verdict, detail = state, reason
            else:
                verdict, detail = score_obligation(obligation, stdout)
            vectors.append((obligation["case"], obligation["consumer"], obligation["obligation_id"], obligation["stream"], obligation["locus"], obligation["support"], verdict, detail))
    if len(runs) != 1065 or len(comparisons) != 4 * len(invocations) or len(vectors) != len(obligations):
        raise RuntimeError(f"ACCOUNTING-FAILED runs={len(runs)} comparisons={len(comparisons)} obligations={len(vectors)}")
    write(RUN / "runs.tsv", "idx\tcase\tconsumer\tsurface\tstate\treason\n" + "".join("\t".join(map(str, row)) + "\n" for row in runs))
    write(RUN / "comparison-vector.tsv", "idx\tcase\tconsumer\tlocus_kind\tverdict\tdetail\n" + "".join("\t".join(map(str, row)) + "\n" for row in comparisons))
    write(RUN / "obligation-vector.tsv", "case\tconsumer\tid\tstream\tlocus\tsupport\tverdict\tdetail\n" + "".join("\t".join(map(str, row)) + "\n" for row in vectors))
    run_counts, comparison_counts, vector_counts = Counter(row[4] for row in runs), Counter(row[4] for row in comparisons), Counter(row[6] for row in vectors)
    write(RUN / "SUMMARY.txt", f"scheduled_rows\t{len(runs)}\nexecuted_children\t{run_counts['EXECUTED']}\nrun_states\t{dict(sorted(run_counts.items()))}\ncomparison_states\t{dict(sorted(comparison_counts.items()))}\nobligation_states\t{dict(sorted(vector_counts.items()))}\n")
    print((RUN / "SUMMARY.txt").read_text(encoding="utf-8"), end="")


def postflight() -> None:
    if not (RUN / "SUMMARY.txt").exists():
        raise RuntimeError("POSTFLIGHT-REFUSED no completed run")
    if git_rc("diff", "--quiet", PRODUCT_SUCCESSOR, "HEAD", "--", "src") != 0:
        raise RuntimeError("PRODUCT-TREE-MOVED before postflight")
    recorded_binary = (RUN / "SUCCESSOR-BINARY-SHA256.txt").read_text(encoding="utf-8").strip()
    if not recorded_binary or not AE.is_file() or sha256_file(AE) != recorded_binary:
        raise RuntimeError("SUCCESSOR-BINARY-MOVED before postflight")
    rows = input_manifest(recorded_binary)
    for label, relative in (
        ("corpus", "docs/migration/evidence/corpus/verify-corpus.py"),
        ("invocations", "docs/migration/evidence/corpus/verify-invocations.py"),
        ("obligations", "docs/migration/evidence/corpus/verify-obligations.py"),
        ("c3", "docs/migration/evidence/verify-contract-obligation-reconciliation.py"),
        ("c8", "docs/migration/evidence/verify-open-choice-reconciliation.py"),
        ("published-fingerprints", "docs/migration/evidence/verify-published-fingerprints.py"),
    ):
        run_verifier("postflight", label, relative)
    write(RUN / "POST-MANIFEST.txt", "\n".join(rows) + "\n")


def selftest() -> None:
    skipped = comparison_rows("1", "NOT-EXECUTED", "0", None, b"", b"", b"", b"", "NOT-EVALUATED")
    if len(skipped) != 4 or any(verdict != "NOT-EXECUTED" for _kind, verdict, _detail in skipped):
        raise RuntimeError("B1 selftest: a skip inflated or vanished from comparison accounting")
    changed = comparison_rows("1", "EXECUTED", "0", 0, b"baseline", b"changed", b"", b"", "PASS")
    if dict((kind, verdict) for kind, verdict, _detail in changed)["stdout"] != "NOT-EVALUATED":
        raise RuntimeError("fail-closed selftest: unexplained stdout change was accepted")
    if sha256(b"same") != sha256(b"same") or sha256(b"same") == sha256(b"different"):
        raise RuntimeError("hash selftest failed")
    unsupported = score_obligation({"support": "OBSERVED", "stream": "stderr", "obligation_id": "SC-X", "locus": "x", "to": "x"}, b"")
    if unsupported[0] != "NOT-EVALUATED":
        raise RuntimeError("fail-closed selftest: unimplemented predicate became product FAIL")
    expected = expected_fingerprints()
    mismatches = []
    for member, (count, tree, canonical) in expected.items():
        actual = REPO / member
        got_canonical, got_exec, got_count = fingerprint_tree(actual)
        got_tree, expected_exec = git_executable_projection(member)
        if (got_count, got_tree, got_canonical, got_exec) != (count, tree, canonical, expected_exec):
            mismatches.append(member)
    if mismatches:
        raise RuntimeError(f"published fingerprint selftest mismatches: {mismatches}")
    print("RUN2-SELFTEST PASS: B1 accounting; fail-closed scoring; 70/70 published identities")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("phase", choices=("preflight", "run", "postflight", "selftest"))
    args = parser.parse_args()
    try:
        if args.phase == "preflight":
            preflight()
        elif args.phase == "run":
            run()
        elif args.phase == "postflight":
            postflight()
        else:
            selftest()
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"RUNNER-ABORT: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
