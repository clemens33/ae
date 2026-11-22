//! The page and its static files, embedded at build time. No build step: the
//! three vendored modules are the upstream bytes `assets/VENDOR.md` records,
//! `hooks.mjs` with its one recorded rewrite. `/assets/licenses.txt` carries
//! both upstream licence texts, byte for byte, each under a heading that names
//! its package, version and pinned tarball.

use std::sync::OnceLock;

/// One static file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// Its media type.
    pub content_type: &'static str,
    /// Its bytes.
    pub bytes: &'static [u8],
}

const HTML: &str = "text/html; charset=utf-8";
const SCRIPT: &str = "text/javascript; charset=utf-8";
const STYLE: &str = "text/css; charset=utf-8";
const TEXT: &str = "text/plain; charset=utf-8";

/// Each vendored package's licence: its heading and its upstream bytes.
const LICENCES: [(&str, &[u8]); 2] = [
    (
        "preact 11.2.0 (preact.mjs, hooks.mjs): package/LICENSE of https://registry.npmjs.org/preact/-/preact-11.2.0.tgz",
        include_bytes!("assets/LICENSE-preact.txt"),
    ),
    (
        "htm 3.1.1 (htm.mjs): package/LICENSE of https://registry.npmjs.org/htm/-/htm-3.1.1.tgz",
        include_bytes!("assets/LICENSE-htm.txt"),
    ),
];

/// The licence page: each heading, a blank row, the upstream text unchanged,
/// then a blank row before the next heading. Composed once.
fn licences() -> &'static [u8] {
    static PAGE: OnceLock<Vec<u8>> = OnceLock::new();
    PAGE.get_or_init(|| {
        let mut page = Vec::new();
        for (heading, text) in LICENCES {
            page.extend_from_slice(heading.as_bytes());
            page.extend_from_slice(b"\n\n");
            page.extend_from_slice(text);
            let gap: &[u8] = if text.ends_with(b"\n") {
                b"\n"
            } else {
                b"\n\n"
            };
            page.extend_from_slice(gap);
        }
        page
    })
}

/// The file served at `path`, or `None` for any other path.
#[must_use]
pub fn find(path: &str) -> Option<Asset> {
    let (content_type, bytes): (&'static str, &'static [u8]) = match path {
        "/" => (HTML, include_bytes!("assets/index.html")),
        "/assets/app.mjs" => (SCRIPT, include_bytes!("assets/app.mjs")),
        "/assets/app.css" => (STYLE, include_bytes!("assets/app.css")),
        "/assets/preact.mjs" => (SCRIPT, include_bytes!("assets/preact.mjs")),
        "/assets/hooks.mjs" => (SCRIPT, include_bytes!("assets/hooks.mjs")),
        "/assets/htm.mjs" => (SCRIPT, include_bytes!("assets/htm.mjs")),
        "/assets/licenses.txt" => (TEXT, licences()),
        _ => return None,
    };
    Some(Asset {
        content_type,
        bytes,
    })
}
