//! The process-table snapshot and its descendant tree — the one non-tmux read
//! the watchdog's dead-check needs, kept PURE here and spawned through the
//! single sealed `ps` door in [`crate::transport`].

use std::collections::{HashMap, HashSet};

use crate::tool::ToolKind;

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

    /// The snapshot argv with each process's state flags ([`background_task`]).
    #[must_use]
    pub fn with_stat() -> Self {
        Self(vec![
            "-A".to_owned(),
            "-o".to_owned(),
            "pid=,ppid=,stat=,comm=".to_owned(),
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

/// The shells the dead-check treats as "no agent in the foreground".
#[must_use]
pub fn command_is_shell(cmd: &str) -> bool {
    matches!(cmd, "bash" | "zsh" | "fish" | "sh" | "dash" | "")
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

/// Where `pid` stands relative to `ancestor` — the walk [`has_descendant_named`]
/// makes downwards, made UPWARDS from one known pid.
///
/// A seat cannot reseat itself, and the pane id is only half of that rule: a
/// caller the target's own tool started may carry no `$TMUX_PANE` to compare,
/// and the process tree is the other half. Walks up with a visited set,
/// because a damaged table can name a cycle and this may not hang.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lineage {
    /// `ancestor` is on its way up.
    Under,
    /// The walk reached pid 1 or 0 without meeting `ancestor`.
    Clear,
    /// The walk stopped here: the table has no row for this pid, or a cycle.
    Broken(u32),
}

/// THE upward walk. A gap is [`Lineage::Broken`], never clear: ae cannot place
/// a process whose parent it never saw.
#[must_use]
pub(crate) fn lineage(procs: &[Proc], ancestor: u32, pid: u32) -> Lineage {
    let parents: HashMap<u32, u32> = procs.iter().map(|proc| (proc.pid, proc.ppid)).collect();
    let mut visited: HashSet<u32> = HashSet::new();
    let mut current = pid;
    while visited.insert(current) {
        let Some(&parent) = parents.get(&current) else {
            return Lineage::Broken(current);
        };
        if parent == ancestor {
            return Lineage::Under;
        }
        if parent <= 1 {
            return Lineage::Clear;
        }
        current = parent;
    }
    Lineage::Broken(current)
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

/// Which harness a pane is OBSERVED to run — for DIAGNOSIS only.
///
/// A seat's identity is its records, and nothing here may rewrite them or
/// authorise a stop. What this answers is the question a refusal and `ae
/// doctor` owe a human when the records and the pane disagree: WHAT runs
/// there instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// A shell holds the foreground and the tree shows no known harness.
    Shell,
    /// Every known harness the foreground word or the pane's process tree
    /// names, deduplicated and sorted by name.
    Harness(Vec<ToolKind>),
    /// A foreground that is no shell and names no harness, over a tree that
    /// shows none — a human's editor, or a tool ae does not know.
    Unrecognised(String),
    /// The pid or the process table is missing, so the tree was never read.
    Unknown,
}

/// THE ONE foreground-to-tool mapping, from the readings a caller already took.
///
/// Measured 2026-09-23 on macOS, Claude Code's native install: `claude` is a
/// symlink to `versions/<version>`, so tmux's `pane_current_command` reads the
/// VERSION (`2.1.280`) while `ps -o comm=` reads the invoked path
/// (`…/bin/claude`). The tree carries the real name, so no version grammar is
/// guessed at: a bare `2.1.274` with no table to corroborate it is
/// [`Observed::Unrecognised`].
#[must_use]
pub(crate) fn observed_harness(
    foreground: &str,
    pane_pid: Option<u32>,
    table: Option<&[Proc]>,
) -> Observed {
    let mut tools: Vec<ToolKind> = harness_of(foreground).into_iter().collect();
    if let (Some(pid), Some(rows)) = (pane_pid, table) {
        let rows = harness_rows(rows, pid);
        tools.extend(
            rows.iter()
                .filter(|row| row.2 == Lineage::Under)
                .map(|row| row.0),
        );
    }
    tools.sort_by_key(|tool| tool.as_str());
    tools.dedup();
    if !tools.is_empty() {
        return Observed::Harness(tools);
    }
    if pane_pid.is_none() || table.is_none() {
        return Observed::Unknown;
    }
    if command_is_shell(foreground) {
        Observed::Shell
    } else {
        Observed::Unrecognised(foreground.to_owned())
    }
}

/// What a pane may be CALLED to run: [`observed_harness`] over a COVERED tree
/// only — the pane's root is in the table and no harness is left that ae cannot
/// place — else [`Observed::Unknown`]. The same readings a reseat's stop
/// refuses on, asked by `ae doctor` and `ae list` alike, so neither can claim a
/// mismatch the other would call unknown.
#[must_use]
pub(crate) fn covered_reading(
    foreground: &str,
    pane_pid: Option<u32>,
    table: Option<&[Proc]>,
) -> Observed {
    match (pane_pid, table) {
        (Some(pid), Some(rows))
            if rows.iter().any(|row| row.pid == pid)
                && harness_rows(rows, pid)
                    .iter()
                    .all(|row| row.2 == Lineage::Under) =>
        {
            observed_harness(foreground, Some(pid), Some(rows))
        }
        _ => Observed::Unknown,
    }
}

/// Whether the harnesses a pane runs disagree with the one tool its seat
/// records: anything but exactly that tool — the recorded one beside another is
/// never clean. The ONE predicate behind `ae doctor`'s `seat-identity` and
/// `ae list`'s `harness:` cell.
#[must_use]
pub(crate) fn harness_disagrees(tools: &[ToolKind], recorded: &str) -> bool {
    !matches!(tools, [only] if name_matches(recorded, only.as_str()))
}

/// The known harness one process name belongs to — basename, `.exe` tolerant.
fn harness_of(comm: &str) -> Option<ToolKind> {
    let base = comm.rsplit('/').next().unwrap_or(comm);
    ToolKind::from_known_binary_name(strip_exe(base))
}

/// Every row naming a known harness that is the pane's root, runs beneath it
/// ([`Lineage::Under`]), or cannot be placed at all ([`Lineage::Broken`]) — in
/// table order. A harness that provably runs elsewhere is left out.
pub(crate) fn harness_rows(procs: &[Proc], pane_pid: u32) -> Vec<(ToolKind, &Proc, Lineage)> {
    procs
        .iter()
        .filter_map(|proc| {
            let tool = harness_of(&proc.comm)?;
            let place = if proc.pid == pane_pid {
                Lineage::Under
            } else {
                lineage(procs, pane_pid, proc.pid)
            };
            (place != Lineage::Clear).then_some((tool, proc, place))
        })
        .collect()
}

/// The pane's one recorded tool; unreadable rows that might hide another hold.
pub(crate) fn tool_in(table: &[(Proc, String)], pane_pid: u32, bin: &str) -> Option<u32> {
    let procs: Vec<Proc> = table.iter().map(|(proc, _)| proc.clone()).collect();
    if table.iter().any(|(proc, stat)| {
        !readable_stat(proc, stat)
            && (proc.pid == pane_pid || lineage(&procs, pane_pid, proc.pid) != Lineage::Clear)
    }) {
        return None;
    }
    let mut found = harness_rows(&procs, pane_pid)
        .into_iter()
        .filter(|(_, proc, place)| *place == Lineage::Under && name_matches(&proc.comm, bin))
        .map(|(_, proc, _)| proc.pid);
    let pid = found.next()?;
    found.next().is_none().then_some(pid)
}

/// Basename compare tolerant of a trailing `.exe` on either operand — the ONE
/// name equality, shared by the descendant walk and by every caller that
/// compares a foreground command with a recorded binary or its live family.
#[must_use]
pub(crate) fn name_matches(comm: &str, agent_bin: &str) -> bool {
    let base = comm.rsplit('/').next().unwrap_or(comm);
    let (live, recorded) = (strip_exe(base), strip_exe(agent_bin));
    live == recorded
        || ToolKind::from_known_binary_name(recorded)
            .is_some_and(|kind| kind.adapter().process_name_matches(live))
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

/// `ps -A -o pid=,ppid=,stat=,comm=` rows, retaining unreadable names and states
/// for the caller's lineage proof. Missing pid/ppid/stat or invalid pids reject
/// the whole table: those rows cannot be placed safely.
#[must_use]
pub(crate) fn parse_stat_table(raw: &str) -> Option<Vec<(Proc, String)>> {
    let mut out = Vec::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let (own, rest) = split_first_token(line)?;
        let (parent, rest) = split_first_token(rest)?;
        let (stat, comm) = split_first_token(rest)?;
        let comm = comm.trim();
        let row = Proc {
            pid: own.parse().ok()?,
            ppid: parent.parse().ok()?,
            comm: comm.to_owned(),
        };
        out.push((row, stat.to_owned()));
    }
    Some(out)
}

/// One state letter then BSD/procps flags, with a known command name.
fn readable_stat(proc: &Proc, stat: &str) -> bool {
    let mut flags = stat.chars();
    !proc.comm.is_empty()
        && flags
            .next()
            .is_some_and(|state| "DIPRSTUWXZt".contains(state))
        && flags.all(|flag| "+<>AEHLNSVWXls".contains(flag))
}

/// Whether the tool process `tool` runs a BACKGROUND task: a shell beneath it
/// leading its own session (`s`), as a command claude and codex run does and
/// an idle tool's MCP wrapper shell does not (measured). `None` is unknown.
#[must_use]
pub(crate) fn background_task(table: Option<&[(Proc, String)]>, tool: u32) -> Option<bool> {
    let table = table?;
    let procs: Vec<Proc> = table.iter().map(|(proc, _)| proc.clone()).collect();
    procs.iter().find(|proc| proc.pid == tool)?;
    if table.iter().any(|(proc, stat)| {
        !readable_stat(proc, stat)
            && (proc.pid == tool || lineage(&procs, tool, proc.pid) != Lineage::Clear)
    }) {
        return None;
    }
    Some(table.iter().any(|(proc, stat)| {
        let base = proc.comm.rsplit('/').next().unwrap_or(&proc.comm);
        let shell = command_is_shell(base.trim_start_matches('-'));
        shell
            && stat.contains('s')
            && proc.pid != tool
            && lineage(&procs, tool, proc.pid) != Lineage::Clear
    }))
}

/// The seat's tool under `pane_pid`: every row named `bin` there, which must
/// form ONE ancestor chain (a launcher and its inner process of the same
/// kind), outermost first. No row, a row ae cannot place, or two rows on
/// parallel branches is `None`: ae cannot say which process is the seat.
fn tool_chain(procs: &[Proc], pane_pid: u32, bin: &str) -> Option<Vec<u32>> {
    let mut chain: Vec<u32> = Vec::new();
    for proc in procs.iter().filter(|proc| name_matches(&proc.comm, bin)) {
        if proc.pid != pane_pid {
            match lineage(procs, pane_pid, proc.pid) {
                Lineage::Under => {}
                Lineage::Clear => continue,
                Lineage::Broken(_) => return None,
            }
        }
        chain.push(proc.pid);
    }
    let under = |a: u32, b: u32| lineage(procs, a, b) == Lineage::Under;
    // A total order by ancestry, or no single seat.
    for (index, a) in chain.iter().enumerate() {
        if chain[index + 1..]
            .iter()
            .any(|b| !under(*a, *b) && !under(*b, *a))
        {
            return None;
        }
    }
    let members = chain.clone();
    chain.sort_by_key(|pid| chain_depth(procs, &members, *pid));
    (!chain.is_empty()).then_some(chain)
}

/// How many other chain members sit above `pid`.
fn chain_depth(procs: &[Proc], chain: &[u32], pid: u32) -> usize {
    chain
        .iter()
        .filter(|ancestor| lineage(procs, **ancestor, pid) == Lineage::Under)
        .count()
}

/// Whether the seat's tool runs a live TASK (#283): a session-leading (`s`),
/// live (not `Z`) DIRECT child of the tool chain that is either a non-shell
/// command — a shell that exec-ed into it (codex 0.167: `codex -> sleep Ss`)
/// — or a shell with a live non-shell descendant at any depth (claude
/// 2.1.x: `claude -> zsh Ss -> cargo`). An idle tool's MCP children are not
/// session leaders, and its session-leading MCP processes are grandchildren
/// (measured, `.local/chal225/ptree-*.txt` of the slice), so neither counts.
/// `None` is unknown: no table, no single tool chain, a tool row ae cannot
/// read, or no task proven while a row at or beneath the tool cannot be read
/// (it may be the task or its witness).
///
/// A task is PROVEN by readable rows alone (#312): the task root and its
/// witness (the root itself when it is a non-shell command, else a live
/// non-shell descendant) must each read, and the pid/ppid walk between them
/// needs no state. A transient `?N (ae)` row — a process between fork/exec or
/// exit, present almost all the time under a running test suite on macOS —
/// can therefore neither void a proven task nor be told apart from a hidden
/// one when none is proven.
#[must_use]
pub fn task_running(table: Option<&[(Proc, String)]>, pane_pid: u32, bin: &str) -> Option<bool> {
    let table = table?;
    let procs: Vec<Proc> = table.iter().map(|(proc, _)| proc.clone()).collect();
    let chain = tool_chain(&procs, pane_pid, bin)?;
    let root = *chain.first()?;
    // The tool's own rows name the seat; one that cannot be read is unknown
    // whatever else the table shows.
    if table
        .iter()
        .any(|(proc, stat)| chain.contains(&proc.pid) && !readable_stat(proc, stat))
    {
        return None;
    }
    let live = |stat: &str| !stat.starts_with('Z');
    let shell = |proc: &Proc| {
        let base = proc.comm.rsplit('/').next().unwrap_or(&proc.comm);
        command_is_shell(base.trim_start_matches('-'))
    };
    let proven = table.iter().any(|(task, stat)| {
        readable_stat(task, stat)
            && chain.contains(&task.ppid)
            && !chain.contains(&task.pid)
            && stat.contains('s')
            && live(stat)
            && (!shell(task)
                || table.iter().any(|(below, below_stat)| {
                    readable_stat(below, below_stat)
                        && live(below_stat)
                        && !shell(below)
                        && lineage(&procs, task.pid, below.pid) == Lineage::Under
                }))
    });
    if proven {
        return Some(true);
    }
    let hidden = table.iter().any(|(proc, stat)| {
        !readable_stat(proc, stat)
            && (proc.pid == root || lineage(&procs, root, proc.pid) != Lineage::Clear)
    });
    (!hidden).then_some(false)
}

/// The plain `pid ppid comm` table a stat reading stands in for, with
/// [`parse_table`]'s validity: a row with no command name makes it `None`
/// (unknown), so the liveness readers see exactly what the plain snapshot
/// would have given them. The stat flags are not part of that contract.
#[must_use]
pub fn plain_table(stat: Option<&[(Proc, String)]>) -> Option<Vec<Proc>> {
    let rows = stat?;
    if rows.iter().any(|(proc, _)| proc.comm.is_empty()) {
        return None;
    }
    Some(rows.iter().map(|(proc, _)| proc.clone()).collect())
}

/// The live state table; failed reads or structural damage yield `None`.
#[must_use]
pub(crate) fn stat_snapshot() -> Option<Vec<(Proc, String)>> {
    let (succeeded, stdout) = crate::transport::run_ps(&PsArgv::with_stat());
    succeeded.then(|| parse_stat_table(&stdout)).flatten()
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
#[path = "../tests/it/statrow_verdict_spec.rs"]
mod statrow_verdict_spec;

#[cfg(test)]
#[path = "../tests/it/procs_task225_spec.rs"]
mod procs_task225_spec;
#[cfg(test)]
#[path = "../tests/it/procs_transient312_spec.rs"]
mod procs_transient312_spec;
#[cfg(test)]
#[path = "../tests/it/statrow_tool_spec.rs"]
mod statrow_tool_spec;

#[cfg(test)]
mod tests {
    use super::{
        Descendancy, Proc, PsArgv, command_is_shell, descendancy, has_descendant_named, parse_table,
    };

    #[test]
    fn the_shell_set_is_exactly_the_bash_command_is_shell_case() {
        for shell in ["bash", "zsh", "fish", "sh", "dash", ""] {
            assert!(command_is_shell(shell), "{shell:?} is a shell");
        }
        for other in [
            "claude",
            "codex",
            "opencode.exe",
            "python",
            "node",
            "bashx",
            "ssh",
        ] {
            assert!(!command_is_shell(other), "{other:?} is not a shell");
        }
    }

    /// Shapes measured in `.local/effort/idle-trees.md`: an idle claude keeps
    /// an MCP wrapper `/bin/sh` (`S+`) running docker; a command either tool
    /// runs is a `/bin/zsh` session leader (`Ss`).
    /// R1 (chal225 review): the daemon's ONE `ps` became the stat reading;
    /// its plain projection must keep the plain parser's verdict, or an
    /// unnamed tool row turns an unknown snapshot into a dead seat.
    #[test]
    fn the_plain_projection_of_a_stat_reading_keeps_the_plain_parsers_verdict() {
        // The same process table read by both doors: `pid ppid comm` and
        // `pid ppid stat comm`. An unnamed row makes the plain door unknown.
        let plain = "1 0 launchd\n90 1 -zsh\n100 90 \n";
        let stat = "1 0 Ss launchd\n90 1 Ss -zsh\n100 90 S+\n";
        assert_eq!(super::parse_table(plain), None, "the plain door: unknown");
        let reading = super::parse_stat_table(stat);
        assert!(reading.is_some(), "the stat parser keeps the row");
        assert_eq!(super::plain_table(reading.as_deref()), None);
        let plain = "1 0 launchd\n90 1 -zsh\n100 90 claude\n";
        let stat = "1 0 Ss launchd\n90 1 Ss -zsh\n100 90 S+ claude\n";
        assert!(super::parse_table(plain).is_some());
        assert_eq!(
            super::plain_table(super::parse_stat_table(stat).as_deref()),
            super::parse_table(plain)
        );
        assert_eq!(super::plain_table(None), None);
    }

    /// Another seat's tool, in another pane, is not part of this seat's
    /// chain: every machine running two codex seats has one.
    #[test]
    fn a_tool_in_another_pane_neither_joins_nor_spoils_the_seats_chain() {
        let table = "1 0 Ss launchd\n90 1 Ss -zsh\n100 90 S+ codex\n110 100 Ss cargo\n\
                     190 1 Ss -zsh\n200 190 S+ codex\n";
        let rows = super::parse_stat_table(table);
        assert_eq!(
            super::task_running(rows.as_deref(), 90, "codex"),
            Some(true)
        );
        assert_eq!(
            super::task_running(rows.as_deref(), 190, "codex"),
            Some(false)
        );
        // A tool exec-ed as the pane's own process is the chain's root.
        let execed = "1 0 Ss launchd\n90 1 Ss codex\n110 90 Ss cargo\n";
        let rows = super::parse_stat_table(execed);
        assert_eq!(
            super::task_running(rows.as_deref(), 90, "codex"),
            Some(true)
        );
    }

    /// The chain's root is its OUTERMOST member whatever order `ps` lists
    /// them in, so an unreadable row under the launcher alone stays unknown.
    #[test]
    fn the_chain_root_is_the_outermost_tool_even_when_listed_inner_first() {
        let table = "1 0 Ss launchd\n90 1 Ss -zsh\n95 100 S /opt/codex/bin/codex\n\
                     100 90 S+ codex\n110 100 ? cargo\n";
        let rows = super::parse_stat_table(table);
        assert_eq!(super::task_running(rows.as_deref(), 90, "codex"), None);
        let readable = table.replace("110 100 ? cargo", "110 100 Ss cargo");
        let rows = super::parse_stat_table(&readable);
        assert_eq!(
            super::task_running(rows.as_deref(), 90, "codex"),
            Some(true)
        );
    }

    #[test]
    fn only_a_session_leading_shell_beneath_the_tool_is_a_background_task() {
        let idle = "  1     0 Ss   /sbin/launchd\n 90     1 Ss   -zsh\n100    90 S+   claude\n\
                    101   100 S+   /bin/sh\n102   101 S+   docker\n";
        let table = super::parse_stat_table(idle).expect("measured rows parse");
        assert_eq!(super::background_task(Some(&table), 100), Some(false));
        assert_eq!(super::background_task(Some(&table), 7), None, "no tool row");
        assert_eq!(super::background_task(None, 100), None, "no table");
        let busy = format!("{idle}103   100 Ss   /bin/zsh\n104   103 S    sleep\n");
        let table = super::parse_stat_table(&busy).expect("measured rows parse");
        assert_eq!(super::background_task(Some(&table), 100), Some(true));
        let deep = format!("{idle}105   102 Ss   -bash\n");
        let table = super::parse_stat_table(&deep).expect("login shell spelling");
        assert_eq!(super::background_task(Some(&table), 100), Some(true));
        let verdict =
            |raw: &str| super::background_task(super::parse_stat_table(raw).as_deref(), 100);
        assert_eq!(
            verdict("100 90 S+\n"),
            None,
            "a tool row needs a name: an unnamed tool is unknown, never a clear tree"
        );
        assert_eq!(
            super::parse_stat_table("x 90 S+ claude\n"),
            None,
            "a row ae cannot place is no table"
        );
        assert_eq!(
            verdict("100 90 ? claude\n101 100 ? /bin/sh\n"),
            None,
            "an unknown stat on the tool or under it is unknown, never a clear tree"
        );
        assert_eq!(
            verdict("100 90 Sq claude\n"),
            None,
            "an unknown flag on the tool is unknown, never a clear tree"
        );
    }

    fn trow(proc: u32, parent: u32, comm: &str) -> Proc {
        Proc {
            pid: proc,
            ppid: parent,
            comm: comm.to_owned(),
        }
    }

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
        use super::Lineage::{Broken, Clear, Under};
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
        assert_eq!(super::lineage(&procs, 100, 300), Under, "under the pane");
        assert_eq!(super::lineage(&procs, 200, 300), Under, "under the tool");
        assert_eq!(super::lineage(&procs, 100, 400), Clear, "a sibling");
        assert_eq!(super::lineage(&procs, 100, 100), Clear, "not its own");
        // A pid the table does not name cannot be placed: fail closed to a
        // BREAK rather than walk off the end of a damaged snapshot.
        assert_eq!(super::lineage(&procs, 100, 999), Broken(999));
        assert_eq!(super::lineage(&[row(50, 60)], 100, 50), Broken(60));
        // A hand-edited or damaged table can name a cycle. The visited set is
        // what makes this terminate at all.
        let cycle = vec![row(10, 11), row(11, 10)];
        assert_eq!(super::lineage(&cycle, 99, 10), Broken(10));
    }

    #[test]
    fn the_pane_names_the_harness_it_runs_by_its_tree_never_by_a_version_word() {
        use super::{Observed, observed_harness};
        use crate::tool::ToolKind;
        let row = |pid, ppid, comm: &str| Proc {
            pid,
            ppid,
            comm: comm.to_owned(),
        };
        // THE INCIDENT, as measured: tmux reads the version, `ps` the invoked
        // path, and the tool runs under the pane's shell.
        let incident = vec![
            row(100, 1, "/opt/homebrew/bin/fish"),
            row(200, 100, "/Users/x/.local/bin/claude"),
            row(300, 1, "/usr/local/bin/codex"),
        ];
        assert_eq!(
            observed_harness("2.1.274", Some(100), Some(&incident)),
            Observed::Harness(vec![ToolKind::Claude]),
            "the tree names claude; a harness under ANOTHER pane is not counted"
        );
        // No table to corroborate it: a version word is not a guess at claude.
        assert_eq!(
            observed_harness("2.1.274", Some(100), None),
            Observed::Unknown
        );
        assert_eq!(
            observed_harness("2.1.274", Some(300), Some(&[row(300, 1, "2.1.274")])),
            Observed::Unrecognised("2.1.274".to_owned())
        );
        // A foreground that IS a harness name needs no tree at all.
        assert_eq!(
            observed_harness("codex", None, None),
            Observed::Harness(vec![ToolKind::Codex])
        );
        // A tool exec'd AS the pane's own process is the pane's tree too.
        assert_eq!(
            observed_harness("x", Some(300), Some(&[row(300, 1, "/opt/grok")])),
            Observed::Harness(vec![ToolKind::Grok])
        );
        // Mixed evidence is reported whole, deduplicated, sorted by name.
        let mixed = vec![
            row(100, 1, "fish"),
            row(200, 100, "opencode.exe"),
            row(210, 200, "claude"),
            row(220, 200, "claude"),
        ];
        assert_eq!(
            observed_harness("opencode", Some(100), Some(&mixed)),
            Observed::Harness(vec![ToolKind::Claude, ToolKind::OpenCode])
        );
        // No harness anywhere: a shell is a shell, anything else is named.
        let editor = vec![row(100, 1, "fish"), row(200, 100, "vim")];
        assert_eq!(
            observed_harness("fish", Some(100), Some(&[row(100, 1, "fish")])),
            Observed::Shell
        );
        assert_eq!(
            observed_harness("vim", Some(100), Some(&editor)),
            Observed::Unrecognised("vim".to_owned())
        );
        // A tree ae could not read cannot rule a harness out, shell or not.
        assert_eq!(
            observed_harness("fish", None, Some(&editor)),
            Observed::Unknown
        );
        assert_eq!(observed_harness("vim", Some(100), None), Observed::Unknown);
        // A cycle in a damaged table terminates.
        let cycle = vec![row(10, 11, "sh"), row(11, 10, "sh")];
        assert_eq!(
            observed_harness("sh", Some(10), Some(&cycle)),
            Observed::Shell
        );
        // A harness ae cannot place is never counted as the pane's: diagnosis
        // claims only what it saw.
        let unplaced = [row(10, 1, "sh"), row(900, 850, "claude")];
        assert_eq!(
            observed_harness("sh", Some(10), Some(&unplaced)),
            Observed::Shell
        );
    }

    #[test]
    fn a_live_muse_binary_under_a_pane_recorded_muse_reads_present() {
        use super::{Observed, observed_harness};
        use crate::tool::ToolKind;
        // MEASURED 2026-09-28 (#206): the seat records `muse` while the live
        // process is the versioned `muse-bin-1.4.0-R4302.1`; `ps -o comm=`
        // reads the full path on macOS.
        let procs = vec![
            trow(100, 1, "fish"),
            trow(200, 100, "/Users/ckriech/.local/bin/muse-bin-1.4.0-R4302.1"),
        ];
        assert_eq!(descendancy(Some(&procs), 100, "muse"), Descendancy::Present);
        assert_eq!(
            observed_harness("fish", Some(100), Some(&procs)),
            Observed::Harness(vec![ToolKind::Muse])
        );
    }

    #[test]
    fn muse_near_names_never_read_present() {
        let procs = |comm: &str| vec![trow(100, 1, "fish"), trow(200, 100, comm)];
        for near in ["musecode", "muse-binx", "muse-bin-", "muse-binary"] {
            assert_eq!(
                descendancy(Some(&procs(near)), 100, "muse"),
                Descendancy::Absent,
                "{near:?} is not the muse family"
            );
        }
        // A Linux-truncated comm keeps the digit after the prefix, so it
        // still matches.
        assert_eq!(
            descendancy(Some(&procs("muse-bin-1.4.0-")), 100, "muse"),
            Descendancy::Present
        );
    }
}
