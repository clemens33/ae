#!/usr/bin/env python3
"""D4: the SC-504b capture, run against a DELIBERATELY CHOSEN SIGPIPE disposition.

Same shape as the SC-504b harness — producer and early-closing consumer as
separately supervised processes, an explicit pipe, one line read, read end
closed — but the disposition handed to the producer is a parameter:

  dfl      SIG_DFL, explicitly reset by this harness (what SC-504b did)
  ign      SIG_IGN, DELIBERATELY LEAKED by this harness (the ability-to-fail control)
  inherit  nothing set at all — whatever this harness itself carried

The point is to show the SAME capture reports differently for a leaked SIG_IGN.
It does NOT, and cannot, say whether ae leaks one: this harness sets the
disposition before exec, so what it reports is what IT handed the producer.
"""
import os, sys, json, signal

out_dir, mode = sys.argv[1], sys.argv[2]
argv = sys.argv[3:]
env = {}
for kv in os.environ.get('AE_L_SIGPIPE_ENV', '').split('\x1f'):
    if '=' in kv:
        k, v = kv.split('=', 1)
        env[k] = v

r, w = os.pipe()
err_fd = os.open(os.path.join(out_dir, 'producer.%s.stderr' % mode),
                 os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
pid = os.fork()
if pid == 0:
    os.close(r)
    os.dup2(w, 1); os.dup2(err_fd, 2)
    os.close(w); os.close(err_fd)
    devnull = os.open(os.devnull, os.O_RDONLY); os.dup2(devnull, 0)
    if mode == 'dfl':
        signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    elif mode == 'ign':
        signal.signal(signal.SIGPIPE, signal.SIG_IGN)
    # 'inherit': set nothing — CPython's own default for SIGPIPE is SIG_IGN
    os.execve(argv[0], argv, env)
    os._exit(127)

os.close(w); os.close(err_fd)
first = b''
f = os.fdopen(r, 'rb', buffering=0)
while True:
    ch = f.read(1)
    if not ch:
        break
    first += ch
    if ch == b'\n':
        break
try:
    f.close()
except OSError:
    pass

_, status = os.waitpid(pid, 0)
rec = {
    'mode': mode,
    'disposition_set_by_harness': {'dfl': 'SIG_DFL', 'ign': 'SIG_IGN (leaked)',
                                   'inherit': '(nothing set)'}[mode],
    'producer_argv': argv,
    'consumer_read_first_line': first.decode('utf-8', 'replace'),
    'consumer_closed_read_end_after': 'one line',
    'raw_wait_status': status,
    'producer_exited_normally': os.WIFEXITED(status),
    'producer_exit_code': os.WEXITSTATUS(status) if os.WIFEXITED(status) else None,
    'producer_signalled': os.WIFSIGNALED(status),
    'producer_term_signal': os.WTERMSIG(status) if os.WIFSIGNALED(status) else None,
    'producer_term_signal_name': (signal.Signals(os.WTERMSIG(status)).name
                                  if os.WIFSIGNALED(status) else None),
    'attribution_limit': ('this harness set the disposition before exec, so this record '
                          'reports what the HARNESS handed the producer, never what ae '
                          'would have leaked'),
}
with open(os.path.join(out_dir, 'sigpipe-record.%s.json' % mode), 'w') as fh:
    json.dump(rec, fh, indent=2); fh.write('\n')
with open(os.path.join(out_dir, 'producer.%s.stdout.firstline' % mode), 'wb') as fh:
    fh.write(first)
print(json.dumps(rec))
