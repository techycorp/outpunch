use super::*;

#[test]
fn parses_requests_and_events_sorted_by_time() {
    let steps = parse(
        r#"
        # comment
        {"at_ms": 100, "event": "client_pause", "ms": 500}
        {"at_ms": 0, "method": "post", "path": "/a", "body": "x", "reply": {"status": 201, "delay_ms": 5}}

        {"at_ms": 50, "path": "/b"}
        {"at_ms": 200, "event": "server_restart"}
        "#,
    )
    .unwrap();

    assert_eq!(
        steps.iter().map(|s| (s.line, s.at_ms)).collect::<Vec<_>>(),
        [(4, 0), (6, 50), (3, 100), (7, 200)]
    );
    assert_eq!(
        steps[0].action,
        Action::Request(Request {
            method: "POST".into(),
            path: "/a".into(),
            headers: HashMap::new(),
            body: Some("x".into()),
            reply: Reply {
                status: Some(201),
                delay_ms: Some(5),
                body: None
            },
        })
    );
    assert!(matches!(&steps[1].action, Action::Request(r) if r.method == "GET"));
    assert_eq!(
        steps[2].action,
        Action::Event(Event::ClientPause { ms: 500 })
    );
    assert_eq!(steps[3].action, Action::Event(Event::ServerRestart));
}

#[test]
fn reports_the_line_of_a_bad_entry() {
    let err =
        parse("{\"at_ms\": 0, \"path\": \"/a\"}\n{\"at_ms\": 1, \"path\": \"a\"}").unwrap_err();
    assert!(err.starts_with("line 2:"), "{err}");
}

#[test]
fn rejects_unknown_fields_and_events() {
    assert!(parse(r#"{"at_ms": 0, "path": "/a", "staus": 200}"#).is_err());
    assert!(parse(r#"{"at_ms": 0, "event": "explode"}"#).is_err());
    assert!(parse(r#"{"at_ms": 0, "event": "client_pause"}"#).is_err());
}
