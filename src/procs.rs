//! The process-table snapshot and its descendant tree — the one non-tmux read
//! the watchdog's dead-check needs, kept PURE here and spawned through the
//! single sealed `ps` door in [`crate::transport`].

use std::collections::{HashMap, HashSet};

/// The fixed argv for the process-table snapshot, sealed the way
/// [`crate::git::GitArgv`] is: the inner vector is private, so no other module
/// can fabricate a `ps` command line and hand it to
/// [`crate::transport::run_ps`].
pub struct PsArgv(Vec<String>);

impl PsArgv {
    /// The snapshot argv.
    #[must_use]
    pub fn snapshot() -> Self {
        Self(vec![
            "-A".to_owned(),
            "-o".to_owned(),
            "pid=,ppid=,comm=".to_owned(),
        ])
    }

    /// The argv reading ONE process's controlling tty — frozen's `ps -o tty= -p
    /// $$`, which `ae next --attach` uses to tell a real pane from an inherited
    /// `$TMUX`.
    #[must_use]
    pub fn tty_of(pid: u32) -> Self {
        Self(vec![
            "-o".to_owned(),
            "tty=".to_owned(),
            "-p".to_owned(),
            pid.to_string(),
        ])
    }

    /// The argv for the transport door to spawn.
    #[must_use]
    pub fn as_args(&self) -> &[String] {
        &self.0
    }
}

/// One row of the process table: a pid, its parent, and its command name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
}

/// Whether the agent named by the slot's binary runs beneath a pane — the third
/// state, `Unknown`, is the snapshot that could not be taken or parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Descendancy {
    /// A descendant process is named the agent binary — the agent is alive.
    Present,
    /// The snapshot is good and no descendant matches — the agent is gone.
    Absent,
    /// No usable snapshot; the dead-check must NOT fire on this.
    Unknown,
}

/// Parse `ps -A -o pid=,ppid=,comm=` output into rows, or `None` if ANY
/// non-blank line is not `<pid> <ppid> <comm>` with numeric pids and a
/// non-empty command.
#[allow(
    clippy::similar_names,
    reason = "pid and ppid are the canonical process-table field names; renaming them to satisfy the lint would obscure the domain, not clarify it"
)]
#[must_use]
pub fn parse_table(raw: &str) -> Option<Vec<Proc>> {
    let mut out = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let (pid_str, rest) = split_first_token(line)?;
        let (ppid_str, comm) = split_first_token(rest)?;
        let comm = comm.trim();
        if comm.is_empty() {
            return None;
        }
        let pid = pid_str.parse::<u32>().ok()?;
        let ppid = ppid_str.parse::<u32>().ok()?;
        out.push(Proc {
            pid,
            ppid,
            comm: comm.to_owned(),
        });
    }
    Some(out)
}

/// The first whitespace-delimited token of `s`, and the remainder after the
/// whitespace run.
fn split_first_token(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    match s.find(char::is_whitespace) {
        Some(i) => Some((&s[..i], &s[i..])),
        None => Some((s, "")),
    }
}

/// Whether a process named `agent_bin` is a descendant of `pane_pid` in the
/// snapshot — one ancestor walk for the whole table rather than one per
/// candidate.
#[must_use]
pub fn has_descendant_named(procs: &[Proc], pane_pid: u32, agent_bin: &str) -> bool {
    let mut children: HashMap<u32, Vec<usize>> = HashMap::new();
    for (idx, p) in procs.iter().enumerate() {
        children.entry(p.ppid).or_default().push(idx);
    }
    let mut visited: HashSet<u32> = HashSet::new();
    let mut stack: Vec<u32> = vec![pane_pid];
    while let Some(pid) = stack.pop() {
        if !visited.insert(pid) {
            continue;
        }
        let Some(kids) = children.get(&pid) else {
            continue;
        };
        for &idx in kids {
            let child = &procs[idx];
            if name_matches(&child.comm, agent_bin) {
                return true;
            }
            stack.push(child.pid);
        }
    }
    false
}

/// Whether `pid` runs BENEATH `ancestor` — the walk [`has_descendant_named`]
/// makes downwards, made UPWARDS from one known pid.
///
/// A seat cannot reseat itself, and the pane id is only half of that rule: a
/// caller the target's own tool started may carry no `$TMUX_PANE` to compare,
/// and the process tree is the other half. Walks up with a visited set,
/// because a damaged table can name a cycle and this may not hang.
#[must_use]
pub(crate) fn is_descendant_of(procs: &[Proc], ancestor: u32, pid: u32) -> bool {
    let parents: HashMap<u32, u32> = procs.iter().map(|proc| (proc.pid, proc.ppid)).collect();
    let mut visited: HashSet<u32> = HashSet::new();
    let mut current = pid;
    while visited.insert(current) {
        let Some(&parent) = parents.get(&current) else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        current = parent;
    }
    false
}

/// Whether ANY process at all runs beneath `pane_pid` — the question "is this
/// pane's shell busy", which [`has_descendant_named`] cannot answer because it
/// asks about ONE name.
///
/// A descendant at any depth implies a direct CHILD, so this is one pass over
/// the table rather than a second tree walk.
///
/// `None` for the pid is FALSE, never "busy": a pane whose pid could not be
/// read is a gap in the liveness proof, and the liveness owner refuses it as
/// unproven. Answering "busy" here would spend that gap on the wrong refusal.
#[must_use]
pub(crate) fn has_any_descendant(procs: &[Proc], pane_pid: Option<u32>) -> bool {
    let Some(pane_pid) = pane_pid else {
        return false;
    };
    procs.iter().any(|proc| proc.ppid == pane_pid)
}

/// Compose a snapshot into a [`Descendancy`]: `None` (no usable snapshot) is
/// `Unknown`; a good snapshot is `Present`/`Absent` by the descendant walk.
#[must_use]
pub fn descendancy(table: Option<&[Proc]>, pane_pid: u32, agent_bin: &str) -> Descendancy {
    match table {
        None => Descendancy::Unknown,
        Some(procs) => {
            if has_descendant_named(procs, pane_pid, agent_bin) {
                Descendancy::Present
            } else {
                Descendancy::Absent
            }
        }
    }
}

/// Basename compare tolerant of a trailing `.exe` on either operand — the ONE
/// name equality, shared by the descendant walk and by every caller that
/// compares a foreground command with a recorded binary.
#[must_use]
pub(crate) fn name_matches(comm: &str, agent_bin: &str) -> bool {
    let base = comm.rsplit('/').next().unwrap_or(comm);
    strip_exe(base) == strip_exe(agent_bin)
}

fn strip_exe(s: &str) -> &str {
    s.strip_suffix(".exe").unwrap_or(s)
}

/// The live process table, parsed — `None` when `ps` could not be run or its
/// output did not parse (both are [`Descendancy::Unknown`] upstream, never a
/// dead agent).
#[must_use]
pub fn snapshot() -> Option<Vec<Proc>> {
    let (succeeded, stdout) = crate::transport::run_ps(&PsArgv::snapshot());
    if succeeded {
        parse_table(&stdout)
    } else {
        None
    }
}

/// This process's controlling tty, or `None` when it has none.
#[must_use]
pub fn own_tty() -> Option<String> {
    let (succeeded, stdout) = crate::transport::run_ps(&PsArgv::tty_of(std::process::id()));
    if !succeeded {
        return None;
    }
    let tty: String = stdout.chars().filter(|ch| !ch.is_whitespace()).collect();
    match tty.as_str() {
        "" | "?" | "??" | "-" => None,
        _ => Some(tty),
    }
}

#[cfg(test)]
mod tests {
    use super::{Descendancy, Proc, PsArgv, descendancy, has_descendant_named, parse_table};

    #[test]
    fn any_descendant_answers_busy_and_a_pidless_pane_is_never_busy() {
        let procs = vec![
            Proc {
                pid: 10,
                ppid: 1,
                comm: "bash".to_owned(),
            },
            Proc {
                pid: 11,
                ppid: 10,
                comm: "vim".to_owned(),
            },
            Proc {
                pid: 20,
                ppid: 1,
                comm: "sh".to_owned(),
            },
        ];
        assert!(
            super::has_any_descendant(&procs, Some(10)),
            "a child of the pane's shell is the pane being busy, whatever it is named"
        );
        assert!(
            !super::has_any_descendant(&procs, Some(20)),
            "a shell with no child is an IDLE shell"
        );
        // THE GAP THIS PIN GUARDS: a pane whose pid could not be read is not
        // busy, so the liveness owner still gets to refuse it as unproven and
        // say which gap. Answering "busy" would spend the gap on the wrong line.
        assert!(!super::has_any_descendant(&procs, None));
        assert!(!super::has_any_descendant(&[], Some(10)));
    }

    #[test]
    fn parses_macos_full_paths_keeping_the_command_intact() {
        // macOS right-justifies pids and `comm` is a full path.
        let raw = "  501   500 /opt/homebrew/bin/fish\n  777   501 /opt/homebrew/bin/node\n";
        let procs = parse_table(raw).expect("well-formed table parses");
        assert_eq!(
            procs,
            vec![
                Proc {
                    pid: 501,
                    ppid: 500,
                    comm: "/opt/homebrew/bin/fish".to_owned()
                },
                Proc {
                    pid: 777,
                    ppid: 501,
                    comm: "/opt/homebrew/bin/node".to_owned()
                },
            ]
        );
    }

    #[test]
    fn parses_linux_bare_truncated_comm() {
        let raw = "1 0 systemd\n501 1 fish\n";
        let procs = parse_table(raw).expect("bare comm parses");
        assert_eq!(procs.len(), 2);
        assert_eq!(procs[1].comm, "fish");
    }

    #[test]
    fn refuses_any_malformed_row_to_none() {
        // Non-numeric pid, non-numeric ppid, and a row missing the command are
        // each a strict refusal — a shifted table must not become a guess.
        assert_eq!(parse_table("notapid 0 fish\n"), None);
        assert_eq!(
            parse_table("501 fish claude\n"),
            None,
            "ppid must be numeric"
        );
        assert_eq!(
            parse_table("501 500\n"),
            None,
            "a row with no command is refused"
        );
    }

    #[test]
    fn skips_blank_trailing_lines_rather_than_refusing_them() {
        let procs = parse_table("501 1 fish\n\n   \n").expect("blank lines are not malformed");
        assert_eq!(procs.len(), 1);
    }

    #[test]
    fn finds_an_agent_running_two_hops_under_the_pane() {
        // Pane(100) -> bash(200) -> claude(300): the wrapper case the whole
        // descendant probe exists for.
        let procs = vec![
            Proc {
                pid: 100,
                ppid: 1,
                comm: "fish".to_owned(),
            },
            Proc {
                pid: 200,
                ppid: 100,
                comm: "bash".to_owned(),
            },
            Proc {
                pid: 300,
                ppid: 200,
                comm: "/opt/homebrew/bin/claude".to_owned(),
            },
        ];
        assert!(
            has_descendant_named(&procs, 100, "claude"),
            "grandchild agent is found by basename"
        );
        assert!(
            !has_descendant_named(&procs, 100, "codex"),
            "an unrelated name is not found"
        );
    }

    #[test]
    fn an_agent_under_a_different_pane_is_not_a_descendant() {
        let procs = vec![
            Proc {
                pid: 100,
                ppid: 1,
                comm: "fish".to_owned(),
            },
            Proc {
                pid: 400,
                ppid: 1,
                comm: "fish".to_owned(),
            },
            Proc {
                pid: 401,
                ppid: 400,
                comm: "claude".to_owned(),
            },
        ];
        assert!(
            !has_descendant_named(&procs, 100, "claude"),
            "another pane's agent is not ours"
        );
        assert!(has_descendant_named(&procs, 400, "claude"));
    }

    #[test]
    fn tolerates_the_opencode_exe_suffix_both_directions() {
        let procs = vec![
            Proc {
                pid: 100,
                ppid: 1,
                comm: "fish".to_owned(),
            },
            Proc {
                pid: 200,
                ppid: 100,
                comm: "opencode.exe".to_owned(),
            },
        ];
        assert!(
            has_descendant_named(&procs, 100, "opencode"),
            "opencode.exe matches roster opencode"
        );
    }

    #[test]
    fn a_ppid_cycle_cannot_loop_the_walk() {
        // Malformed: 100 and 200 are each other's parent.
        let procs = vec![
            Proc {
                pid: 100,
                ppid: 200,
                comm: "bash".to_owned(),
            },
            Proc {
                pid: 200,
                ppid: 100,
                comm: "bash".to_owned(),
            },
        ];
        assert!(
            !has_descendant_named(&procs, 100, "claude"),
            "no agent, and no infinite loop"
        );
    }

    #[test]
    fn the_snapshot_argv_is_the_cross_platform_spelling() {
        // Pinned: -A selects all processes on POSIX/GNU/BSD; the = empty header
        // suppresses titles; comm is the one field portable across both targets.
        assert_eq!(
            PsArgv::snapshot().as_args(),
            ["-A", "-o", "pid=,ppid=,comm="]
        );
    }

    #[test]
    fn descendancy_maps_none_to_unknown_and_a_snapshot_to_present_or_absent() {
        let procs = vec![
            Proc {
                pid: 100,
                ppid: 1,
                comm: "fish".to_owned(),
            },
            Proc {
                pid: 200,
                ppid: 100,
                comm: "codex".to_owned(),
            },
        ];
        assert_eq!(
            descendancy(None, 100, "codex"),
            Descendancy::Unknown,
            "no snapshot is never dead"
        );
        assert_eq!(
            descendancy(Some(&procs), 100, "codex"),
            Descendancy::Present
        );
        assert_eq!(
            descendancy(Some(&procs), 100, "claude"),
            Descendancy::Absent
        );
    }

    #[test]
    fn a_caller_under_the_target_tool_is_found_and_a_cycle_does_not_hang() {
        // Only the edges matter here, so the rows are built by one helper:
        // the walk asks about parentage and never about a command name.
        let row = |pid, ppid| Proc {
            pid,
            ppid,
            comm: "x".to_owned(),
        };
        // The pane's shell, the seat's tool, and a process the TOOL started —
        // which is what a `reseat` run from inside the seat would be.
        let procs = vec![row(100, 1), row(200, 100), row(300, 200), row(400, 1)];
        assert!(super::is_descendant_of(&procs, 100, 300), "under the pane");
        assert!(super::is_descendant_of(&procs, 200, 300), "under the tool");
        assert!(
            !super::is_descendant_of(&procs, 100, 400),
            "a sibling shell"
        );
        assert!(
            !super::is_descendant_of(&procs, 100, 100),
            "not its own ancestor"
        );
        // A pid the table does not name cannot be placed: fail closed to false
        // rather than walk off the end of a damaged snapshot.
        assert!(!super::is_descendant_of(&procs, 100, 999));
        // A hand-edited or damaged table can name a cycle. The visited set is
        // what makes this terminate at all.
        let cycle = vec![row(10, 11), row(11, 10)];
        assert!(!super::is_descendant_of(&cycle, 99, 10));
    }
}
