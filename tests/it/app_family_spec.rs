//! The `src/app.rs` module family: the retained parent and every child it registers, read as one text.
//!
//! `src/app.rs` used to hold the coordinator whole. A negative text check over that one file stays green when its code
//! moves into a child, so this guard reads the parent together with each production child it registers. The base
//! siblings (the 23 children the parent registered before any code left it) are not part of the family: their bodies
//! have their own guards.
//!
//! Membership comes from the syntax-aware registrations of `source_view::classify` and the resolver `target_of`, never
//! from matching `mod` lines. A row is one `(path, production text)` pair of the product view, so a name never names
//! another file.
//!
//! What the family accepts and refuses, no more: an ungated registration is a child and its file is read. A gated
//! registration (`#[cfg(test)]`) is test-only and is not part of the family. A registration redirected by `#[path]`
//! or nested in an inline module is an error. A `cfg` that mentions `test` in a form `source_view` does not recognise,
//! and a `cfg_attr` that could change `cfg` or `path`, make the classification fail. Every other attribute (`allow`,
//! `cfg(not(test))`, `cfg(unix)`) is not inspected: the child is accepted and read.

use crate::support::source_view::{
    Product, Registration, Target, classify, product, product_of, target_of, tree,
};

/// The children `src/app.rs` registered ungated before the split, sorted. They are the only exclusions: a child cut
/// from the parent is never listed here.
const OUTSIDE: [&str; 23] = [
    "backend",
    "cue",
    "draw",
    "filedoor",
    "fleet",
    "handover",
    "journals",
    "lane",
    "lanemd",
    "loader",
    "maintainer",
    "markdown",
    "menu",
    "model",
    "overview",
    "paneview",
    "peek",
    "reader",
    "route",
    "select",
    "settings",
    "tty",
    "upgrade",
];

/// Calls that only the loader may use to build live seat facts from its own reads.
const NEEDLES: [&str; 2] = ["live_seat_facts(", "project_live_seats("];

const PARENT: &str = "src/app.rs";

type Rows = Vec<(String, String)>;

/// The text of the family: `src/app.rs` first, then the production files below each ungated child that
/// `src/app.rs` registers and that is not an `OUTSIDE` sibling, in registration order.
fn family(files: &[(String, String)], product: &Product) -> Result<Rows, String> {
    let known: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
    let Some((_, parent)) = files.iter().find(|(path, _)| path == PARENT) else {
        return Err(format!("{PARENT} is not among the loaded files"));
    };
    let text_of = |path: &str| {
        product
            .files
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.text.clone())
    };
    let Some(parent_text) = text_of(PARENT) else {
        return Err(format!("{PARENT} is not a production file"));
    };
    let mut rows: Rows = vec![(PARENT.to_owned(), parent_text)];
    let classified = classify(PARENT, parent).map_err(|failure| failure.to_string())?;
    for reg in classified.registrations.iter().filter(|reg| !reg.gated) {
        if reg.path_attr.is_some() || !reg.nesting.is_empty() {
            return Err(format!(
                "{PARENT}:{}: `mod {}` is registered through #[path] or an inline module, which the family does not read",
                reg.line, reg.name
            ));
        }
        if OUTSIDE.contains(&reg.name.as_str()) {
            continue;
        }
        let target = child_file(reg, &known)?;
        let below = child_dir(&target);
        let mut found = false;
        for file in &product.files {
            let own = file.path == target || file.path.starts_with(&below);
            if own && !rows.iter().any(|(path, _)| *path == file.path) {
                rows.push((file.path.clone(), file.text.clone()));
            }
            found |= own;
        }
        if !found {
            return Err(format!(
                "{PARENT}:{}: `mod {}` resolves to {target}, which is not a production file",
                reg.line, reg.name
            ));
        }
    }
    Ok(rows)
}

fn child_file(reg: &Registration, known: &[&str]) -> Result<String, String> {
    match target_of(PARENT, reg, known).map_err(|failure| failure.to_string())? {
        Target::Src(path) => Ok(path),
        Target::Outside(path) => Err(format!(
            "{PARENT}:{}: `mod {}` resolves to {path}, outside src/",
            reg.line, reg.name
        )),
    }
}

/// The directory that holds the submodules of a child file: `a.rs` owns `a/`, `a/mod.rs` owns `a/`. The `mod.rs` is
/// matched as a whole path component, so a file such as `loadermod.rs` owns `loadermod/`, never `loader`.
fn child_dir(target: &str) -> String {
    match target.strip_suffix("/mod.rs") {
        Some(dir) => format!("{dir}/"),
        None => format!("{}/", target.trim_end_matches(".rs")),
    }
}

/// Every `path call` where a family row holds one of the `NEEDLES`.
fn offenders(rows: &[(String, String)]) -> Vec<String> {
    let mut hits = Vec::new();
    for (path, text) in rows {
        for call in NEEDLES {
            if text.contains(call) {
                hits.push(format!("{path} {call}"));
            }
        }
    }
    hits
}

/// A copy of the product view where `path` also holds `needle` as production text.
fn injected(product: &Product, path: &str, needle: &str) -> Product {
    let mut copy = product.clone();
    let Some(file) = copy.files.iter_mut().find(|file| file.path == path) else {
        panic!("{path} is not a production file of the control");
    };
    file.text.push_str("\nfn injected() { ");
    file.text.push_str(needle);
    file.text.push_str("); }\n");
    copy
}

fn real_family() -> Rows {
    match family(tree(), product()) {
        Ok(rows) => rows,
        Err(why) => panic!("{why}"),
    }
}

/// The ungated registrations of the real `src/app.rs`, by name.
fn registered() -> Vec<String> {
    let Some((_, text)) = tree().iter().find(|(path, _)| path == PARENT) else {
        panic!("{PARENT} was not loaded");
    };
    match classify(PARENT, text) {
        Ok(classified) => classified
            .registrations
            .into_iter()
            .filter(|reg| !reg.gated)
            .map(|reg| reg.name)
            .collect(),
        Err(failure) => panic!("{failure}"),
    }
}

/// A control tree: `src/lib.rs` registers `app`, `src/app.rs` holds `app`, and each child is a file below
/// `src/app/`. Returns the loaded files and the product view built from them.
fn synthetic(app: &str, children: &[(&str, &str)]) -> (Vec<(String, String)>, Product) {
    let mut files: Vec<(String, String)> = vec![
        ("src/lib.rs".to_owned(), "mod app;\n".to_owned()),
        (PARENT.to_owned(), app.to_owned()),
    ];
    for (path, text) in children {
        files.push(((*path).to_owned(), (*text).to_owned()));
    }
    match product_of(&files) {
        Ok(product) => (files, product),
        Err(failure) => panic!("the control tree was rejected: {failure}"),
    }
}

fn paths(rows: &[(String, String)]) -> Vec<&str> {
    rows.iter().map(|(path, _)| path.as_str()).collect()
}

fn built(app: &str, children: &[(&str, &str)]) -> Rows {
    let (files, product) = synthetic(app, children);
    match family(&files, &product) {
        Ok(rows) => rows,
        Err(why) => panic!("the control family was refused: {why}"),
    }
}

#[test]
fn the_app_family_builds_no_live_seat_facts() {
    let rows = real_family();
    assert_eq!(rows[0].0, PARENT, "the parent is the first row");
    assert_eq!(
        offenders(&rows),
        Vec::<String>::new(),
        "only the loader builds live seat facts"
    );
}

#[test]
fn the_base_siblings_are_exactly_the_23_the_parent_registers() {
    assert!(
        OUTSIDE.windows(2).all(|pair| pair[0] < pair[1]),
        "the exclusion list is sorted and has no duplicate"
    );
    let names = registered();
    for sibling in OUTSIDE {
        assert!(
            names.iter().any(|name| name == sibling),
            "{sibling} is no longer registered by {PARENT}: an exclusion never stands for a child that left"
        );
    }
}

#[test]
fn every_row_path_resolves_from_a_registered_name() {
    let rows = real_family();
    let names = registered();
    let children: Vec<&String> = names
        .iter()
        .filter(|name| !OUTSIDE.contains(&name.as_str()))
        .collect();
    for (path, _) in rows.iter().skip(1) {
        let rest = path
            .strip_prefix("src/app/")
            .unwrap_or_else(|| panic!("{path} is not below src/app/"));
        let stem = rest.split(['/', '.']).next().unwrap_or(rest);
        assert!(
            children.iter().any(|name| name.as_str() == stem),
            "{path} is a row of no registered child"
        );
    }
    for child in &children {
        let own = format!("src/app/{child}");
        assert!(
            rows.iter().skip(1).any(|(path, _)| path.starts_with(&own)),
            "the registered child {child} has no row"
        );
    }
}

#[test]
fn a_needle_in_the_retained_parent_is_found_for_each_call() {
    for needle in NEEDLES {
        let copy = injected(product(), PARENT, needle);
        let rows = family(tree(), &copy).unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(offenders(&rows), [format!("{PARENT} {needle}")]);
    }
}

#[test]
fn a_needle_in_every_extracted_child_is_found_for_each_call() {
    let rows = real_family();
    for (path, _) in rows.iter().skip(1) {
        for needle in NEEDLES {
            let copy = injected(product(), path, needle);
            let family = family(tree(), &copy).unwrap_or_else(|why| panic!("{why}"));
            assert_eq!(offenders(&family), [format!("{path} {needle}")]);
        }
    }
}

#[test]
fn a_needle_in_a_base_sibling_is_outside_the_family() {
    for needle in NEEDLES {
        let copy = injected(product(), "src/app/loader.rs", needle);
        let rows = family(tree(), &copy).unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(offenders(&rows), Vec::<String>::new());
        assert_eq!(paths(&rows), paths(&real_family()));
    }
}

#[test]
fn identical_text_in_two_children_keeps_two_rows() {
    let same = "pub(super) fn same() {}\n";
    let app = "mod a;\nmod b;\n";
    let (files, product) = synthetic(app, &[("src/app/a.rs", same), ("src/app/b.rs", same)]);
    let rows = family(&files, &product).unwrap_or_else(|why| panic!("{why}"));
    assert_eq!(paths(&rows), [PARENT, "src/app/a.rs", "src/app/b.rs"]);
    let copy = injected(&product, "src/app/a.rs", NEEDLES[0]);
    let rows = family(&files, &copy).unwrap_or_else(|why| panic!("{why}"));
    assert_eq!(offenders(&rows), ["src/app/a.rs live_seat_facts("]);
}

#[test]
fn an_omitted_parent_is_an_error_not_an_empty_family() {
    let (files, product) = synthetic("mod a;\n", &[("src/app/a.rs", "fn a() {}\n")]);
    let without_file: Vec<(String, String)> = files
        .iter()
        .filter(|(path, _)| path != PARENT)
        .cloned()
        .collect();
    assert!(family(&without_file, &product).is_err());
    let mut without_product = product.clone();
    without_product.files.retain(|file| file.path != PARENT);
    assert!(family(&files, &without_product).is_err());
}

#[test]
fn a_comment_or_raw_string_mod_decoy_registers_nothing() {
    let app = "// mod ghost;\nconst S: &str = r#\"mod ghost;\"#;\n/* mod phantom; */\nmod real;\n";
    let rows = built(app, &[("src/app/real.rs", "fn real() {}\n")]);
    assert_eq!(paths(&rows), [PARENT, "src/app/real.rs"]);
    let decoyed = [
        ("src/lib.rs".to_owned(), "mod app;\n".to_owned()),
        (PARENT.to_owned(), app.to_owned()),
        ("src/app/real.rs".to_owned(), "fn real() {}\n".to_owned()),
        ("src/app/ghost.rs".to_owned(), "fn ghost() {}\n".to_owned()),
    ];
    assert!(
        product_of(&decoyed).is_err(),
        "a file only a decoy names is registered by nothing, so the product view refuses the tree"
    );
}

#[test]
fn a_gated_registration_is_not_family() {
    let app = "mod keys;\n#[cfg(test)]\nmod keys_spec;\n";
    let rows = built(
        app,
        &[
            ("src/app/keys.rs", "fn keys() {}\n"),
            ("src/app/keys_spec.rs", "fn keys_spec() {}\n"),
        ],
    );
    assert_eq!(paths(&rows), [PARENT, "src/app/keys.rs"]);
}

#[test]
fn a_redirected_production_registration_is_refused() {
    let app = "#[path = \"elsewhere/y.rs\"]\nmod y;\n";
    let (files, product) = synthetic(app, &[("src/elsewhere/y.rs", "fn y() {}\n")]);
    let Err(why) = family(&files, &product) else {
        panic!("a #[path] registration was read as a plain child");
    };
    assert!(why.contains("`mod y`"), "{why}");
}

#[test]
fn a_nested_production_registration_is_refused() {
    let nested = "mod inline {\n    mod deep;\n}\n";
    let (files, product) = synthetic(nested, &[("src/app/inline/deep.rs", "fn deep() {}\n")]);
    let Err(why) = family(&files, &product) else {
        panic!("a nested registration was read as a plain child");
    };
    assert!(why.contains("`mod deep`"), "{why}");
}

#[test]
fn an_unrecognised_cfg_or_cfg_attr_fails_the_classification_so_the_family_is_refused() {
    for attr in [
        "#[cfg(all(test, unix))]",
        "#[cfg_attr(unix, path = \"a.rs\")]",
    ] {
        let files = [
            ("src/lib.rs".to_owned(), "mod app;\n".to_owned()),
            (PARENT.to_owned(), format!("{attr}\nmod z;\n")),
            ("src/app/z.rs".to_owned(), "fn z() {}\n".to_owned()),
        ];
        assert!(
            product_of(&files).is_err(),
            "{attr} must make the product view fail, not pass quietly"
        );
        assert!(
            family(&files, product()).is_err(),
            "{attr} must make the family refuse the parent"
        );
    }
}

#[test]
fn other_attributes_do_not_stop_a_child_being_read() {
    let rows = built(
        "#[cfg(not(test))]\nmod kept;\n#[allow(dead_code)]\nmod also;\n#[cfg(unix)]\nmod host;\n",
        &[
            ("src/app/kept.rs", "fn kept() {}\n"),
            ("src/app/also.rs", "fn also() {}\n"),
            ("src/app/host.rs", "fn host() {}\n"),
        ],
    );
    assert_eq!(
        paths(&rows),
        [
            PARENT,
            "src/app/kept.rs",
            "src/app/also.rs",
            "src/app/host.rs"
        ],
        "a child with an attribute the guard does not inspect is read, not dropped"
    );
}

#[test]
fn a_child_does_not_take_the_files_of_a_sibling_whose_name_starts_with_its_own() {
    let rows = built(
        "mod load;\nmod loader;\n",
        &[
            ("src/app/load.rs", "fn load() {}\n"),
            ("src/app/loader.rs", "mod deep;\n"),
            ("src/app/loader/deep.rs", "fn deep() {}\n"),
        ],
    );
    assert_eq!(paths(&rows), [PARENT, "src/app/load.rs"]);
}

#[test]
fn pub_and_pub_crate_registrations_are_family_children() {
    let rows = built(
        "pub mod a;\npub(crate) mod b;\nmod c;\n",
        &[
            ("src/app/a.rs", "fn a() {}\n"),
            ("src/app/b.rs", "fn b() {}\n"),
            ("src/app/c.rs", "fn c() {}\n"),
        ],
    );
    assert_eq!(
        paths(&rows),
        [PARENT, "src/app/a.rs", "src/app/b.rs", "src/app/c.rs"]
    );
}

#[test]
fn a_child_subdirectory_is_family_with_its_child() {
    let rows = built(
        "mod keys;\nmod mid;\nmod loader;\n",
        &[
            ("src/app/keys.rs", "mod inner;\n"),
            ("src/app/keys/inner.rs", "fn inner() {}\n"),
            ("src/app/mid/mod.rs", "mod part;\n"),
            ("src/app/mid/part.rs", "fn part() {}\n"),
            ("src/app/loader.rs", "mod deep;\n"),
            ("src/app/loader/deep.rs", "fn deep() {}\n"),
        ],
    );
    assert_eq!(
        paths(&rows),
        [
            PARENT,
            "src/app/keys.rs",
            "src/app/keys/inner.rs",
            "src/app/mid/mod.rs",
            "src/app/mid/part.rs"
        ],
        "the loader subtree belongs to a base sibling and stays out"
    );
}

#[test]
fn an_outside_name_is_excluded_and_any_other_registered_name_is_included() {
    let app = "mod loader;\nmod draw;\nmod keys;\nmod loaders;\n";
    let children = [
        ("src/app/loader.rs", "fn l() {}\n"),
        ("src/app/draw.rs", "fn d() {}\n"),
        ("src/app/keys.rs", "fn k() {}\n"),
        ("src/app/loaders.rs", "fn s() {}\n"),
    ];
    let rows = built(app, &children);
    assert_eq!(
        paths(&rows),
        [PARENT, "src/app/keys.rs", "src/app/loaders.rs"],
        "an exclusion is an exact name, never a prefix"
    );
    let (files, product) = synthetic(app, &children);
    for (path, hit) in [
        ("src/app/loader.rs", false),
        ("src/app/draw.rs", false),
        ("src/app/keys.rs", true),
        ("src/app/loaders.rs", true),
    ] {
        let copy = injected(&product, path, NEEDLES[1]);
        let rows = family(&files, &copy).unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(!offenders(&rows).is_empty(), hit, "{path}");
    }
}

#[test]
fn a_registered_child_that_is_not_a_production_file_is_an_error() {
    let (files, product) = synthetic("mod a;\n", &[("src/app/a.rs", "fn a() {}\n")]);
    let mut without_child = product.clone();
    without_child
        .files
        .retain(|file| file.path != "src/app/a.rs");
    let Err(why) = family(&files, &without_child) else {
        panic!("a registered child without a production file was skipped");
    };
    assert!(why.contains("`mod a`"), "{why}");
}

#[test]
fn a_child_file_whose_name_ends_in_mod_rs_owns_no_directory_by_that_prefix() {
    let app = "mod loadermod;\nmod loader;\n";
    let children = [
        ("src/app/loadermod.rs", "fn lm() {}\n"),
        ("src/app/loader.rs", "mod deep;\n"),
        ("src/app/loader/deep.rs", "fn deep() {}\n"),
    ];
    let rows = built(app, &children);
    assert_eq!(
        paths(&rows),
        [PARENT, "src/app/loadermod.rs"],
        "loadermod.rs is a file child: the loader files stay outside"
    );
    let (files, product) = synthetic(app, &children);
    for path in ["src/app/loader.rs", "src/app/loader/deep.rs"] {
        let copy = injected(&product, path, NEEDLES[0]);
        let rows = family(&files, &copy).unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(
            offenders(&rows),
            Vec::<String>::new(),
            "a needle in {path} belongs to the loader"
        );
    }
}

#[test]
fn a_mod_rs_child_owns_its_directory_up_to_the_boundary_only() {
    let app = "mod load;\nmod loader;\n";
    let children = [
        ("src/app/load/mod.rs", "mod part;\n"),
        ("src/app/load/part.rs", "fn part() {}\n"),
        ("src/app/loader.rs", "mod deep;\n"),
        ("src/app/loader/deep.rs", "fn deep() {}\n"),
    ];
    let rows = built(app, &children);
    assert_eq!(
        paths(&rows),
        [PARENT, "src/app/load/mod.rs", "src/app/load/part.rs"],
        "load/ ends at its slash: loader.rs and loader/ stay outside"
    );
}
