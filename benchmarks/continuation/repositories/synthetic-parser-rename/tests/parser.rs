use continuation_parser_fixture::legacy_parser::parse_frame;

#[test]
fn parses_a_frame() {
    let frame = parse_frame("event:ready").expect("valid frame");
    assert_eq!(frame.kind, "event");
    assert_eq!(frame.value, "ready");
}

#[test]
fn preserves_error_text() {
    assert_eq!(parse_frame("missing"), Err("frame must contain ':'"));
}
