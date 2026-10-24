//! Ordinary End's writer holds, retry decision and post-publication signal.
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{
    Action, Git, Notice, Plan, archive, archive_keep, archive_step, cleanup, meta, meta_value,
    recorded_server, sessions_dir,
};

/// Advisory raw inspection: never wait on a source writer before taking holds.
pub(super) fn inspect(dir: &Path, live: &[u8]) -> Result<Option<Git>, String> {
    let id = archive::canonical_uuid(&meta_value(live, "session_id"));
    let target = target(dir, &id);
    if id.is_empty() || !archive::retained::node_exists(&target) {
        return Ok(None);
    }
    let saved = archive::publish::completed_end(dir, &id).ok_or_else(|| {
        archive::publish::mismatch(dir, &target, "no complete strict End archive")
    })?;
    Ok(Some(Git {
        outcome: meta_value(&saved, "git_push_outcome"),
        push_ref: meta_value(&saved, "git_push_ref"),
        preserved: meta_value(&saved, "preserved_work_dir"),
        workdir: meta_value(&saved, "source_work_dir"),
        retry_id: Some(id),
    }))
}

fn target(dir: &Path, id: &str) -> PathBuf {
    dir.parent()
        .and_then(Path::parent)
        .unwrap_or(dir)
        .join("archive")
        .join(id)
}

struct Holds {
    _activities: Vec<File>,
    sources: archive::publish::SourceGuards,
}

impl Holds {
    fn take(dir: &Path) -> Result<Self, String> {
        let mut activities = Vec::new();
        for name in [
            ".console-writer.lock",
            ".console-admission.lock",
            ".goal-setter.lock",
            ".console-draft-mutation.lock",
            "telegram-outbound.cursor.lock",
        ] {
            let path = dir.join(name);
            activities.push(
                crate::store::lock(&path, Duration::ZERO)
                    .map_err(|why| format!("End could not lock {}: {why}", path.display()))?,
            );
        }
        Ok(Self {
            _activities: activities,
            sources: archive::publish::hold_sources(dir)?,
        })
    }
}

/// Constructed only under all End holds after proving the exact managed path
/// positively absent and a matching strict archive with a durable push outcome.
pub(crate) struct RemovedWorkdir {
    path: PathBuf,
    git: archive::GitFacts,
}

impl RemovedWorkdir {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    fn prove(root: &Path, name: &str, live: &[u8], saved: &[u8]) -> Result<Self, String> {
        let path = root.join("worktrees").join(name);
        let absent = !archive::retained::node_exists(&path);
        let root_real = archive::retained::plain_dir(&root.join("worktrees"));
        let base = meta_value(live, "git_base_commit");
        if !absent
            || !root_real
            || meta_value(live, "work_dir") != path.to_string_lossy()
            || meta_value(saved, "source_work_dir") != path.to_string_lossy()
            || meta_value(saved, "source_session") != name
            || meta_value(saved, "source_mode") != meta_value(live, "mode")
            || !matches!(meta_value(live, "mode").as_str(), "full" | "git")
            || !matches!(
                meta_value(saved, "git_push_outcome").as_str(),
                "pushed" | "already-reachable"
            )
            || meta_value(saved, "preserved_work_dir") != "-"
            || meta_value(saved, "git_base_commit") != if base.is_empty() { "-" } else { &base }
        {
            return Err(format!(
                "missing working directory {} has no completed-removal proof",
                path.display()
            ));
        }
        let facts = archive::GitFacts {
            base: meta_value(saved, "git_base_commit"),
            final_commit: meta_value(saved, "git_final_commit"),
            range: meta_value(saved, "git_commit_range"),
            count: meta_value(saved, "git_commit_count"),
        };
        if facts.final_commit.is_empty() || facts.final_commit == "-" {
            return Err(format!(
                "missing working directory {} has unknown git outcome",
                path.display()
            ));
        }
        Ok(Self { path, git: facts })
    }
}

fn removed_workdir(
    root: &Path,
    name: &str,
    live: &[u8],
    git: &Git,
    dir: &Path,
    id: &str,
) -> Result<Option<RemovedWorkdir>, String> {
    if matches!(meta_value(live, "mode").as_str(), "local" | "")
        || archive::retained::node_exists(Path::new(&git.workdir))
    {
        return Ok(None);
    }
    let saved = archive::publish::completed_end(dir, id)
        .ok_or_else(|| format!("no complete archive for missing workdir {}", git.workdir))?;
    RemovedWorkdir::prove(root, name, live, &saved).map(Some)
}

/// Git network work is finished before entry. Every handle survives the final
/// source proof, managed removal and durable canonical unregister.
pub(super) fn finish(
    root: &Path,
    name: &str,
    before: &[u8],
    plan: &Plan,
    git: &Git,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    if plan.action != Action::Keep {
        return Ok(archive_step(root, name, plan, git, out, err)?
            && cleanup(root, name, before, plan, git.preserved != "-", out, err)?);
    }
    let dir = sessions_dir(root).join(name);
    let id = archive::canonical_uuid(&meta_value(before, "session_id"));
    let result = (|| {
        let mut held = match Holds::take(&dir) {
            Ok(held) => held,
            Err(why) => {
                writeln!(err, "Error: {why}")?;
                return Ok(false);
            }
        };
        let live = meta::read_bytes(&dir)?;
        let target = target(&dir, &id);
        for key in [
            "session",
            "session_id",
            "mode",
            "work_dir",
            "origin",
            "git_base_commit",
        ] {
            if meta_value(before, key) != meta_value(&live, key) {
                writeln!(
                    err,
                    "Error: {}",
                    archive::publish::mismatch(
                        &dir,
                        &target,
                        "operation identity changed before the holds"
                    )
                )?;
                return Ok(false);
            }
        }
        if let Some(expected) = &git.retry_id
            && (expected != &id || archive::publish::completed_end(&dir, expected).is_none())
        {
            writeln!(
                err,
                "Error: {}",
                archive::publish::mismatch(
                    &dir,
                    &target,
                    "early reuse changed after git was skipped"
                )
            )?;
            return Ok(false);
        }
        let removed = match removed_workdir(root, name, &live, git, &dir, &id) {
            Ok(proof) => proof,
            Err(why) => {
                writeln!(
                    err,
                    "Error: {}",
                    archive::publish::mismatch(&dir, &target, &why)
                )?;
                return Ok(false);
            }
        };
        if let Some(proof) = &removed {
            held.sources.removed_git = Some(archive::GitFacts {
                base: proof.git.base.clone(),
                final_commit: proof.git.final_commit.clone(),
                range: proof.git.range.clone(),
                count: proof.git.count.clone(),
            });
        }
        if !archive_keep(&dir, name, git, &held.sources, out, err)? {
            return Ok(false);
        }
        // The first durable publish released and fsynced its completed claim.
        // Rebuild the complete candidate from current sources under the SAME
        // holds: this final proof cannot rewrite the immutable archive.
        if !archive_keep(&dir, name, git, &held.sources, &mut io::sink(), err)? {
            return Ok(false);
        }
        if let Some(proof) = removed {
            return Ok(crate::teardown::end_removed(&dir, &proof, err)? == 0);
        }
        cleanup(root, name, &live, plan, git.preserved != "-", out, err)
    })();
    if !matches!(result, Ok(true)) {
        let notice = Notice {
            name: name.to_owned(),
            dir,
            server: recorded_server(root, name),
            archive_id: id,
            purge: false,
        };
        self::notice(
            root,
            &notice,
            "End did not complete; retry ae end, or follow the recovery diagnostic",
        );
    }
    result
}

/// This closure holds EVENTS ONLY. It never takes meta/memo locks or compares
/// live sources: the captured UUID and complete strict archive are sufficient.
pub(super) fn emit(dir: &Path, name: &str, id: &str, action: &str, summary: &str) {
    let line = crate::tracked::event_line(&crate::tracked::EventFields::new(
        crate::time::Timestamp::now(),
        "human",
        action,
        name,
        "",
        "",
        "",
        "",
        "",
        summary,
        "",
    ));
    let _ = crate::store::open(dir).append_event_decided(|_| {
        if archive::publish::completed_end(dir, id).is_some() {
            Err(())
        } else {
            Ok(line)
        }
    });
}

pub(super) fn notice(root: &Path, notice: &Notice, why: &str) {
    if !notice.purge
        && archive::publish::completed_end(&notice.dir, &notice.archive_id).is_some()
        && let Some(server) = &notice.server
    {
        super::announce_to_clients(
            server,
            &format!(
                "End {}: {why}; archive {}; stopped live directory {} remains in ae list --all until unregister; retry or move it outside {} and restart under a fresh name",
                notice.name,
                root.join("archive").join(&notice.archive_id).display(),
                notice.dir.display(),
                sessions_dir(root).display()
            ),
        );
    }
}
