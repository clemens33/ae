//! S1a static asset pins. Browser behavior is exercised by the separate Chrome smoke.

use std::path::Path;

const ASSETS: &str = "src/l5_ui/web/assets";
const PREACT: &str = "bdcbd9abdf2c77c5c45326eab42191e949222d479d0d5f69467387a77040cb55";
const HTM: &str = "ab33dd3f38059b9be4d5f5350128eefb2356639c4e0bbe9d9e8b3ba75847e9e4";
const HOOKS: &str = "427a330c992595bfd3f6cfe43eb6ba7a238ea1a7fa99c0e4a8f380a30dde473b";
const UPSTREAM_HOOKS: &str = "e862ab2973aa6455eaa0f77c5d832d2157d6be59a17c8ba546550a575b58c6b4";
const SERVED_IMPORT: &str = r#"from"./preact.mjs""#;
const UPSTREAM_IMPORT: &str = r#"from"preact""#;

#[allow(
    clippy::disallowed_methods,
    reason = "read the repository's pinned static fixtures"
)]
fn asset(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(ASSETS)
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn source(name: &str) -> String {
    String::from_utf8(asset(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn compact(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect()
}

#[test]
fn approved_vendor_bytes_match_both_ends_of_the_hooks_rewrite() {
    for (file, digest) in [
        ("preact.mjs", PREACT),
        ("htm.mjs", HTM),
        ("hooks.mjs", HOOKS),
    ] {
        assert_eq!(ae::install::sha256_hex(&asset(file)), digest, "{file}");
    }

    let hooks = source("hooks.mjs");
    assert_eq!(hooks.matches(SERVED_IMPORT).count(), 1);
    assert!(!hooks.contains(UPSTREAM_IMPORT));
    let upstream = hooks.replacen(SERVED_IMPORT, UPSTREAM_IMPORT, 1);
    assert_eq!(ae::install::sha256_hex(upstream.as_bytes()), UPSTREAM_HOOKS);

    let vendor = source("VENDOR.md");
    for fact in [
        PREACT,
        HTM,
        HOOKS,
        UPSTREAM_HOOKS,
        SERVED_IMPORT,
        UPSTREAM_IMPORT,
        "https://registry.npmjs.org/preact/-/preact-11.2.0.tgz",
        "https://registry.npmjs.org/htm/-/htm-3.1.1.tgz",
    ] {
        assert!(vendor.contains(fact), "VENDOR.md omits {fact}");
    }
    assert!(vendor.contains("MIT"), "VENDOR.md omits the license");
}

#[test]
fn the_index_loads_external_modules_and_styles_with_no_inline_code() {
    let markup = source("index.html");
    let normalized = markup.to_ascii_lowercase();
    assert!(!normalized.contains("<style"));
    assert!(!normalized.contains("javascript:"));

    assert_eq!(normalized.matches("<script").count(), 1);
    let mut app_entry = false;
    for (start, _) in normalized.match_indices("<script") {
        let (tag, body) = normalized[start..]
            .split_once('>')
            .unwrap_or_else(|| panic!("unterminated script start tag"));
        assert!(compact(tag).contains("src="), "missing external src: {tag}");
        let (script, _) = body
            .split_once("</script")
            .unwrap_or_else(|| panic!("missing script end tag"));
        assert!(script.trim().is_empty(), "inline script: {script}");
        app_entry |= compact(tag).contains(r#"src="/assets/app.mjs""#)
            && compact(tag).contains(r#"type="module""#);
    }
    assert!(app_entry, "the mount's module entry is missing");
    for (tag, _) in normalized
        .split('<')
        .filter_map(|text| text.split_once('>'))
    {
        for (before, _) in tag.match_indices('=') {
            let name = tag[..before]
                .trim_end()
                .rsplit(|character: char| !character.is_ascii_alphanumeric() && character != '-')
                .next()
                .unwrap_or_default();
            assert_ne!(name, "style", "inline style: {tag}");
            assert!(!name.starts_with("on"), "inline event handler: {tag}");
        }
    }
    let markup = compact(&normalized);
    assert!(markup.contains(r#"href="/assets/app.css""#));
    assert!(markup.contains(r#"<divid="root"></div>"#));
}

#[test]
fn the_application_source_keeps_agent_text_away_from_html_and_code_sinks() {
    let app = source("app.mjs");
    let words = app.split(|character: char| {
        !character.is_ascii_alphanumeric() && !matches!(character, '_' | '$')
    });
    for forbidden in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "dangerouslySetInnerHTML",
        "createContextualFragment",
        "parseFromString",
        "setHTML",
        "setHTMLUnsafe",
        "srcdoc",
        "eval",
        "localStorage",
        "indexedDB",
    ] {
        assert!(!words.clone().any(|word| word == forbidden), "{forbidden}");
    }

    let app = compact(&app);
    for forbidden in [
        "newFunction(",
        "document.write(",
        "document.writeln(",
        "document.cookie",
        r#"document["cookie"]"#,
        "document['cookie']",
        "style=",
        "style:",
        r#""style":"#,
        "'style':",
        ".style.",
    ] {
        assert!(!app.contains(forbidden), "{forbidden}");
    }
    for timer in ["setTimeout(", "setInterval("] {
        for (at, _) in app.match_indices(timer) {
            let first = app[at + timer.len()..].trim_start_matches('(');
            assert!(
                !first.starts_with(['"', '\'', '`']),
                "string argument to {timer}"
            );
        }
    }
}

#[test]
fn application_network_calls_use_the_api_prefix_and_literal_endpoint_names() {
    let app = compact(&source("app.mjs"));
    assert_eq!(app.matches("fetch(").count(), 1);
    assert!(
        app.contains(r#"fetch("/api/"+path,"#) || app.contains("fetch('/api/'+path,"),
        "the sole fetch must use the API prefix"
    );
    assert!(app.contains("from\"./preact.mjs\"") || app.contains("from'./preact.mjs'"));
    assert!(app.contains("from\"./hooks.mjs\"") || app.contains("from'./hooks.mjs'"));
    assert!(app.contains("from\"./htm.mjs\"") || app.contains("from'./htm.mjs'"));
    for endpoint in ["session", "state"] {
        assert!(
            app.contains(&format!("api(\"{endpoint}\""))
                || app.contains(&format!("api('{endpoint}'")),
            "missing API consumer for {endpoint}"
        );
    }
    for (at, _) in app.match_indices("api(") {
        if app[..at].ends_with("function") {
            continue;
        }
        let arguments = &app[at + "api(".len()..];
        let mut chars = arguments.chars();
        let quote = chars.next().unwrap_or_default();
        assert!(matches!(quote, '"' | '\''), "computed API path");
        let (path, rest) = chars
            .as_str()
            .split_once(quote)
            .unwrap_or_else(|| panic!("unterminated API path"));
        assert!(
            (matches!(path, "session" | "state" | "nonce" | "send")
                && rest.starts_with([',', ')']))
                || (path == "topic/" && rest.starts_with('+')),
            "unexpected API path: {path}"
        );
    }
    for forbidden in [
        "://",
        r#""//"#,
        "'//",
        "XMLHttpRequest",
        "WebSocket",
        "EventSource",
        "sendBeacon",
        "window.open(",
        "location.assign(",
        "location.replace(",
        "location.href=",
        "location.hash=",
        "href=",
        "href:",
    ] {
        assert!(!app.contains(forbidden), "unexpected URL use: {forbidden}");
    }
}
