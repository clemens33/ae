# SC-017s — the empirical column

`probe.sh` is the arm that closes SC-017s's Empirical column. It is deterministic, needs
only tmux, and takes a few seconds. `probe-run.txt` is its captured output.

**It measures before it asserts.** Every tmux reading is captured and printed first, then
the predicate is applied to those readings. A fixture defect therefore surfaces as a
strange READING rather than as a product verdict — which matters, because the first
version of this probe targeted panes by index, `split-window` renumbering put the
`@ae_agent` markers on the wrong panes, and it reported a confident FAIL that was entirely
the fixture's. Panes are now addressed by the id `split-window -P -F '#{pane_id}'`
returns, and a marker-uniqueness check aborts the run before any verdict if the three
markers are not each present exactly once.

**What it found that the design did not anticipate.** A pane retained by `remain-on-exit`
reports `pane_dead=1` while `pane_current_command` still names the exited process. A
predicate over the command field ALONE therefore proves a dead agent `alive` — the #105
harm direction. That is why SC-017s carries the `#{pane_dead}` conjunct, and it is the one
part of the row that came from running the arm rather than from reading the artifact.
Neither ae@72c7293 nor the successor sets `remain-on-exit`, so the hazard is
operator-configurable rather than default; the guard costs one more field in a query the
row already makes.

**What it cannot show.** Live tmux did not produce an empty `pane_current_command` in any
arm here, so the empty-reading case is asserted at unit level against the predicate rather
than observed from tmux. Stated rather than smoothed over: present / absent / unobservable
are three different answers, and this one is unobservable in this fixture.

The control reproduces the frozen defect: the list-path set at ae:4201-4206 omits the empty
string that `command_is_shell` (ae:428-434) includes, so an empty reading maps to `alive`.
