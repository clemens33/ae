#![no_main]
use ae::quota::history::{fold, parse, render};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let rows: Vec<_> = text
        .lines()
        .take(256)
        .filter_map(|s| parse(s).ok())
        .collect();
    for row in &rows {
        let line = render(row);
        // Rendering writes absent optionals as null, so it can grow a valid line.
        if line.len() <= 65_536 {
            assert_eq!(parse(&line).as_ref(), Ok(row));
        }
    }
    let report = std::hint::black_box(fold(rows.clone()));
    assert_eq!(report, fold(rows));
    assert_eq!(report.aligned.len(), report.samples.len());
    assert!(
        report
            .aligned
            .iter()
            .flatten()
            .all(|i| *i < report.workload.len())
    );
    assert!(
        report
            .segments
            .iter()
            .all(|s| s.sample < report.samples.len())
    );
});
