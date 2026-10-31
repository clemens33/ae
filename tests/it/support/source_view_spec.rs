//! Independent R2c acceptance controls. Frozen by r2c-nav before implementation.
//! A prefix-only/silently truncating view must fail the positive tail assertions.

use super::source_view::{classify, load, product_of};

fn files(input: &[(&str, &str)]) -> Vec<(String, String)> {
    input
        .iter()
        .map(|(p, t)| ((*p).into(), (*t).into()))
        .collect()
}

fn view(text: &str) -> String {
    match classify("src/control.rs", text) {
        Ok(classified) => classified.production,
        Err(failure) => panic!("trusted control was rejected: {failure}"),
    }
}

fn rejected(text: &str, line: usize) {
    let failure = match classify("src/control.rs", text) {
        Ok(_) => panic!("untrusted source was accepted: {text:?}"),
        Err(failure) => failure,
    };
    assert_eq!(failure.file, "src/control.rs");
    assert_eq!(failure.line, line);
    assert!(!failure.what.trim().is_empty());
    assert!(
        failure
            .to_string()
            .contains(&format!("src/control.rs:{line}:"))
    );
}

#[test]
fn production_after_inline_tests_is_visible_at_its_original_offset() {
    // Mutation: stop at the first inline test module. Silent truncation FAILS.
    let text = "fn before() {}\n#[cfg(test)]\nmod tests { fn test_only() { ToolKind::Hidden; } }\nfn after() { ToolKind::FutureVariant; }\n";
    let product = view(text);
    assert_eq!(product.len(), text.len());
    assert_eq!(product.matches('\n').count(), text.matches('\n').count());
    assert!(!product.contains("test_only"));
    assert!(!product.contains("ToolKind::Hidden"));
    for needle in ["fn before()", "fn after()", "ToolKind::FutureVariant"] {
        assert_eq!(product.find(needle), text.find(needle));
    }
}

#[test]
fn individual_test_items_do_not_hide_following_production() {
    // Mutation: stop at a helper item, or leave the helper visible. Both FAIL.
    for gated in [
        "#[cfg(test)]\nconst TEST_ONLY: &str = \"events.jsonl\";\n",
        "#[cfg(test)]\n#[must_use]\npub(crate) const fn test_only() -> bool { true }\n",
        "#[must_use]\n#[cfg(test)]\nfn test_only() {}\n",
        "#[cfg(test)]\nuse crate::test_only;\n",
        "#[cfg(test)]\ninclude!(\"test_only.rs\");\n",
        "#[cfg(test)]\nthread_local! { static TEST_ONLY: u8 = 0; }\n",
    ] {
        let product = view(&format!(
            "fn before() {{}}\n{gated}fn after() {{ ToolKind::FutureVariant; }}\n"
        ));
        assert!(product.contains("fn before()"), "{gated}");
        assert!(product.contains("ToolKind::FutureVariant"), "{gated}");
        assert!(!product.to_lowercase().contains("test_only"), "{gated}");
        assert!(!product.contains("must_use"), "{gated}");
    }
}

#[test]
fn gated_if_else_removes_the_complete_statement_and_preserves_its_tail() {
    // Mutation: stop at the first closing brace, or swallow following code.
    let text = "fn choose() {\nlet live_before = 0;\n#[cfg(test)]\nif test_only() { test_only_one(); } else { test_only_two(); }\nToolKind::FutureVariant;\n}\n";
    let product = view(text);
    assert!(!product.contains("test_only"));
    assert!(!product.contains("else"));
    assert!(product.contains("let live_before = 0;"));
    assert_eq!(
        product.find("ToolKind::FutureVariant"),
        text.find("ToolKind::FutureVariant")
    );
    assert_eq!(product.len(), text.len());
}

#[test]
fn calibrated_nested_variants_fields_and_arms_leave_the_rest_visible() {
    // Mutation: classify all remaining text as test after a nested gate. FAILS.
    let text = "enum Choice {\n#[cfg(test)]\nTestOnly(String),\nLive,\n}\nstruct Row {\n#[cfg(test)]\ntest_only: usize,\nlive: usize,\n}\nfn choose(c: Choice) { match c {\n#[cfg(test)]\nChoice::TestOnly(s) => { consume(s); },\nChoice::Live => live(),\n} }\nfn after() { ToolKind::FutureVariant; }\n";
    let product = view(text);
    assert!(!product.contains("TestOnly"));
    assert!(!product.contains("test_only"));
    assert!(product.contains("Live,"));
    assert!(product.contains("live: usize"));
    assert!(product.contains("Choice::Live => live()"));
    assert!(product.contains("ToolKind::FutureVariant"));
}

#[test]
fn comment_and_literal_gate_text_cannot_change_classification() {
    // Mutation: recognise gates/braces inside comments/raw strings. FAILS.
    let text = concat!(
        "/* outer #[cfg(test)] mod bogus { /* nested } */ } */\n",
        "// #[cfg(test)]\n",
        "const RAW: &str = r###\"\n#[cfg(test)]\nmod fake { }\n// retained literal\n\"###;\n",
        "const NAME: &str = \"events.jsonl\"; // #[cfg(test)]\n",
        "#[cfg(test)]\nmod tests { const S: &str = r#\"} #[cfg(test)]\"#; }\n",
        "fn after() { ToolKind::FutureVariant; }\n",
    );
    let product = view(text);
    assert_eq!(product.len(), text.len());
    assert!(!product.contains("outer"));
    assert!(!product.contains("nested"));
    assert!(!product.contains("mod bogus"));
    assert!(product.contains("#[cfg(test)]\nmod fake { }\n// retained literal"));
    assert!(product.contains("\"events.jsonl\""));
    assert!(!product.contains("const S:"));
    assert_eq!(product.find("fn after()"), text.find("fn after()"));
}

#[test]
fn unrecognised_and_malformed_gates_fail_with_source_location() {
    // Mutation: return an Ok prefix on doubt. Every case FAILS that mutant.
    for tail in [
        "#[cfg(test)]\nlet unrecognised = 0;\nfn after() {}\n",
        "#[cfg(test)]\nfn unfinished() {\n",
        "#[cfg(test)]\nmod tests { fn unfinished() {}\n",
        "#[cfg(test)]\n}\nfn after() {}\n",
        "#[cfg(test)]\n",
    ] {
        rejected(&format!("fn before() {{}}\n{tail}"), 2);
    }
    rejected(
        "fn before() {}\n#[cfg(any(test, feature = \"x\"))]\nmod unclear {}\n",
        2,
    );
}

#[test]
fn unclosed_lexical_regions_fail_at_their_opening_line() {
    // Mutation: accept an unterminated literal/comment and hide later gates.
    for tail in [
        "/* unclosed",
        "const S: &str = \"unclosed",
        "const S: &str = r##\"unclosed",
    ] {
        rejected(&format!("fn before() {{}}\n{tail}"), 2);
    }
}

#[test]
fn conditional_path_attributes_cannot_silently_choose_a_target() {
    // Mutation: ignore an attribute which changes production file identity.
    rejected(
        "fn before() {}\n#[cfg_attr(not(test), path = \"other.rs\")]\nmod redirected;\n",
        2,
    );
}

#[test]
fn aliased_test_files_use_the_path_attribute_and_keep_later_production() {
    // Mutation: resolve by alias name, or include test-only files. FAILS.
    let product = product_of(&files(&[
        ("src/lib.rs", "mod owner;\n"),
        ("src/owner.rs", "#[cfg(test)]\n#[path = \"fixtures/actual.rs\"]\nmod renamed;\nfn after() { ToolKind::FutureVariant; }\n"),
        ("src/fixtures/actual.rs", "fn test_only() { ToolKind::Hidden; }\n"),
    ])).unwrap();
    assert_eq!(
        product
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["src/lib.rs", "src/owner.rs"]
    );
    assert_eq!(product.test_only, ["src/fixtures/actual.rs"]);
    assert!(
        product
            .files
            .iter()
            .find(|f| f.path == "src/owner.rs")
            .unwrap()
            .text
            .contains("ToolKind::FutureVariant")
    );
}

#[test]
fn test_only_file_descendants_are_excluded_transitively() {
    // Mutation: exclude only the registered seed, not its module descendants.
    let product = product_of(&files(&[
        ("src/lib.rs", "#[cfg(test)]\nmod spec;\nfn live() {}\n"),
        ("src/spec.rs", "mod child;\n"),
        ("src/spec/child.rs", "fn test_only() {}\n"),
    ]))
    .unwrap();
    assert_eq!(
        product
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["src/lib.rs"]
    );
    assert_eq!(product.test_only, ["src/spec.rs", "src/spec/child.rs"]);
}

#[test]
fn nested_module_paths_and_inline_test_descendants_have_one_identity() {
    // Mutation: resolve nested registrations relative to the wrong directory.
    let product = product_of(&files(&[
        ("src/lib.rs", "mod owner;\n"),
        ("src/owner.rs", "mod nested { mod live; #[cfg(test)] mod spec; }\n#[cfg(test)] mod tests { mod child; }\nfn after() {}\n"),
        ("src/owner/nested/live.rs", "fn nested_live() {}\n"),
        ("src/owner/nested/spec.rs", "fn test_only() {}\n"),
        ("src/owner/tests/child.rs", "fn test_only() {}\n"),
    ])).unwrap();
    assert_eq!(
        product
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["src/lib.rs", "src/owner.rs", "src/owner/nested/live.rs"]
    );
    assert_eq!(
        product.test_only,
        ["src/owner/nested/spec.rs", "src/owner/tests/child.rs"]
    );
    assert!(
        product
            .files
            .iter()
            .find(|f| f.path == "src/owner.rs")
            .unwrap()
            .text
            .contains("fn after()")
    );
}

#[test]
fn production_reachability_wins_over_a_test_alias() {
    // Mutation: subtract every test registration globally, hiding shared code.
    let product = product_of(&files(&[
        (
            "src/lib.rs",
            "#[path = \"shared.rs\"] mod live;\n#[cfg(test)] #[path = \"shared.rs\"] mod spec;\n",
        ),
        ("src/shared.rs", "fn live() { ToolKind::FutureVariant; }\n"),
    ]))
    .unwrap();
    assert!(product.test_only.is_empty());
    assert!(
        product
            .files
            .iter()
            .any(|f| f.path == "src/shared.rs" && f.text.contains("ToolKind::FutureVariant"))
    );
}

#[test]
fn ambiguous_missing_and_orphaned_files_are_named_failures() {
    // Mutation: silently skip an unresolved file or choose one ambiguous target.
    for (input, expected) in [
        (files(&[("src/lib.rs", "mod missing;\n")]), "src/lib.rs"),
        (
            files(&[
                ("src/lib.rs", "mod child;\n"),
                ("src/child.rs", ""),
                ("src/child/mod.rs", ""),
            ]),
            "src/lib.rs",
        ),
        (
            files(&[
                ("src/lib.rs", "fn live() {}\n"),
                ("src/orphan.rs", "fn unseen() {}\n"),
            ]),
            "src/orphan.rs",
        ),
    ] {
        let failure = match product_of(&input) {
            Ok(_) => panic!("untrusted inventory accepted"),
            Err(failure) => failure,
        };
        assert_eq!(failure.file, expected);
        assert!(!failure.what.is_empty());
        assert!(failure.to_string().contains(expected));
    }
}

#[test]
fn external_test_path_does_not_invent_a_source_file() {
    // Mutation: manufacture a src alias for a registered test outside src.
    let product = product_of(&files(&[(
        "src/lib.rs",
        "#[cfg(test)] #[path = \"../tests/it/control.rs\"] mod renamed;\nfn live() {}\n",
    )]))
    .unwrap();
    assert_eq!(product.files.len(), 1);
    assert!(product.files[0].text.contains("fn live()"));
    assert!(!product.files[0].text.contains("mod renamed"));
}

#[test]
fn external_production_target_cannot_disappear_from_the_inventory() {
    // Mutation: treat every outside-src registration as test-only.
    let failure = match product_of(&files(&[(
        "src/lib.rs",
        "#[path = \"../tests/it/live.rs\"] mod live;\n",
    )])) {
        Ok(_) => panic!("outside-src production target was silently lost"),
        Err(failure) => failure,
    };
    assert_eq!(failure.file, "src/lib.rs");
    assert_eq!(failure.line, 1);
    assert!(!failure.what.is_empty());
}

#[test]
fn loader_refuses_a_missing_directory_and_an_unreadable_source_by_name() {
    // Mutation: flatten read errors or return an empty tree. Deterministic
    // invalid UTF-8 tests the read boundary without permission assumptions.
    let scratch = crate::cli::OwnedScratch::root("r2c", "source");
    let root = scratch.path();
    let missing = load(root, &["missing"])
        .err()
        .expect("missing directory must fail");
    assert!(missing.file.contains("missing"));
    assert!(!missing.what.is_empty());
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::create_dir(root.join("tests")).unwrap();
    std::fs::write(root.join("src/lib.rs"), b"fn live() {}\n").unwrap();
    std::fs::write(root.join("tests/broken.rs"), [0xff]).unwrap();
    let bad = load(root, &["src", "tests"])
        .err()
        .expect("invalid UTF-8 source must fail");
    assert!(bad.file.ends_with("tests/broken.rs"), "{bad}");
    assert!(!bad.what.is_empty());
}
