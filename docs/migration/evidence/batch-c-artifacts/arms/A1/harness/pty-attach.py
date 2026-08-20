#!/usr/bin/env python3
"""Attach a scripted tmux client on a REAL pty, without needing a controlling tty.

`script(1)` cannot be used here: it calls tcgetattr on its own stdin, and a harness that
runs without a controlling terminal has none ("Operation not supported on socket"). This
forks a pty directly, gives the child the pty as its controlling terminal, and drains the
master side to a log so the client keeps running until it is killed.

usage: pty-attach.py <logfile> <argv...>
"""
import os, pty, signal, sys

log = sys.argv[1]
argv = sys.argv[2:]
pid, fd = pty.fork()
if pid == 0:
    os.execvp(argv[0], argv)
    os._exit(127)
try:
    # a real client needs a sane window size or tmux refuses to attach
    import fcntl, struct, termios
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
except Exception:
    pass
with open(log, "wb", buffering=0) as fh:
    def bye(*_):
        try: os.kill(pid, signal.SIGTERM)
        except Exception: pass
        os._exit(0)
    signal.signal(signal.SIGTERM, bye)
    signal.signal(signal.SIGINT, bye)
    while True:
        try:
            data = os.read(fd, 4096)
        except OSError:
            break
        if not data:
            break
        fh.write(data)
