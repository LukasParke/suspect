//! The Bun worker's public JSONL protocol must survive exact-number support.

use suspect_rules::protocol::WorkerFrame;

#[test]
fn worker_completion_accepts_fractional_timing_with_any_tag_order() {
    for input in [
        r#"{"t":"done","run_id":4,"ms":0.125,"findings":2}"#,
        r#"{"run_id":4,"ms":0.125,"findings":2,"t":"done"}"#,
    ] {
        let frame: WorkerFrame = serde_json::from_str(input).expect("valid worker frame");
        let WorkerFrame::Done(done) = frame else {
            panic!("wrong frame kind");
        };
        assert_eq!(done.ms, 0.125);
        assert_eq!(done.run_id, 4);
        assert_eq!(done.findings, 2);
    }
}

#[test]
fn worker_frames_reject_duplicate_tags_and_fields() {
    for input in [
        r#"{"t":"done","t":"pong","run_id":4,"ms":0.125,"findings":2}"#,
        r#"{"t":"done","run_id":4,"run_id":5,"ms":0.125,"findings":2}"#,
    ] {
        assert!(serde_json::from_str::<WorkerFrame>(input).is_err());
    }
}

#[test]
fn worker_fixes_retain_exact_numeric_schema_constraints() {
    let input = r#"{"t":"finding","run_id":4,"rule_id":"bounds","pointer":"/components/schemas/Amount","message":"bound","fix":{"maximum":184467440737095516160}}"#;
    let frame: WorkerFrame = serde_json::from_str(input).unwrap();
    let WorkerFrame::Finding(finding) = frame else {
        panic!("wrong frame kind");
    };
    assert_eq!(
        finding.fix.unwrap()["maximum"].to_string(),
        "184467440737095516160"
    );
}
