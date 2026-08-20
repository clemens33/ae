#!/usr/bin/env python3
"""Whole-cohort schema mutation over an events.jsonl fixture.

Byte-level edits on the raw line (no re-serialisation, so every unrelated byte is
preserved exactly), followed by the design's mutation-validity self-check:
  remove : json(mutated) == json(control) with the named key deleted
  rename : json(mutated) == json(control) with the named key renamed to <key>_x
  insert : json(mutated) == json(control) plus exactly the inserted pair, and the
           inserted key occupies the requested object position
A line failing its self-check makes the run a harness defect; the tool exits 3 and
names the line. Emits no verdict about any consumer.
"""
import json, sys, argparse
from collections import OrderedDict


def top_level_pairs(line):
    """[(key, key_start, key_end, pair_start, pair_end)] for a flat JSON object.

    Offsets are byte/char offsets into `line`; pair_start is the opening quote of the
    key, pair_end is one past the last char of the value.
    """
    s = line
    i = 0
    n = len(s)
    while i < n and s[i] in ' \t':
        i += 1
    if i >= n or s[i] != '{':
        raise ValueError('not a JSON object')
    i += 1
    out = []
    while True:
        while i < n and s[i] in ' \t\r\n':
            i += 1
        if i < n and s[i] == '}':
            return out
        if i >= n:
            raise ValueError('unterminated object')
        if s[i] != '"':
            raise ValueError('expected key quote at %d' % i)
        pair_start = i
        key_start = i
        i, key = scan_string(s, i)
        key_end = i
        while i < n and s[i] in ' \t':
            i += 1
        if s[i] != ':':
            raise ValueError('expected colon at %d' % i)
        i += 1
        while i < n and s[i] in ' \t':
            i += 1
        i = scan_value(s, i)
        out.append((key, key_start, key_end, pair_start, i))
        while i < n and s[i] in ' \t':
            i += 1
        if i < n and s[i] == ',':
            i += 1
            continue
        if i < n and s[i] == '}':
            return out
        raise ValueError('expected , or } at %d' % i)


def scan_string(s, i):
    assert s[i] == '"'
    i += 1
    buf = []
    while True:
        c = s[i]
        if c == '\\':
            buf.append(s[i:i + 2]); i += 2; continue
        if c == '"':
            return i + 1, json.loads('"' + ''.join(buf) + '"')
        buf.append(c); i += 1


def scan_value(s, i):
    c = s[i]
    if c == '"':
        j, _ = scan_string(s, i)
        return j
    if c in '-0123456789':
        j = i
        while j < len(s) and s[j] in '-+.eE0123456789':
            j += 1
        return j
    for lit in ('true', 'false', 'null'):
        if s.startswith(lit, i):
            return i + len(lit)
    if c in '{[':
        depth = 0
        j = i
        while j < len(s):
            ch = s[j]
            if ch == '"':
                j, _ = scan_string(s, j); continue
            if ch in '{[':
                depth += 1
            elif ch in '}]':
                depth -= 1
                if depth == 0:
                    return j + 1
            j += 1
    raise ValueError('bad value at %d' % i)


def remove_key(line, key):
    pairs = top_level_pairs(line)
    idx = [k for k, _, _, _, _ in pairs].index(key)
    _, _, _, ps, pe = pairs[idx]
    if idx > 0:
        cut_from = pairs[idx - 1][4]          # end of previous value -> eats the comma
        return line[:cut_from] + line[pe:]
    if len(pairs) > 1:
        cut_to = pairs[idx + 1][3]            # start of next key -> eats the comma
        return line[:ps] + line[cut_to:]
    return line[:ps] + line[pe:]


def rename_key(line, key, newkey):
    pairs = top_level_pairs(line)
    idx = [k for k, _, _, _, _ in pairs].index(key)
    _, ks, ke, _, _ = pairs[idx]
    return line[:ks] + json.dumps(newkey) + line[ke:]


def insert_key(line, key, value, where):
    pairs = top_level_pairs(line)
    frag = '%s:%s' % (json.dumps(key), json.dumps(value))
    if not pairs:
        raise ValueError('empty object')
    if where == 'first':
        at = pairs[0][3]
        return line[:at] + frag + ',' + line[at:]
    if where == 'last':
        at = pairs[-1][4]
        return line[:at] + ',' + frag + line[at:]
    if where == 'middle':
        mid = len(pairs) // 2
        at = pairs[mid][3]
        return line[:at] + frag + ',' + line[at:]
    raise ValueError('bad position')


def decode(line):
    return json.loads(line, object_pairs_hook=OrderedDict)


def selfcheck(op, key, control, mutated, newkey=None, value=None, where=None):
    c = decode(control)
    m = decode(mutated)
    if op == 'remove':
        exp = OrderedDict((k, v) for k, v in c.items() if k != key)
        return (dict(m) == dict(exp)), 'decoded(mutated) == decoded(control) minus %r' % key
    if op == 'rename':
        exp = OrderedDict(((newkey if k == key else k), v) for k, v in c.items())
        return (dict(m) == dict(exp)), 'decoded(mutated) == decoded(control) with %r renamed to %r' % (key, newkey)
    if op == 'insert':
        exp = dict(c); exp[key] = value
        if dict(m) != exp:
            return False, 'decoded(mutated) != decoded(control) plus the inserted pair'
        keys = list(m.keys())
        pos = keys.index(key)
        ok = {'first': pos == 0,
              'last': pos == len(keys) - 1,
              'middle': 0 < pos < len(keys) - 1}[where]
        return ok, 'inserted key at object position %d of %d (requested %s)' % (pos, len(keys), where)
    raise ValueError(op)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--in', dest='src', required=True)
    ap.add_argument('--out', dest='dst', required=True)
    ap.add_argument('--report', required=True)
    ap.add_argument('--op', required=True, choices=['remove', 'rename', 'insert'])
    ap.add_argument('--key', required=True)
    ap.add_argument('--newkey')
    ap.add_argument('--value')
    ap.add_argument('--where', choices=['first', 'middle', 'last'])
    a = ap.parse_args()

    src = open(a.src, 'r', encoding='utf-8').read()
    lines = src.split('\n')
    trailing_newline = lines and lines[-1] == ''
    if trailing_newline:
        lines = lines[:-1]

    out, rep = [], []
    cohort = 0
    failures = 0
    for i, line in enumerate(lines, 1):
        if not line.strip():
            out.append(line); continue
        try:
            keys = [k for k, _, _, _, _ in top_level_pairs(line)]
        except Exception as e:
            out.append(line)
            rep.append('line %d: UNPARSED (%s) — left untouched' % (i, e))
            continue
        if a.op == 'insert':
            member = True
        else:
            member = a.key in keys
        if not member:
            out.append(line)
            rep.append('line %d: not in cohort (no %r)' % (i, a.key))
            continue
        cohort += 1
        if a.op == 'remove':
            mut = remove_key(line, a.key)
            ok, how = selfcheck('remove', a.key, line, mut)
        elif a.op == 'rename':
            mut = rename_key(line, a.key, a.newkey)
            ok, how = selfcheck('rename', a.key, line, mut, newkey=a.newkey)
        else:
            mut = insert_key(line, a.key, a.value, a.where)
            ok, how = selfcheck('insert', a.key, line, mut, value=a.value, where=a.where)
        out.append(mut)
        if not ok:
            failures += 1
        rep.append('line %d: cohort member; selfcheck=%s (%s)\n  pre : %s\n  post: %s'
                   % (i, 'PASS' if ok else 'FAIL', how, line, mut))

    body = '\n'.join(out) + ('\n' if trailing_newline else '')
    open(a.dst, 'w', encoding='utf-8').write(body)
    with open(a.report, 'w', encoding='utf-8') as f:
        f.write('op=%s key=%s newkey=%s value=%r where=%s\n' % (a.op, a.key, a.newkey, a.value, a.where))
        f.write('input=%s output=%s\n' % (a.src, a.dst))
        f.write('lines=%d cohort=%d selfcheck_failures=%d\n' % (len(lines), cohort, failures))
        f.write('\n'.join(rep) + '\n')
    print('cohort=%d failures=%d' % (cohort, failures))
    sys.exit(3 if failures else 0)


if __name__ == '__main__':
    main()
