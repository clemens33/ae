#![no_main]

use libfuzzer_sys::fuzz_target;

// R15's request-id grammar over hostile ledger bytes: the whole input, lossy,
// as one candidate id, plus newline-split as positioned refs through the
// newest-first 16-cap selection. Positions are synthetic — ledger order is a
// caller fact this instrument cannot observe.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::tracked::is_request_id(&text));
    let refs: Vec<ae::sanitize::LedgerRef<'_>> = data
        .split(|b| *b == b'\n')
        .enumerate()
        .map(|(position, bytes)| ae::sanitize::LedgerRef { position, bytes })
        .collect();
    let _ = std::hint::black_box(ae::sanitize::select_ids(&refs));
});
