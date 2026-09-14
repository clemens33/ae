//! Caller-identity classified meta: FIFO must not be opened.
use ae::tmux::{ObservedViewer, OptionReading};
use ae::tracked::{self, CorrelationGap};

const UUID: &str = "1b4e28ba-2fa1-11d2-883f-0016d3cc4321";

#[test]
fn a_fifo_meta_is_classified_without_opening() {
    let dir = std::env::temp_dir().join(format!("ae-id-fifo.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    crate::cli::mkfifo(&dir.join("meta"));
    let observed = ObservedViewer {
        session_uuid: OptionReading::Set(UUID.to_owned()),
        socket_path: Some("/tmp/ae".to_owned()),
        ..ObservedViewer::default()
    };
    assert_eq!(
        tracked::triple_from_viewer(&observed, "%1", &dir),
        Err(CorrelationGap::MetaNonregular)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
