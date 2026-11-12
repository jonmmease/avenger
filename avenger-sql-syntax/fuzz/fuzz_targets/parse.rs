//! Parses arbitrary text as a script and as a module, and checks that each tree holds the text,
//! and that completion at the end of the text runs.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let script = avenger_sql_syntax::parse(text);
    assert_eq!(script.syntax().text().to_string(), text);
    let module = avenger_sql_syntax::parse_module(text);
    assert_eq!(module.syntax().text().to_string(), text);
    avenger_sql_syntax::expected_at(text, (text.len() as u32).into());
});
