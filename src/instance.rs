//! Which ae INSTANCE this invocation serves — the ONE owner of its state root,
//! its global config, its own tmux server and the user HOME they sit under.
//!
//! Today there is exactly one instance, the DEFAULT, and every answer here is
//! the one [`crate::doors`] derived before this module existed. The facts are
//! READ at the doors; this module only derives from them, purely, so a caller
//! can ask [`select`] about a shape and an environment it fabricated.

use std::path::{Path, PathBuf};

use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::shape::Shape;

/// The ambient facts a selection is derived from, each read at its own door.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// `$HOME`, empty folded to unset.
    pub home: Option<PathBuf>,
    /// `AE_HOME`, empty folded to unset.
    pub ae_home: Option<PathBuf>,
    /// `CONFIG_FILE`, empty folded to unset; only a CHECKOUT reads it.
    pub config_file: Option<PathBuf>,
}

impl Env {
    /// The facts this process carries, as the doors read them for `shape`.
    #[must_use]
    pub fn read(shape: &Shape) -> Self {
        Self {
            home: crate::doors::home(),
            ae_home: crate::doors::ae_home(),
            config_file: crate::doors::declared_config_file(shape),
        }
    }
}

/// One instance's derived places.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    root: PathBuf,
    config: PathBuf,
}

impl Selection {
    /// Where this instance's state lives.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The global config this instance reads.
    #[must_use]
    pub fn config(&self) -> &Path {
        &self.config
    }
}

/// The instance `shape` and `env` select, or `None` when no state root can be
/// derived at all.
///
/// A published core's state is where it SITS, whatever the environment says;
/// a checkout honours `AE_HOME`, else `$HOME/.ae`. The config is the checkout's
/// `CONFIG_FILE` when declared, else `<root>/config`.
#[must_use]
pub fn select(shape: &Shape, env: &Env) -> Option<Selection> {
    let root = match shape.published_home() {
        Some(home) => home.to_path_buf(),
        None => env
            .ae_home
            .clone()
            .or_else(|| env.home.as_ref().map(|home| home.join(".ae")))?,
    };
    let config = config_for(shape, env, &root);
    Some(Selection { root, config })
}

/// The global config read for `root`: the checkout's declared `CONFIG_FILE`,
/// else `<root>/config`.
#[must_use]
pub fn config_for(shape: &Shape, env: &Env, root: &Path) -> PathBuf {
    match &env.config_file {
        Some(named) if shape.honours_environment() => named.clone(),
        _ => root.join("config"),
    }
}

/// The directory under a base that holds the named instance roots.
pub const INSTANCES: &str = "instances";

/// The one instance name the checkout's `ae-dev` server already spells.
const RESERVED: &str = "dev";

/// Whether `name` can name an instance: `^[a-z0-9][a-z0-9-]{0,23}$`, lowercase
/// because APFS folds case, and never [`RESERVED`].
#[must_use]
pub fn is_instance_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    name != RESERVED
        && (1..=24).contains(&bytes.len())
        && bytes.first() != Some(&b'-')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// The user HOME a state root sits under — what a publish, a command link and
/// a harness's default config home are spelled from. ONE owner, so nothing
/// re-derives it as `root.parent()` at its own site.
#[must_use]
pub fn user_home(root: &Path) -> Option<PathBuf> {
    root.parent().map(Path::to_path_buf)
}

/// The name of the tmux server this instance launches on and owns.
#[must_use]
pub fn own_server_name() -> String {
    crate::doors::DEFAULT_SERVER_NAME.to_owned()
}

/// [`own_server_name`] as the server it selects.
#[must_use]
pub fn own_server() -> ServerId {
    ServerId::Selected(Selector::Name(own_server_name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(home: &str) -> Shape {
        Shape::Installed {
            home: PathBuf::from(home),
            version_dir: PathBuf::from(format!("{home}/versions/2026.10.1")),
            version: "2026.10.1".to_owned(),
        }
    }

    fn env(home: Option<&str>, ae_home: Option<&str>, config: Option<&str>) -> Env {
        Env {
            home: home.map(PathBuf::from),
            ae_home: ae_home.map(PathBuf::from),
            config_file: config.map(PathBuf::from),
        }
    }

    fn places(shape: &Shape, env: &Env) -> Option<(PathBuf, PathBuf)> {
        select(shape, env).map(|s| (s.root().to_path_buf(), s.config().to_path_buf()))
    }

    #[test]
    fn a_published_core_keeps_its_own_home_whatever_the_environment_declares() {
        let shape = installed("/u/me/.ae");
        for declared in [
            env(Some("/u/me"), None, None),
            env(Some("/u/me"), Some("/elsewhere"), Some("/elsewhere/config")),
            env(None, None, None),
        ] {
            assert_eq!(
                places(&shape, &declared),
                Some((
                    PathBuf::from("/u/me/.ae"),
                    PathBuf::from("/u/me/.ae/config")
                )),
                "{declared:?}"
            );
        }
        let displaced = Shape::Displaced {
            home: PathBuf::from("/u/me/.ae"),
            declared: PathBuf::from("/u/other"),
        };
        assert_eq!(
            places(&displaced, &env(Some("/u/other"), Some("/x"), Some("/x/c"))),
            Some((
                PathBuf::from("/u/me/.ae"),
                PathBuf::from("/u/me/.ae/config")
            ))
        );
    }

    #[test]
    fn a_checkout_honours_ae_home_then_home_and_its_declared_config() {
        let checkout = Shape::Checkout;
        assert_eq!(
            places(&checkout, &env(Some("/u/me"), None, None)),
            Some((
                PathBuf::from("/u/me/.ae"),
                PathBuf::from("/u/me/.ae/config")
            ))
        );
        assert_eq!(
            places(&checkout, &env(Some("/u/me"), Some("/u/me/.ae-dev"), None)),
            Some((
                PathBuf::from("/u/me/.ae-dev"),
                PathBuf::from("/u/me/.ae-dev/config")
            ))
        );
        assert_eq!(
            places(&checkout, &env(None, Some("/s"), Some("/c/config"))),
            Some((PathBuf::from("/s"), PathBuf::from("/c/config")))
        );
        assert_eq!(places(&checkout, &env(None, None, Some("/c"))), None);
    }

    #[test]
    fn the_user_home_is_the_state_root_parent_and_the_server_is_ae() {
        assert_eq!(
            user_home(Path::new("/u/me/.ae")),
            Some(PathBuf::from("/u/me"))
        );
        assert_eq!(user_home(Path::new("/")), None);
        assert_eq!(own_server_name(), "ae");
        for good in ["a", "mic", "work-2", "0", &"x".repeat(24)] {
            assert!(is_instance_name(good), "{good}");
        }
        for bad in ["", "dev", "Work", "-a", "a_b", "a.b", ".a", &"x".repeat(25)] {
            assert!(!is_instance_name(bad), "{bad}");
        }
        assert_eq!(
            own_server(),
            ServerId::Selected(Selector::Name("ae".to_owned()))
        );
    }
}
