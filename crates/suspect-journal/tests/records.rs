//! JSONL record decoding at the public serde boundary.

use suspect_journal::Record;

#[test]
fn fractional_durations_decode_regardless_of_tag_order() {
    for input in [
        r#"{"kind":"run_summary","ts_ms":123,"run_kind":"gen","passed":1,"failed":0,"skipped":0,"duration_ms":1.25}"#,
        r#"{"ts_ms":123,"run_kind":"gen","passed":1,"failed":0,"skipped":0,"duration_ms":1.25,"kind":"run_summary"}"#,
    ] {
        let record: Record = serde_json::from_str(input).expect("valid JSONL record");
        let Record::RunSummary(summary) = record else {
            panic!("wrong record kind");
        };
        assert_eq!(summary.duration_ms, 1.25);
    }
}

#[test]
fn record_metadata_retains_exact_contract_numbers() {
    let input = r#"{"kind":"meta","ts_ms":123,"component":"gen","msg":"bounds","fields":{"maximum":184467440737095516160,"multipleOf":0.12345678901234567890123456789}}"#;
    let record: Record = serde_json::from_str(input).unwrap();
    let Record::Meta(meta) = record else {
        panic!("wrong record kind");
    };
    assert_eq!(meta.fields["maximum"].to_string(), "184467440737095516160");
    assert_eq!(
        meta.fields["multipleOf"].to_string(),
        "0.12345678901234567890123456789"
    );
}

#[test]
fn duplicate_record_fields_are_rejected() {
    for input in [
        r#"{"kind":"meta","kind":"log","ts_ms":123,"component":"gen","msg":"bounds","fields":{}}"#,
        r#"{"kind":"meta","ts_ms":123,"component":"gen","msg":"bounds","fields":{},"ts_ms":456}"#,
    ] {
        assert!(serde_json::from_str::<Record>(input).is_err());
    }
}
