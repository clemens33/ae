#!/usr/bin/env python3
"""SC-504b: producer and early-closing consumer as SEPARATELY SUPERVISED processes.

No shell pipeline is placed over the subject. The consumer is this process: it
creates an explicit pipe, hands the write end to the producer, reads ONE line,
closes the read end, and then reports BOTH statuses and the producer's signal
disposition.
"""
import os, sys, json, signal

out_dir = sys.argv[1]
argv = sys.argv[2:]
env = {}
for kv in os.environ.get('AE_L_SIGPIPE_ENV', '').split('\x1f'):
    if '=' in kv:
        k, v = kv.split('=', 1)
        env[k] = v

r, w = os.pipe()
err_fd = os.open(os.path.join(out_dir, 'producer.stderr'), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
pid = os.fork()
if pid == 0:
    os.close(r)
    os.dup2(w, 1)
    os.dup2(err_fd, 2)
    os.close(w)
    os.close(err_fd)
    devnull = os.open(os.devnull, os.O_RDONLY)
    os.dup2(devnull, 0)
    # SIGPIPE restored to default in the child, as any exec'd program gets it
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    os.execve(argv[0], argv, env)
    os._exit(127)

os.close(w)
os.close(err_fd)
first = b''
with os.fdopen(r, 'rb', buffering=0) as f:
    while True:
        ch = f.read(1)
        if not ch:
            break
        first += ch
        if ch == b'\n':
            break
# THE CONSUMER CLOSES EARLY, here, with the producer still running.
# (the with-block close happens on exit; force it now)
try:
    os.close(r)
except OSError:
    pass

_, status = os.waitpid(pid, 0)
rec = {
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
    'consumer_exit_code': 0,
}
with open(os.path.join(out_dir, 'sigpipe-record.json'), 'w') as fh:
    json.dump(rec, fh, indent=2)
    fh.write('\n')
with open(os.path.join(out_dir, 'producer.stdout.firstline'), 'wb') as fh:
    fh.write(first)
print(json.dumps(rec))
