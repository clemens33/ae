//! The process-table snapshot and its descendant tree — the one non-tmux read
//! the watchdog's dead-check needs, kept PURE here and spawned through the
//! single sealed `ps` door in [`crate::transport`].
//!
//! # Why a process table at all
//!
//! A pane whose foreground command is a shell is not proof its agent died: a
//! `bash -lc <tool>` wrapper shows a shell in the foreground while the real
//! agent runs underneath. The frozen bash `_pane_has_descendant_named`
//! (ae:16224-16248) resolves that by asking whether a process *named* the
//! agent binary is a descendant of the pane's pid — `pgrep -x` plus a
//! `ps -o ppid=` ancestor walk, never `/proc` (macOS has none, and on Linux
//! `/proc/<pid>/stat` field 4 shifts on a comm containing a space).
//!
//! # The clean-cut shape
//!
//! Rather than spawn `ps` once per agent, the daemon takes ONE process-table
//! snapshot per cycle and builds the descendant relationships in Rust for every
//! pane (colead's ruling). [`crate::transport`] owns the spawn (the sealed
//! `run_ps` door); this module owns the FIXED argv it runs and every byte of
//! interpretation.
//!
//! # Failure is UNKNOWN, never dead
//!
//! A snapshot that could not be taken (spawn/exit failure) or could not be
//! parsed (malformed output) yields [`Descendancy::Unknown`], and the dead-check
//! treats Unknown as NOT dead. The direction is deliberate: a missed dead is
//! silent and self-heals next cycle, but a FALSE dead spams a live agent with a
//! process-died alert. So parse refuses strictly — any row that is not
//! `<pid> <ppid> <comm>` fails the whole table to `None` rather than guessing.

use std::collections::{HashMap, HashSet};

/// The fixed argv for the process-table snapshot, sealed the way
/// [`crate::git::GitArgv`] is: the inner vector is private, so no other module
/// can fabricate a `ps` command line and hand it to [`crate::transport::run_ps`].
/// Unlike git there is NO caller input at all — the argv is a constant, so this
/// door has no injection surface to seal beyond the type itself.
///
/// `ps -A -o pid=,ppid=,comm=` is the one spelling supported on BOTH targets:
/// `-A` selects every process (POSIX, GNU and BSD), and the `=` empty header
/// suppresses the column titles per field. `comm` is a full executable path on
/// macOS (basename it) and the 15-char-truncated command name on Linux; every
/// agent binary name is shorter than that truncation, so a bare basename compare
/// is exact on both.
pub struct PsArgv(Vec<String>);

impl PsArgv {
    /// The snapshot argv. There is exactly one, so this is a constructor with no
    /// parameters rather than a builder over a query.
    #[must_use]
    pub fn snapshot() -> Self {
        Self(vec![
            "-A".to_owned(),
            "-o".to_owned(),
            "pid=,ppid=,comm=".to_owned(),
        ])
    }

    /// The argv reading ONE process's controlling tty — frozen's
    /// `ps -o tty= -p $$`, which `ae next --attach` uses to tell a real pane
    /// from an inherited `$TMUX`. The only parameter is a pid, so this door has
    /// no injection surface either: a `u32` cannot be a flag.
    #[must_use]
    pub fn tty_of(pid: u32) -> Self {
        Self(vec![
            "-o".to_owned(),
            "tty=".to_owned(),
            "-p".to_owned(),
            pid.to_string(),
        ])
    }

    /// The argv for the transport door to spawn. Reading is harmless;
    /// construction is what is sealed.
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
///
/// Blank lines (a trailing newline) are skipped, not refused. Everything from
/// the third field to end of line is the command, trimmed — so a macOS full
/// path that happens to contain a space is preserved intact rather than
/// splitting the row.
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
/// whitespace run. `None` when `s` holds no token.
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
/// snapshot — the pure port of the bash ancestor walk, done once for the whole
/// table instead of per candidate.
///
/// The name compare is by basename (macOS `comm` is a path) and tolerant of a
/// trailing `.exe` on either side (`pane_current_command` reports `opencode.exe`
/// for a binary the roster calls `opencode`). A malformed table that formed a
/// ppid cycle cannot loop the walk — a visited set bounds it.
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

/// Compose a snapshot into a [`Descendancy`]: `None` (no usable snapshot) is
/// `Unknown`; a good snapshot is `Present`/`Absent` by the descendant walk. This
/// is the one call the daemon makes per pane, and the only place `Unknown` is
/// minted.
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

/// Basename compare tolerant of a trailing `.exe` on either operand.
fn name_matches(comm: &str, agent_bin: &str) -> bool {
    let base = comm.rsplit('/').next().unwrap_or(comm);
    strip_exe(base) == strip_exe(agent_bin)
}

fn strip_exe(s: &str) -> &str {
    s.strip_suffix(".exe").unwrap_or(s)
}

/// The live process table, parsed — `None` when `ps` could not be run or its
/// output did not parse (both are [`Descendancy::Unknown`] upstream, never a
/// dead agent). The one impure entry in this module; everything else decides
/// over the bytes it returns.
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
///
/// Frozen folded whitespace out of the reading and then treated `?`, `??` and
/// `-` as "no tty" — three spellings because the answer is `ps`-flavour
/// dependent and every one of them means the same thing. An empty answer is the
/// fourth.
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
        // pane(100) -> bash(200) -> claude(300): the wrapper case the whole
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
        // Malformed: 100 and 200 are each other's parent. The walk must end.
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
}
