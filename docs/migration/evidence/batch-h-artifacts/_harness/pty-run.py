#!/usr/bin/env python3
"""Run a command with stdin attached to a REAL pty, so `[[ -t 0 ]]` is true.

`say` branches on whether stdin is a terminal (ae:14473). A redirected empty stdin and a
real TTY are different inputs, and only a pty can present the second.
"""
import os, pty, sys
pid, fd = pty.fork()
if pid == 0:
    os.execv(sys.argv[1], sys.argv[1:])
out = b""
try:
    while True:
        b = os.read(fd, 4096)
        if not b: break
        out += b
except OSError:
    pass
_, status = os.waitpid(pid, 0)
sys.stdout.write(out.decode(errors="replace"))
sys.exit(os.waitstatus_to_exitcode(status) if hasattr(os, "waitstatus_to_exitcode") else (status >> 8))
