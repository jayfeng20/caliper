//! Contract and rendering tests, driven by the shipped example report.

use caliper::report::{Decision, Report, Severity, ThreadRole};

const EXAMPLE: &str = include_str!("../../../examples/report.json");

fn example() -> Report {
    serde_json::from_str(EXAMPLE).expect("shipped example must satisfy the schema")
}

#[test]
fn example_round_trips_through_serde() {
    let once = example();
    let json = serde_json::to_string(&once).unwrap();
    let twice: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(once.findings.len(), twice.findings.len());
    assert_eq!(
        serde_json::to_value(&once).unwrap(),
        serde_json::to_value(&twice).unwrap()
    );
}

#[test]
fn ranked_puts_worst_first_then_best_evidenced() {
    let report = example();
    let ranked = report.ranked();
    assert_eq!(ranked[0].severity, Severity::Critical);
    assert_eq!(ranked.last().unwrap().severity, Severity::Nice);

    // Within a severity, higher confidence comes first.
    let bugs: Vec<u8> = ranked
        .iter()
        .filter(|f| f.severity == Severity::Bug)
        .map(|f| f.confidence)
        .collect();
    assert!(
        bugs.windows(2).all(|w| w[0] >= w[1]),
        "bugs not ranked: {bugs:?}"
    );
}

#[test]
fn severity_ordering_matches_urgency() {
    assert!(Severity::Critical < Severity::Bug);
    assert!(Severity::Bug < Severity::Nice);
    assert!(Severity::Critical.blocking() && Severity::Bug.blocking());
    assert!(!Severity::Style.blocking() && !Severity::Nice.blocking());
}

#[test]
fn severity_parses_case_insensitively_and_rejects_junk() {
    assert_eq!("critical".parse::<Severity>().unwrap(), Severity::Critical);
    assert_eq!(
        "PERFORMANCE".parse::<Severity>().unwrap(),
        Severity::Performance
    );
    let err = "sev1".parse::<Severity>().unwrap_err();
    assert!(
        err.contains("sev1") && err.contains("critical"),
        "unhelpful error: {err}"
    );
}

#[test]
fn kebab_case_variants_are_accepted() {
    // Agents write both `push_back` and `push-back`; neither may break a review.
    let snake: caliper::report::ActionKind = serde_json::from_str("\"push_back\"").unwrap();
    let kebab: caliper::report::ActionKind = serde_json::from_str("\"push-back\"").unwrap();
    assert_eq!(snake, kebab);

    let snake: Decision = serde_json::from_str("\"follow_up\"").unwrap();
    let kebab: Decision = serde_json::from_str("\"follow-up\"").unwrap();
    assert_eq!(snake, kebab);
}

#[test]
fn agents_may_omit_every_reviewer_field() {
    let f: caliper::report::Finding = serde_json::from_str(
        r#"{"id":"a","severity":"BUG","confidence":70,"path":"x.rs","line":1,
            "claim":"c","consequence":"q","agent":"pr-bug-hunter"}"#,
    )
    .unwrap();
    assert!(f.decision.is_none() && f.comment.is_none() && f.thread.is_empty());
    assert_eq!(f.location(), "x.rs:1");
}

#[test]
fn awaiting_agent_only_counts_unanswered_questions() {
    let mut r = example();
    r.findings[0].decision = Some(Decision::FollowUp);
    r.findings[0].thread = vec![entry(ThreadRole::Reviewer, "why is this unsafe?")];
    r.findings[1].decision = Some(Decision::FollowUp);
    r.findings[1].thread = vec![
        entry(ThreadRole::Reviewer, "does the caller retry?"),
        entry(ThreadRole::Agent, "no, it propagates"),
    ];

    let waiting: Vec<&str> = r.awaiting_agent().iter().map(|f| f.id.as_str()).collect();
    assert_eq!(
        waiting,
        vec!["bug-1"],
        "answered questions must not stay pending"
    );
}

#[test]
fn kept_returns_only_keeps_in_rank_order() {
    let mut r = example();
    r.findings[3].decision = Some(Decision::Keep); // perf-1
    r.findings[0].decision = Some(Decision::Keep); // bug-1, CRITICAL
    r.findings[1].decision = Some(Decision::Drop);

    let kept: Vec<&str> = r.kept().iter().map(|f| f.id.as_str()).collect();
    assert_eq!(kept, vec!["bug-1", "perf-1"]);
}

fn entry(role: ThreadRole, text: &str) -> caliper::report::ThreadEntry {
    caliper::report::ThreadEntry {
        role,
        text: text.to_string(),
    }
}

// ------------------------------------------------------- schema evolution
// These four tests are the contract that lets the ipr prompt change without a
// caliper release. Breaking one of them is a breaking change to the pipeline.

#[test]
fn an_unrecognized_severity_degrades_instead_of_failing() {
    let f: caliper::report::Finding = serde_json::from_str(
        r#"{"id":"a","severity":"DATA_LOSS","path":"x.rs","line":1,"claim":"c"}"#,
    )
    .expect("a new severity must not invalidate the finding");
    assert_eq!(f.severity, Severity::Unknown);

    // ...and it must rank where a reviewer will still see it.
    assert!(
        Severity::Unknown > Severity::Bug,
        "unknown must not outrank a real bug"
    );
    assert!(
        Severity::Unknown < Severity::Style,
        "unknown must not be buried in the nits"
    );
}

#[test]
fn unknown_fields_survive_a_round_trip() {
    // A prompt starts emitting something caliper has never heard of. The reviewer's
    // triage pass must not silently delete it.
    let src = r#"{
        "findings": [{
            "id":"a","severity":"BUG","path":"x.rs","line":1,"claim":"c",
            "metrics_delta": {"recall": -0.02}
        }],
        "run_id": "abc123"
    }"#;
    let r: Report = serde_json::from_str(src).unwrap();
    assert_eq!(r.extra["run_id"], "abc123");
    assert_eq!(r.findings[0].extra["metrics_delta"]["recall"], -0.02);

    let back = serde_json::to_value(&r).unwrap();
    assert_eq!(back["run_id"], "abc123");
    assert_eq!(back["findings"][0]["metrics_delta"]["recall"], -0.02);
}

#[test]
fn a_minimal_report_still_parses_and_renders_data() {
    // The floor: only what is needed to put a finding on screen.
    let src = r#"{"findings":[{"severity":"BUG","path":"x.rs","line":7,"claim":"leaks"}]}"#;
    let mut r: Report = serde_json::from_str(src).unwrap();
    r.normalize();

    assert_eq!(r.schema_version, caliper::report::SCHEMA_VERSION);
    assert_eq!(
        r.findings[0].id, "f1",
        "normalize must invent a usable handle"
    );
    assert_eq!(r.findings[0].confidence, 50);
    assert!(r.findings[0].consequence.is_empty());
    assert!(r.pr.head_sha.is_none());
    assert_eq!(r.pr.title, "");
}

#[test]
fn a_future_schema_version_is_readable() {
    let src = r#"{"schema_version":99,"findings":[]}"#;
    let r: Report = serde_json::from_str(src).unwrap();
    assert_eq!(
        r.schema_version, 99,
        "caliper must read it and let validate complain"
    );
}
