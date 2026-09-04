#!/usr/bin/env python3
"""Score OBSERVED obligations against successor captures. Never writes the corpus.

UNSCORABLE is preserved, never promoted. Missing capture is FIXTURE-ABORT,
not a product fail.
"""
from __future__ import annotations

import json
import re
import sys
import tempfile
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
from pathlib import Path

from artifact_tuple import ArtifactTuple, ArtifactTupleError, parse_tsv_bytes, read_generated_tuple

REPO = Path(__file__).resolve().parents[4]
INV = REPO / "docs/migration/evidence/corpus/INVOCATIONS.tsv"
RUN = Path(__file__).resolve().parent
CAPS = RUN / "captures"


def load_tsv(path: Path):
    rows = [ln.split("\t") for ln in path.read_text(encoding="utf-8").splitlines() if ln]
    hdr = rows[0]
    return hdr, [dict(zip(hdr, r)) for r in rows[1:]]


def cap_dir(idx: int, consumer: str) -> Path:
    return CAPS / f"{idx:04d}-{consumer}"


def parse_json(raw: bytes):
    s = raw.decode("utf-8", "replace").strip()
    if not s:
        return None
    try:
        return json.loads(s)
    except json.JSONDecodeError:
        return None


def human_sessions(stdout: str) -> list[tuple[str, str]]:
    """(name, status) for non-agent rows."""
    out = []
    for ln in stdout.splitlines():
        if ln.startswith("  "):
            continue
        parts = ln.split("\t")
        if len(parts) >= 2 and parts[0] and not parts[0].startswith("No "):
            out.append((parts[0], parts[1]))
    return out


def score_one(obl: dict, cap: Path) -> tuple[str, str]:
    """Return (PASS|FAIL|FIXTURE-ABORT, detail)."""
    stdout = (cap / "stdout").read_bytes() if (cap / "stdout").exists() else b""
    stderr = (cap / "stderr").read_bytes() if (cap / "stderr").exists() else b""
    stream = obl["stream"]
    locus = obl["locus"]
    pred = obl["predicate"]
    want = obl["to"]
    oid = obl["obligation_id"]

    if stream == "digest":
        doc = parse_json(stdout)
        if doc is None:
            return "FAIL", "successor-not-json"
        if oid == "SC-509d" and locus == "schema_version":
            got = doc.get("schema_version")
            ok = pred == "equals" and got == 2 or got == 2.0
            # JSON number 2
            ok = got == 2
            return ("PASS" if ok else "FAIL"), f"schema_version={got!r}"
        if oid == "SC-017o" and locus == "inventory_complete":
            got = doc.get("inventory_complete")
            if want == "true":
                ok = got is True
            elif want == "false":
                ok = got is False
            else:
                ok = False
            return ("PASS" if ok else "FAIL"), f"inventory_complete={got!r} want={want}"
        if oid == "SC-509b" and "degraded" in locus:
            sessions = doc.get("sessions") if isinstance(doc, dict) else None
            if not isinstance(sessions, list):
                return "FAIL", "no sessions"
            flags = [s.get("degraded") for s in sessions if isinstance(s, dict)]
            ok = True in flags or any(s.get("degraded") is True for s in sessions if isinstance(s, dict))
            return ("PASS" if ok else "FAIL"), f"degraded_values={flags!r}"
        if oid == "SC-017l" and pred == "all-of":
            sessions = doc.get("sessions") if isinstance(doc, dict) else None
            if not isinstance(sessions, list):
                return "FAIL", "no sessions"
            statuses = [s.get("status") for s in sessions if isinstance(s, dict)]
            if not statuses:
                return "FAIL", "empty-sessions"
            ok = all(st == want for st in statuses)
            return ("PASS" if ok else "FAIL"), f"statuses={statuses!r}"
        if oid == "SC-017m" and pred == "present":
            sessions = doc.get("sessions") if isinstance(doc, dict) else None
            if not isinstance(sessions, list):
                return "FAIL", "no sessions"
            ok = any(isinstance(s, dict) and s.get("status") == "unknown" for s in sessions)
            return ("PASS" if ok else "FAIL"), f"n={len(sessions)} unknown={ok}"
        if oid == "SC-509c" and locus.startswith("sessions[") and ".reason" in locus:
            # sessions[NAME].agents[REF].reason
            m = re.match(r"sessions\[([^\]]+)\]\.agents\[([^\]]+)\]\.reason", locus)
            if not m:
                return "FAIL", f"unparsed-locus {locus}"
            name, ref = m.group(1), m.group(2)
            sessions = doc.get("sessions") if isinstance(doc, dict) else None
            if not isinstance(sessions, list):
                return "FAIL", "no sessions"
            sess = next((s for s in sessions if isinstance(s, dict) and s.get("name") == name), None)
            if sess is None:
                return "FAIL", f"session {name} absent"
            agents = sess.get("agents")
            if not isinstance(agents, list):
                return "FAIL", "no agents"
            ag = next((a for a in agents if isinstance(a, dict) and a.get("ref") == ref), None)
            if ag is None:
                return "FAIL", f"agent {ref} absent"
            got = ag.get("reason")
            # JSON null vs missing
            ok = got == want
            return ("PASS" if ok else "FAIL"), f"reason={got!r} want={want!r}"
        if oid == "SC-509e":
            return "FAIL", "should be UNSCORABLE"
        return "FAIL", f"unhandled digest {oid} {locus} {pred}"

    if stream == "stderr":
        text = stderr.decode("utf-8", "replace")
        if pred == "at-least":
            nums = [int(x) for x in re.findall(r"\b(\d+)\b", text)]
            try:
                need = int(want)
            except ValueError:
                need = 1
            ok = bool(text.strip()) and any(n >= need for n in nums)
            return ("PASS" if ok else "FAIL"), f"stderr={text.strip()[:80]!r} nums={nums}"
        return "FAIL", f"unhandled stderr {pred}"

    if stream == "stdout":
        text = stdout.decode("utf-8", "replace")
        if oid == "SC-017o" and pred == "at-least":
            nums = [int(x) for x in re.findall(r"\b(\d+)\b", text)]
            try:
                need = int(want)
            except ValueError:
                need = 1
            ok = any(n >= need for n in nums)
            return ("PASS" if ok else "FAIL"), f"stdout-nums={nums}"
        if oid == "SC-017l" and pred == "all-of":
            rows = human_sessions(text)
            if not rows:
                return "FAIL", "no-human-session-rows"
            ok = all(st == want for _n, st in rows)
            return ("PASS" if ok else "FAIL"), f"human_status={[st for _n,st in rows]!r}"
        if oid == "SC-017m" and pred == "present":
            rows = human_sessions(text)
            ok = any(st == "unknown" for _n, st in rows)
            return ("PASS" if ok else "FAIL"), f"human_rows={rows!r}"
        return "FAIL", f"unhandled stdout {oid} {locus} {pred}"

    return "FAIL", f"unhandled stream {stream}"


def main() -> int:
    # This is intentionally before opening output: an incoherent generated tuple
    # has no scorer result.  `snapshot.obligations` is the one read whose hash the
    # FRESHNESS file bound; do not turn this into a verifier subprocess plus a
    # second OBLIGATIONS open.
    try:
        snapshot = read_generated_tuple(REPO)
        _, obls = parse_tsv_bytes(snapshot.obligations, "saved OBLIGATIONS snapshot")
    except ArtifactTupleError as error:
        print(error, file=sys.stderr)
        return 2
    _, inv = load_tsv(INV)
    p1 = [r for r in inv if r["phase"] == "P1"]
    index = {}
    for i, r in enumerate(p1, 1):
        case_dir = str(Path(r["case"]).parent)
        index[(case_dir, r["consumer"])] = i

    out_path = RUN / "obligation-scores.tsv"
    n_obs = n_uns = n_pass = n_fail = n_abort = 0
    by_id = {}
    with out_path.open("w", encoding="utf-8") as fh:
        fh.write(f"# generated_tuple\t{snapshot.identity}\n")
        fh.write("case\tconsumer\tobligation_id\tlocus\tsupport\tverdict\tdetail\n")
        for obl in obls:
            key = (obl["case"], obl["consumer"])
            idx = index.get(key)
            if obl["support"] == "UNSCORABLE":
                n_uns += 1
                fh.write(
                    f"{obl['case']}\t{obl['consumer']}\t{obl['obligation_id']}\t"
                    f"{obl['locus']}\tUNSCORABLE\tUNSCORABLE\tnot-scored\n"
                )
                continue
            n_obs += 1
            if idx is None:
                n_abort += 1
                fh.write(
                    f"{obl['case']}\t{obl['consumer']}\t{obl['obligation_id']}\t"
                    f"{obl['locus']}\tOBSERVED\tFIXTURE-ABORT\tno-p1-invocation\n"
                )
                continue
            cap = cap_dir(idx, obl["consumer"])
            if not cap.is_dir():
                n_abort += 1
                fh.write(
                    f"{obl['case']}\t{obl['consumer']}\t{obl['obligation_id']}\t"
                    f"{obl['locus']}\tOBSERVED\tFIXTURE-ABORT\tno-capture\n"
                )
                continue
            verdict, detail = score_one(obl, cap)
            if verdict == "PASS":
                n_pass += 1
            elif verdict == "FIXTURE-ABORT":
                n_abort += 1
            else:
                n_fail += 1
            oid = obl["obligation_id"]
            by_id.setdefault(oid, {"PASS": 0, "FAIL": 0, "ABORT": 0})
            by_id[oid]["PASS" if verdict == "PASS" else ("ABORT" if verdict == "FIXTURE-ABORT" else "FAIL")] += 1
            detail = detail.replace("\t", " ").replace("\n", " ")
            fh.write(
                f"{obl['case']}\t{obl['consumer']}\t{obl['obligation_id']}\t"
                f"{obl['locus']}\tOBSERVED\t{verdict}\t{detail}\n"
            )
    print(f"OBSERVED {n_obs} PASS={n_pass} FAIL={n_fail} FIXTURE-ABORT={n_abort}")
    print(f"UNSCORABLE {n_uns} preserved")
    for oid, c in sorted(by_id.items()):
        print(f"  {oid} PASS={c['PASS']} FAIL={c['FAIL']} ABORT={c['ABORT']}")
    print(f"wrote {out_path}")
    return 0


def redproof() -> None:
    """Exercise main's actual no-output refusal and its saved-tuple attribution."""
    global INV, RUN, read_generated_tuple
    original_inv, original_run, original_reader = INV, RUN, read_generated_tuple
    try:
        with tempfile.TemporaryDirectory(prefix="ae-score-tuple-") as temp:
            root = Path(temp)
            INV = root / "INVOCATIONS.tsv"
            INV.write_text("phase\tcase\tconsumer\nP1\tarms/X/case.tsv\tview\n", encoding="utf-8")
            RUN = root / "out"
            RUN.mkdir()
            out_path = RUN / "obligation-scores.tsv"

            def refuse(_repo: Path) -> ArtifactTuple:
                raise ArtifactTupleError("ARTIFACT-TUPLE RED fixture")

            read_generated_tuple = refuse
            refused_stdout, refused_stderr = StringIO(), StringIO()
            with redirect_stdout(refused_stdout), redirect_stderr(refused_stderr):
                refusal_rc = main()
            if (
                refusal_rc != 2
                or out_path.exists()
                or refused_stdout.getvalue()
                or "ARTIFACT-TUPLE RED fixture" not in refused_stderr.getvalue()
            ):
                raise RuntimeError("REDPROOF tuple refusal wrote a scorer output")
            print("RED scorer tuple refusal: named stderr, no output")

            fields = {
                "contract_blob": "a" * 40,
                "obligations_sha256": "b" * 64,
                "added_roster_gap_sha256": "c" * 64,
                "sc509c_unproved_sha256": "d" * 64,
            }
            snapshot = ArtifactTuple(
                obligations=b"case\tconsumer\n",
                added_roster_gap=b"",
                sc509c_unproved=b"",
                freshness=b"",
                fields=fields,
            )
            read_generated_tuple = lambda _repo: snapshot
            if main() != 0:
                raise RuntimeError("REDPROOF valid saved tuple did not score")
            first = out_path.read_text(encoding="utf-8").splitlines()[0]
            if first != f"# generated_tuple\t{snapshot.identity}":
                raise RuntimeError(f"REDPROOF scorer lost tuple identity: {first!r}")
            print("GREEN scorer saved tuple parsed and attributed")
    finally:
        INV, RUN, read_generated_tuple = original_inv, original_run, original_reader
    print("SCORE-OBLIGATIONS-TUPLE-REDPROOF PASS")


if __name__ == "__main__":
    if sys.argv[1:] == ["redproof"]:
        redproof()
    else:
        raise SystemExit(main())
