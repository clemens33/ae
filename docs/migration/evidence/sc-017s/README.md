# SC-017s — the empirical column

`probe.sh` is the arm that closes SC-017s's Empirical column. `probe-run.txt` is its
captured output on the authoring host.

## What it proves, and what the row claims — these are not the same set

**Proved on measured bytes:**

- a live pane whose foreground command is outside the closed shell set → `alive`;
- a live pane at a shell → `unknown`;
- a pane retained by `remain-on-exit` (`pane_dead=1`) whose command field still names the
  exited process → `unknown`, **asserted**, not narrated;
- red-proof 1: the command-only predicate answers `alive` on those same bytes while the
  ratified two-field predicate answers `unknown` — so the `pane_dead` conjunct is shown to
  be load-bearing rather than asserted to be;
- red-proof 2: the frozen list-path set (ae:4201-4206) maps an empty reading to `alive`
  while `command_is_shell` (ae:428-434) maps it to `unknown`.

**Claimed by the row and NOT proved here:**

- the empty-reading case. Live tmux produced no empty `pane_current_command` in any arm,
  so that case is asserted at unit level against the predicate, not observed from tmux.
  Present / absent / **unobservable** are three different answers and this one is
  unobservable in this fixture.
- the association half. The probe plants `@ae_agent` markers to address panes; SC-017p's
  exact-association rule and SC-602's `@ae_slot`-is-identity point are out of its scope.
- anything about SC-017p's `dead` routes. The row is one-directional and so is the probe.

## Three outcomes, deliberately distinguished

`rc=0` the predicate answered correctly · `rc=1` **PRODUCT FAIL**, the predicate is wrong ·
`rc=2` **FIXTURE ABORT**, the world the probe built is not the world it meant to build, so
nothing in the run is evidence about the predicate.

That split is not decoration. Both earlier versions of this probe failed in a way that
presented as the other kind:

1. **Pane-index targeting.** `split-window` renumbering put the `@ae_agent` markers on the
   wrong panes, and the probe reported a confident product FAIL — plus a `CONFIRMED`
   control that was measuring nothing — that was entirely the fixture's.
2. **`sleep 300` through the default shell.** tmux runs the command through `$SHELL`, so
   on a fish host `pane_current_command` reported `fish` and the expected-alive arm got
   `unknown`. Measured by gpt56sol:colead on their host; it read as the predicate being
   wrong. Fixed with `exec sleep 30`, so the agent process replaces the shell.

Both are the same lesson from the errexit-probe entry in AGENTS.md: **the instrument must
be shown to answer correctly for a known case before it is trusted about an unknown one**,
and a broken instrument's first answer was a confident FAIL rather than silence.

## Preconditions are measured, not trusted

Before any verdict is read, the probe asserts: three markers present exactly once; the
non-shell pane's measured command is **outside** the closed shell set; the shell pane's is
**inside** it; the live pane reports `pane_dead=0`; and the exited pane reports
`pane_dead=1` with a **non-shell** command — the combination the whole `pane_dead` arm
depends on. Any of these failing is a `rc=2` abort with the measured value named.
A probe that cannot confirm its fixture landed is measuring its own assumption.

Both failure paths were exercised rather than assumed: a fixture whose non-shell pane runs
a shell aborts with `rc=2` naming the reading, and a predicate that ignores `pane_dead`
fails with `rc=1` on the exited pane.

## Portability

Not called deterministic. It needs `tmux`, a short socket path (macOS `sun_path` is 104
bytes), and `remain-on-exit` support. `exec` in the launch command is what makes
`pane_current_command` report the agent process rather than the host's login shell — the
one place a different host previously changed the answer.
