//! The dependency-free JSON codec for halmi's own schema: writer escaping,
//! parser round trips, both baseline envelope forms, and malformed input.

mod common;

use common::raw;
use halmi_core::json::{Json, parse};
use halmi_core::report::{DeltaStatus, FunctionReport, SCHEMA_VERSION, parse_reports};

fn sample() -> FunctionReport {
    let mut r = FunctionReport::from_raw(&raw(10, 2));
    r.reason = "quote \" back\\slash \n newline\ttab \u{1} ctrl".into();
    r.delta_status = Some(DeltaStatus::Worsened);
    r
}

#[test]
fn string_escaping_is_json_correct_both_ways() {
    let r = sample();
    let text = r.to_json_string();

    assert!(text.contains("\\\""), "{text}");
    assert!(text.contains("\\n"), "{text}");
    assert!(text.contains("\\u0001"), "{text}");

    let parsed = parse(&text).unwrap();
    let back = FunctionReport::from_json(&parsed).unwrap();
    assert_eq!(back, r);
}

#[test]
fn round_trip_preserves_every_field() {
    let mut r = FunctionReport::from_raw(&raw(1, 0));
    r.priority = 1.5;
    r.reason = "test".into();
    let text = r.to_json_string();
    for field in [
        "path",
        "name",
        "start_line",
        "end_line",
        "difficulty",
        "effort",
        "volume",
        "estimated_bugs",
        "purity_ratio",
        "mi_vs",
        "mi_original",
        "mi_sei",
        "mi_band",
        "cyclomatic",
        "loc",
        "nesting",
        "priority",
        "reason",
        "potential_volume",
        "language",
    ] {
        assert!(text.contains(&format!("\"{field}\"")), "missing {field}");
    }

    assert!(!text.contains("\"delta\""));
    assert!(!text.contains("\"delta_status\""));
    let back = FunctionReport::from_json(&parse(&text).unwrap()).unwrap();
    assert_eq!(back, r);
}

#[test]
fn numbers_are_finite_and_reparse_to_the_same_value() {
    let r = sample();
    let text = r.to_json_string();
    assert!(!text.contains("NaN") && !text.contains("inf"));
    let back = FunctionReport::from_json(&parse(&text).unwrap()).unwrap();
    assert_eq!(back.volume, r.volume);
    assert_eq!(back.effort.to_bits(), r.effort.to_bits());
}

#[test]
fn parse_reports_accepts_wrapped_and_bare_envelopes() {
    let mut r = FunctionReport::from_raw(&raw(1, 0));
    r.reason = "x".into();
    let one = format!("[{}]", r.to_json_string());
    let reports = parse_reports(&one).unwrap();
    assert_eq!(reports, vec![r.clone()]);
    let wrapped = format!(
        "{{\"schema_version\": {SCHEMA_VERSION}, \"results\": [{}]}}",
        r.to_json_string()
    );
    assert_eq!(parse_reports(&wrapped).unwrap(), vec![r]);
}

#[test]
fn parser_skips_unknown_fields_and_nulls() {
    let text = r#"{"future": {"deep": [1, 2.5, null, true]}, "path": "a.rs", "mi_sei": null, "reason": "ok"}"#;
    let v = parse(text).unwrap();
    assert_eq!(v.get("path"), Some(&Json::from("a.rs")));
    assert!(v.get("mi_sei").unwrap().is_null());
    let future = v.get("future").unwrap();
    assert_eq!(future.as_object().unwrap().len(), 1);
    assert_eq!(future.get("deep").unwrap().as_array().unwrap().len(), 4);
}

#[test]
fn our_codec_agrees_with_an_independent_json_parser() {
    let r = sample();
    let text = r.to_json_string();
    let theirs: serde_json::Value =
        serde_json::from_str(&text).expect("serde_json accepts our output");
    let ours = parse(&text).unwrap();
    let ours_obj = ours.as_object().unwrap();
    for field in [
        "path",
        "name",
        "start_line",
        "end_line",
        "difficulty",
        "effort",
        "volume",
        "estimated_bugs",
        "purity_ratio",
        "mi_vs",
        "mi_original",
        "mi_band",
        "cyclomatic",
        "loc",
        "nesting",
        "priority",
        "reason",
        "delta_status",
    ] {
        let mine = ours_obj.iter().find(|(k, _)| k == field).map(|(_, v)| v);
        let theirs_v = theirs.get(field);
        assert!(mine.is_some(), "our parser lost {field}");
        assert!(theirs_v.is_some(), "serde_json lost {field}");

        match (mine.unwrap(), theirs_v.unwrap()) {
            (Json::Num(a), serde_json::Value::Number(b)) => {
                assert_eq!(a.to_string(), b.to_string(), "{field}");
            }
            (Json::Str(a), serde_json::Value::String(b)) => assert_eq!(a, b, "{field}"),
            (a, b) => panic!("{field}: kind mismatch {a:?} vs {b}"),
        }
    }

    let wrapped = format!("{{\"results\": [{text}]}}");
    assert!(serde_json::from_str::<serde_json::Value>(&wrapped).is_ok());
}

#[test]
fn malformed_input_is_a_structured_error() {
    for bad in [
        "",
        "{",
        "{\"a\":}",
        "[1,",
        "{\"a\": \"unterminated",
        "not json",
        "{\"a\": tru}",
        "{\"a\": 1} trailing",
        "[] [1]",
    ] {
        assert!(parse(bad).is_err(), "should reject {bad:?}");
    }
    assert!(parse_reports("{\"no_results\": 1}").is_err());
}

#[test]
fn low_surrogates_must_use_the_low_range() {
    for bad in [r"\uD800\uE000", r"\uD800\u0041"] {
        let text = format!("\"{bad}\"");
        assert!(parse(&text).is_err(), "should reject invalid pair {text}");
    }
}

#[test]
fn json_numbers_follow_the_grammar_and_stay_finite() {
    for bad in ["01", "-01", "1.", "1e400"] {
        assert!(parse(bad).is_err(), "should reject {bad}");
    }
    for good in ["0", "-0", "0.0", "0e0", "-1.5e-2"] {
        assert!(parse(good).is_ok(), "should accept {good}");
    }
}
