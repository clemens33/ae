//! Frozen locked-entry pins at 7004fa85. Oracle: the under-lock symlink
//! recheck contract; exact diagnostics and precedence are base behaviour pins.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "isolated characterization fixtures inspect state and output"
)]

use super::*;

struct Root(PathBuf);
impl Root {
    fn new(tag: &str) -> Self {
        Self(super::tests::scratch(&format!("rl-{tag}")))
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn locked_rechecks_old_then_new_symlinks_before_resolving_the_server() {
    for mask in 0..4 {
        let root = Root::new(&format!("links-{mask}"));
        let sessions = root.0.join("sessions");
        std::fs::create_dir_all(&sessions).expect("sessions");
        for (index, name) in ["sold", "snew"].into_iter().enumerate() {
            let target = root.0.join(format!("target-{name}"));
            std::fs::create_dir(&target).expect("link target");
            std::fs::write(target.join("sentinel"), name).expect("target sentinel");
            let dir = sessions.join(name);
            if mask & (1 << index) != 0 {
                std::os::unix::fs::symlink(&target, &dir).expect("session link");
            } else {
                std::fs::create_dir(&dir).expect("session directory");
                std::fs::write(
                    dir.join("meta"),
                    b"session=sold\ntmux_server_kind=invalid\ntmux_server=x\n",
                )
                .expect("ambiguous server");
            }
        }
        let want = if mask == 0 {
            "Error: session 'sold' records a tmux server ae cannot resolve — its meta rows 'tmux_server_kind' / 'tmux_server' do not name exactly one server. Nothing was renamed.\n".to_owned()
        } else {
            let first = if mask & 1 != 0 { "sold" } else { "snew" };
            format!(
                "Error: the session path for '{first}' is a symlink; refusing to rename through it.\n"
            )
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = locked(
            &root.0,
            "sold",
            "snew",
            Some("after-intent".into()),
            &mut out,
            &mut err,
        )
        .expect("buffer writes");
        assert_eq!(result, 1, "mask {mask}");
        assert!(out.is_empty(), "refusal cannot claim success");
        assert_eq!(String::from_utf8(err).expect("UTF-8 diagnostic"), want);
        for (index, name) in ["sold", "snew"].into_iter().enumerate() {
            let dir = sessions.join(name);
            assert_eq!(
                std::fs::symlink_metadata(&dir)
                    .expect("unchanged entry")
                    .file_type()
                    .is_symlink(),
                mask & (1 << index) != 0,
                "entry kind preserved"
            );
            assert_eq!(
                std::fs::read(root.0.join(format!("target-{name}/sentinel")))
                    .expect("target preserved"),
                name.as_bytes(),
                "link target untouched"
            );
            if mask & (1 << index) == 0 {
                assert_eq!(
                    std::fs::read(dir.join("meta")).expect("meta"),
                    b"session=sold\ntmux_server_kind=invalid\ntmux_server=x\n"
                );
            }
        }
        assert_eq!(std::fs::read_dir(sessions).expect("sessions").count(), 2);
    }
}

struct Broken;
impl std::io::Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn locked_refusal_propagates_the_original_output_error() {
    let root = Root::new("broken-output");
    let dir = root.0.join("sessions/sold");
    std::fs::create_dir_all(&dir).expect("session");
    let bytes = b"tmux_server_kind=invalid\ntmux_server=x\n";
    std::fs::write(dir.join("meta"), bytes).expect("ambiguous server");
    let mut out = Vec::new();
    let failure = locked(&root.0, "sold", "snew", None, &mut out, &mut Broken)
        .expect_err("a refusal write must propagate");
    assert!(
        matches!(failure, crate::Error::Io(ref why) if why.kind() == std::io::ErrorKind::BrokenPipe)
    );
    assert!(out.is_empty());
    assert_eq!(std::fs::read(dir.join("meta")).expect("meta"), bytes);
    assert!(!root.0.join("sessions/snew").exists());
}
