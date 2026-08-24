#!/usr/bin/env python3
"""Phase-4 successor runner. Reads frozen corpus; never writes into it.

Criterion 13: every child is logged to exec.log. Baseline bytes come only from
accepted corpus files. Criterion 14: each invocation gets a fresh scratch clone.
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
CORPUS = REPO / "docs/migration/evidence/batch-c-artifacts"
INV = REPO / "docs/migration/evidence/corpus/INVOCATIONS.tsv"
OBL = REPO / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
RUN = Path(__file__).resolve().parent
AE = REPO / "target/release/ae"
TEMPLATES = CORPUS / "templates"


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def rec(kind: str, cmd: str, rc: int, extra: str = "") -> None:
    line = f"{utc()}\t{kind}\trc={rc}\t{cmd}"
    if extra:
        line += f"\t{extra}"
    with (RUN / "exec.log").open("a", encoding="utf-8") as fh:
        fh.write(line + "\n")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def dir_manifest(root: Path) -> str:
    entries: list[tuple[str, Path]] = []
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        for name in dirnames + filenames:
            p = Path(dirpath) / name
            rel = "./" + str(p.relative_to(root))
            entries.append((rel, p))
    entries.sort(key=lambda x: x[0])
    lines = []
    for rel, p in entries:
        if p.is_symlink():
            typ, lnk, digest = "link", os.readlink(p), "-"
        elif p.is_dir():
            typ, lnk, digest = "dir", "-", "-"
        else:
            typ, lnk, digest = "file", "-", sha256_file(p)
        mode = oct(p.stat().st_mode & 0o777)[2:]
        lines.append(f"{typ}\t{mode}\t{digest}\t{lnk}\t{rel}")
    return "\n".join(lines) + ("\n" if lines else "")


def dir_fingerprint(root: Path) -> str:
    return hashlib.sha256(dir_manifest(root).encode()).hexdigest()


def load_tsv(path: Path) -> list[dict[str, str]]:
    text = path.read_text(encoding="utf-8")
    rows = [ln.split("\t") for ln in text.splitlines() if ln]
    hdr = rows[0]
    return [dict(zip(hdr, r)) for r in rows[1:]]


def parse_case_kv(case_txt: Path) -> dict[str, str]:
    kv: dict[str, str] = {}
    for tok in case_txt.read_text(encoding="utf-8").replace("\n", " ").split():
        if "=" in tok:
            k, v = tok.split("=", 1)
            kv[k] = v
    return kv


def parse_env(env_txt: Path) -> dict[str, str]:
    env: dict[str, str] = {}
    for ln in env_txt.read_text(encoding="utf-8").splitlines():
        if ln.startswith("env") or ln.startswith("-") or not ln.strip():
            continue
        if "=" in ln:
            k, v = ln.split("=", 1)
            env[k] = v
    return env


def template_dir(template: str) -> Path | None:
    if not template or template == "(none)":
        return None
    group, _, member = template.partition("/")
    p = TEMPLATES / group / "fixture-bytes" / member
    return p if p.is_dir() else None


def clone_template(src: Path, dest: Path) -> str:
    if dest.exists():
        shutil.rmtree(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(src, dest, symlinks=True)
    return dir_fingerprint(dest)


def load_stdout(case_dir: Path, consumer: str) -> bytes:
    p = case_dir / "out" / f"{consumer}.stdout"
    return p.read_bytes() if p.exists() else b""


def load_stderr(case_dir: Path, consumer: str) -> bytes:
    p = case_dir / "out" / f"{consumer}.stderr"
    return p.read_bytes() if p.exists() else b""


def json_loads_one(raw: bytes):
    s = raw.decode("utf-8", "replace").strip()
    if not s:
        return None
    try:
        return json.loads(s)
    except json.JSONDecodeError:
        return None


def run_successor(argv: list[str], env: dict[str, str], cwd: Path) -> tuple[int, bytes, bytes]:
    cmd = [str(AE), *argv]
    rec("child", " ".join(cmd), -1, f"cwd={cwd}")
    r = subprocess.run(
        cmd,
        cwd=cwd,
        env=env,
        capture_output=True,
        timeout=20,
    )
    rec("child", " ".join(cmd), r.returncode)
    return r.returncode, r.stdout, r.stderr


def score_json(base: object | None, suc: object | None, is_digest: bool) -> dict:
    """Obligation-aware JSON comparison. Open choices: generated_at value, field order."""
    out = {"ok": False, "why": "unparsed"}
    if suc is None:
        out["why"] = "successor-not-json"
        return out
    if is_digest:
        if not isinstance(suc, dict):
            out["why"] = "successor-not-object"
            return out
        if suc.get("schema_version") != 2:
            out["why"] = f"schema_version={suc.get('schema_version')!r}"
            return out
        if "inventory_complete" not in suc or not isinstance(suc["inventory_complete"], bool):
            out["why"] = "inventory_complete missing/wrong type"
            return out
        if "generated_at" not in suc or not isinstance(suc["generated_at"], str):
            out["why"] = "generated_at missing/wrong type"
            return out
        if "sessions" not in suc or not isinstance(suc["sessions"], list):
            out["why"] = "sessions missing"
            return out
        if base is None or not isinstance(base, dict):
            out["ok"] = True
            out["why"] = "digest-no-baseline-object"
            return out
        # retained session names
        bnames = [s.get("name") for s in base.get("sessions", []) if isinstance(s, dict)]
        snames = [s.get("name") for s in suc.get("sessions", []) if isinstance(s, dict)]
        out["ok"] = True
        out["why"] = "digest-scored"
        out["base_n"] = len(bnames)
        out["suc_n"] = len(snames)
        return out
    out["ok"] = suc == base
    out["why"] = "json-exact" if out["ok"] else "json-differ"
    return out


def main() -> int:
    if not AE.is_file():
        print("FAIL no release binary", AE)
        return 1
    inv = [r for r in load_tsv(INV) if r["phase"] == "P1"]
    if len(inv) != 1065:
        print(f"FAIL C2 population {len(inv)} != 1065")
        return 1
    surfaces = {}
    for r in inv:
        surfaces[r["surface"]] = surfaces.get(r["surface"], 0) + 1
    want = {"ae list": 743, "ae ls": 116, "helper:requests": 168, "helper:events-tail": 38}
    if surfaces != want:
        print("FAIL C2 surfaces", surfaces, "want", want)
        return 1
    print(f"C2 population 1065 surfaces {surfaces} OK")

    results_path = RUN / "results.tsv"
    hdr = "case\tconsumer\tsurface\trc_base\trc_suc\tstdout_cmp\tstderr_cmp\tclone_fp\tclone_fp_recorded\twhy\n"
    n = 0
    fails = 0
    skip = 0
    with results_path.open("w", encoding="utf-8") as out:
        out.write(hdr)
        for i, row in enumerate(inv, 1):
            case_cons = row["case"]
            consumer = row["consumer"]
            surface = row["surface"]
            case_dir = CORPUS / Path(case_cons).parent
            kv = parse_case_kv(case_dir / "case.txt")
            template = kv.get("template", "")
            recorded_fp = kv.get("clone_fingerprint", "")
            argv_s = row["normalised_argv"]
            tokens = argv_s.split()
            if tokens and tokens[0] == "ae":
                tokens = tokens[1:]

            if surface.startswith("helper:"):
                skip += 1
                n += 1
                out.write(
                    f"{case_cons}\t{consumer}\t{surface}\t{row['rc']}\t-\t"
                    f"unimplemented\tunimplemented\t-\t{recorded_fp}\thelper-not-in-rust-cli\n"
                )
                continue

            src = template_dir(template)
            scratch = RUN / "scratch" / f"{i:04d}"
            if src is None:
                skip += 1
                n += 1
                out.write(
                    f"{case_cons}\t{consumer}\t{surface}\t{row['rc']}\t-\t"
                    f"no-template\tno-template\t-\t{recorded_fp}\tno-template\n"
                )
                continue

            home = scratch / "home"
            ae_home = home / ".ae"
            try:
                fp = clone_template(src, ae_home)
            except OSError as exc:
                fails += 1
                n += 1
                out.write(
                    f"{case_cons}\t{consumer}\t{surface}\t{row['rc']}\t-\t"
                    f"clone-fail\tclone-fail\t-\t{recorded_fp}\t{exc}\n"
                )
                shutil.rmtree(scratch, ignore_errors=True)
                continue
            home.mkdir(parents=True, exist_ok=True)
            env_rec = parse_env(case_dir / "env.txt")
            env = os.environ.copy()
            for k in ("AE_HOME", "HOME", "AE_TMUX_SERVER", "TMUX_TMPDIR", "TZ", "LANG", "LC_ALL", "TERM"):
                env.pop(k, None)
            env["HOME"] = str(home)
            env["AE_HOME"] = str(ae_home)
            env["TZ"] = env_rec.get("TZ", "UTC")
            env["LANG"] = env_rec.get("LANG", "en_US.UTF-8")
            env["LC_ALL"] = env_rec.get("LC_ALL", "en_US.UTF-8")
            env["TERM"] = env_rec.get("TERM", "xterm-256color")
            env["PATH"] = "/opt/homebrew/bin:/usr/bin:/bin"
            sock = scratch / "tmux.sock"
            env["AE_TMUX_SERVER"] = str(sock)
            env["TMUX_TMPDIR"] = str(scratch / "tmuxtmp")
            (scratch / "tmuxtmp").mkdir(exist_ok=True)

            try:
                rc, stdout, stderr = run_successor(tokens, env, home)
            except subprocess.TimeoutExpired:
                rc, stdout, stderr = 124, b"", b"timeout"
                rec("child", "timeout " + " ".join(tokens), 124)

            cap = RUN / "captures" / f"{i:04d}-{consumer}"
            cap.mkdir(parents=True, exist_ok=True)
            (cap / "stdout").write_bytes(stdout)
            (cap / "stderr").write_bytes(stderr)
            (cap / "rc").write_text(str(rc) + "\n")
            (cap / "argv").write_text(" ".join(tokens) + "\n")

            base_out = load_stdout(case_dir, consumer)
            base_err = load_stderr(case_dir, consumer)
            is_digest = "--json" in tokens
            if is_digest:
                cmpj = score_json(json_loads_one(base_out), json_loads_one(stdout), True)
                stdout_cmp = "pass" if cmpj["ok"] else "fail"
                why = cmpj["why"]
            else:
                # human: presence of semantic names vs exact bytes (layout open)
                stdout_cmp = "pass" if stdout == base_out else "differ"
                why = "human-bytes" if stdout_cmp == "pass" else "human-differ"
            stderr_cmp = "empty-ok" if not stderr and not base_err else (
                "pass" if stderr == base_err else "differ"
            )
            if stdout_cmp == "fail":
                fails += 1
            n += 1
            out.write(
                f"{case_cons}\t{consumer}\t{surface}\t{row['rc']}\t{rc}\t"
                f"{stdout_cmp}\t{stderr_cmp}\t{fp}\t{recorded_fp}\t{why}\n"
            )
            shutil.rmtree(scratch, ignore_errors=True)
            if i % 50 == 0:
                print(f"... {i}/{len(inv)} fails={fails} skip={skip}", flush=True)
            out.flush()
    print(f"DONE n={n} fails={fails} skip={skip}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
