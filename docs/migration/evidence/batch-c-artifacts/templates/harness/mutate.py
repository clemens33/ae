#!/usr/bin/env python3
"""Named byte-level mutations on a fixture file, with a recorded byte diff.

Usage:
  mutate.py <file> <out-diff.txt> <name> OP [args...]
Ops:
  repl <line_no> <old_bytes_utf8> <new_bytes_utf8>   replace first occurrence on that line
  delkey <line_no> <key>                             delete ,"key":"..." from that line
  droptail <n_bytes>                                 truncate n bytes from the end
  dropline <line_no>                                 delete the whole line (with its newline)
Every op rewrites <file> in place and appends a byte diff record to <out-diff.txt>.
"""
import hashlib, re, sys

def h(b): return hashlib.sha256(b).hexdigest()

path, diffpath, name, op = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
args = sys.argv[5:]
before = open(path, "rb").read()
lines = before.splitlines(keepends=True)

def line_span(n):
    off = sum(len(l) for l in lines[:n-1])
    return off, lines[n-1]

if op == "repl":
    n = int(args[0]); old = args[1].encode(); new = args[2].encode()
    off, ln = line_span(n)
    if old not in ln:
        sys.exit(f"MUTATION FAILED: {old!r} not on line {n}")
    lines[n-1] = ln.replace(old, new, 1)
    detail = f"line {n}: replace {old!r} -> {new!r} (first occurrence)"
elif op == "delkey":
    n = int(args[0]); key = args[1]
    off, ln = line_span(n)
    pat = re.compile((r',"%s":"(?:[^"\\]|\\.)*"' % re.escape(key)).encode())
    m = pat.search(ln)
    if not m:
        sys.exit(f"MUTATION FAILED: key {key} not on line {n}")
    lines[n-1] = ln[:m.start()] + ln[m.end():]
    detail = f'line {n}: delete member {m.group(0)!r}'
elif op == "droptail":
    k = int(args[0])
    after = before[:-k] if k else before
    detail = f"truncate {k} trailing byte(s): {before[-k:]!r}"
    open(path, "wb").write(after)
    lines = None
elif op == "dropline":
    n = int(args[0])
    off, ln = line_span(n)
    detail = f"delete line {n} ({len(ln)} bytes): {ln!r}"
    del lines[n-1]
else:
    sys.exit(f"unknown op {op}")

if lines is not None:
    after = b"".join(lines)
    open(path, "wb").write(after)

with open(diffpath, "a") as fh:
    fh.write(f"## mutation: {name}\n")
    fh.write(f"file: {path}\n")
    fh.write(f"op: {op} {' '.join(repr(a) for a in args)}\n")
    fh.write(f"detail: {detail}\n")
    fh.write(f"before: sha256={h(before)} bytes={len(before)}\n")
    fh.write(f"after:  sha256={h(after)} bytes={len(after)}\n")
    fh.write("byte-diff:\n")
    # minimal common-prefix/suffix diff
    i = 0
    while i < min(len(before), len(after)) and before[i] == after[i]:
        i += 1
    j = 0
    while j < min(len(before), len(after)) - i and before[len(before)-1-j] == after[len(after)-1-j]:
        j += 1
    fh.write(f"  common_prefix_bytes={i} common_suffix_bytes={j}\n")
    fh.write(f"  removed={before[i:len(before)-j]!r}\n")
    fh.write(f"  inserted={after[i:len(after)-j]!r}\n\n")
print(f"MUTATED {name}: {h(before)[:12]} -> {h(after)[:12]}")
