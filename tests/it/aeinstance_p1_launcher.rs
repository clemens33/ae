//! Included as a unit spec: the launcher is intentionally crate-private.
use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::shape::Shape;
use std::path::Path;

#[test]
fn default_bindings_keep_the_link_and_checkout_bakes_all_namespace_doors() {
    let root = Path::new("/spec-home/.ae");
    let core = Path::new("/checkout core/ae");
    let config = Path::new("/custom config");
    let installed = Shape::Installed {
        home: root.to_path_buf(),
        version_dir: root.join("versions").join(crate::VERSION),
        version: crate::VERSION.to_owned(),
    };
    let named = ServerId::Selected(Selector::Name("ae-dev".to_owned()));
    assert_eq!(
        super::picker_launcher(&installed, core, root, config, &named),
        ["/spec-home/.local/bin/ae"]
    );
    for (server, value, kind) in [
        (named, "ae-dev", "name"),
        (
            ServerId::Selected(Selector::Socket("/private socket".into())),
            "/private socket",
            "socket",
        ),
    ] {
        assert_eq!(
            super::picker_launcher(&Shape::Checkout, core, root, config, &server),
            [
                "env".to_owned(),
                "AE_HOME=/spec-home/.ae".to_owned(),
                "CONFIG_FILE=/custom config".to_owned(),
                format!("AE_TMUX_SERVER={value}"),
                format!("AE_TMUX_SERVER_KIND={kind}"),
                "/checkout core/ae".to_owned(),
            ]
        );
    }
}
