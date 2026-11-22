//! S1a addendum. The public license response carries the upstream texts for the
//! immutable Preact and htm modules it accompanies.

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ae::web::clock::ManualClock;
use ae::web::server::{self, NoProbe, Options, Running};
use ae::web::state::{Read, Source};

const WAIT: Duration = Duration::from_secs(5);
const VENDOR_DIR: &str = "src/l5_ui/web/assets";

struct Empty;

impl Source for Empty {
    fn read(&self) -> Read {
        Read::default()
    }
}

struct Listener(Option<Running>);

impl Listener {
    fn start() -> Self {
        let running = server::start(Options {
            port: 0,
            clock: Arc::new(ManualClock::new(1_795_000_000)),
            source: Arc::new(Empty),
            probe: Arc::new(NoProbe),
            links: Arc::new(|_| {}),
            version: "license-fixture".into(),
        })
        .unwrap_or_else(|error| panic!("start listener: {error}"));
        Self(Some(running))
    }

    fn get(&self, path: &str) -> (String, Vec<u8>) {
        let address = self.0.as_ref().unwrap_or_else(|| panic!("listener")).addr();
        let mut stream = TcpStream::connect_timeout(&address, WAIT)
            .unwrap_or_else(|error| panic!("connect: {error}"));
        stream
            .set_read_timeout(Some(WAIT))
            .unwrap_or_else(|error| panic!("set read timeout: {error}"));
        stream
            .set_write_timeout(Some(WAIT))
            .unwrap_or_else(|error| panic!("set write timeout: {error}"));
        write!(stream, "GET {path} HTTP/1.1\r\nHost: {address}\r\n\r\n")
            .unwrap_or_else(|error| panic!("write {path}: {error}"));
        stream
            .shutdown(std::net::Shutdown::Write)
            .unwrap_or_else(|error| panic!("finish {path} request: {error}"));
        let mut wire = Vec::new();
        stream
            .read_to_end(&mut wire)
            .unwrap_or_else(|error| panic!("read {path}: {error}"));
        let split = wire
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap_or_else(|| panic!("{path}: missing response framing"));
        let content = wire.split_off(split + 4);
        wire.truncate(split);
        let head = String::from_utf8(wire)
            .unwrap_or_else(|error| panic!("{path}: invalid response head: {error}"));
        assert_eq!(head.lines().next(), Some("HTTP/1.1 200 OK"), "{path}");
        assert_eq!(field(&head, "Content-Length"), content.len().to_string());
        (head, content)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(running) = self.0.take() {
            running.shutdown();
        }
    }
}

fn field<'a>(head: &'a str, name: &str) -> &'a str {
    head.lines()
        .filter_map(|line| line.split_once(": "))
        .find_map(|(key, value)| (key == name).then_some(value))
        .unwrap_or_else(|| panic!("{name} missing from {head}"))
}

fn security_fields(head: &str) -> Vec<&str> {
    head.lines()
        .skip(1)
        .filter(|line| !line.starts_with("Content-Type: ") && !line.starts_with("Content-Length: "))
        .collect()
}

#[allow(
    clippy::disallowed_methods,
    reason = "read the repository's static assets for a byte comparison"
)]
fn disk(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(VENDOR_DIR)
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn the_public_license_asset_carries_both_exact_upstream_texts() {
    let listener = Listener::start();
    let (head, body) = listener.get("/assets/licenses.txt");
    assert!(matches!(
        field(&head, "Content-Type"),
        "text/plain" | "text/plain; charset=utf-8"
    ));
    let (index_head, _) = listener.get("/");
    assert_eq!(security_fields(&head), security_fields(&index_head));

    let vendor = String::from_utf8(disk("VENDOR.md"))
        .unwrap_or_else(|error| panic!("vendor attribution: {error}"));
    for (file, package, version, tarball, digest) in [
        (
            "LICENSE-preact.txt",
            "preact",
            "11.2.0",
            "https://registry.npmjs.org/preact/-/preact-11.2.0.tgz",
            "1fe6958409c8c257a70c587a18b6f7f412b179b456630790d30b2ec9a8e4b7d4",
        ),
        (
            "LICENSE-htm.txt",
            "htm",
            "3.1.1",
            "https://registry.npmjs.org/htm/-/htm-3.1.1.tgz",
            "740725f7252e750af735d0028cc534970772f513331e9f68150fede8fb3ce00f",
        ),
    ] {
        let license = disk(file);
        assert_eq!(ae::install::sha256_hex(&license), digest, "{file}");
        assert!(vendor.contains(file) && vendor.contains(digest), "{file}");
        let at = body
            .windows(license.len())
            .position(|bytes| bytes == license)
            .unwrap_or_else(|| panic!("{file}: exact upstream text missing from the HTTP body"));
        let prefix = std::str::from_utf8(&body[..at])
            .unwrap_or_else(|error| panic!("{file}: invalid attribution header: {error}"));
        assert!(prefix.ends_with('\n'), "{file}: missing header line");
        let heading = prefix.trim_end().lines().last().unwrap_or_default();
        let label = heading.split_once(tarball).map_or("", |(shown, _)| shown);
        assert!(
            label.to_ascii_lowercase().contains(package) && label.contains(version),
            "{file}: wrong attribution header: {heading:?}"
        );
    }
}

#[test]
fn all_previous_static_routes_keep_their_own_bytes_and_media_type() {
    let listener = Listener::start();
    for (path, file, media) in [
        ("/", "index.html", "text/html; charset=utf-8"),
        ("/assets/app.css", "app.css", "text/css; charset=utf-8"),
        (
            "/assets/app.mjs",
            "app.mjs",
            "text/javascript; charset=utf-8",
        ),
        (
            "/assets/preact.mjs",
            "preact.mjs",
            "text/javascript; charset=utf-8",
        ),
        (
            "/assets/hooks.mjs",
            "hooks.mjs",
            "text/javascript; charset=utf-8",
        ),
        (
            "/assets/htm.mjs",
            "htm.mjs",
            "text/javascript; charset=utf-8",
        ),
    ] {
        let (head, body) = listener.get(path);
        assert_eq!(field(&head, "Content-Type"), media, "{path}");
        assert_eq!(body, disk(file), "{path} served the wrong source file");
    }
}
