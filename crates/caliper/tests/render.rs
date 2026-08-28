//! Rendering tests: layout invariants, filtering and section selection.

use caliper::render::{render, GroupBy, RenderOpts, Section};
use caliper::report::{Decision, Report, Severity, ThreadRole};

const EXAMPLE: &str = include_str!("../../../examples/report.json");

fn example() -> Report {
    serde_json::from_str(EXAMPLE).expect("shipped example must satisfy the schema")
}

fn opts(width: usize) -> RenderOpts {
    RenderOpts {
        width,
        ..RenderOpts::default()
    }
}

/// Every emitted line must fit the requested width. A line may only overflow if it
/// holds a single unbreakable token (a long path or URL), which wrapping cannot help.
#[test]
fn rendered_lines_respect_the_requested_width() {
    let r = example();
    for width in [72, 80, 100, 110] {
        let out = render(&r, &opts(width));
        for line in out.lines() {
            if line.chars().count() > width {
                assert!(
                    !line.trim_start().contains(' '),
                    "width {width} exceeded by a wrappable line: {line:?}"
                );
            }
        }
    }
}

#[test]
fn filters_drop_the_right_findings() {
    let r = example();

    let strict = RenderOpts {
        max_severity: Severity::Bug,
        sections: vec![Section::Findings],
        ..opts(100)
    };
    let out = render(&r, &strict);
    assert!(out.contains("bug-1") && out.contains("blast-1"));
    assert!(
        !out.contains("perf-1"),
        "PERFORMANCE survived max-severity=bug"
    );
    assert!(!out.contains("nice-1"));

    let confident = RenderOpts {
        min_confidence: 85,
        sections: vec![Section::Findings],
        ..opts(100)
    };
    let out = render(&r, &confident);
    assert!(out.contains("bug-1"), "91%% finding was filtered out");
    assert!(
        !out.contains("[q-1]"),
        "55%% finding survived min-confidence=85"
    );
}

#[test]
fn kept_only_view_shows_nothing_before_triage() {
    let r = example();
    let out = render(
        &r,
        &RenderOpts {
            kept_only: true,
            sections: vec![Section::Findings],
            ..opts(100)
        },
    );
    assert!(
        out.contains("Nothing to report"),
        "untriaged report leaked into --kept"
    );
}

#[test]
fn grouping_by_category_uses_the_tags_present() {
    let out = render(
        &example(),
        &RenderOpts {
            group_by: GroupBy::Category,
            sections: vec![Section::Findings],
            ..opts(100)
        },
    );
    assert!(out.contains("DATA_CORRECTNESS  (2)"));
    assert!(out.contains("SCHEMA_COMPAT  (1)"));
    // A finding with no category still has to appear somewhere.
    assert!(out.contains("UNCATEGORIZED"));
}

#[test]
fn reviewer_wording_and_thread_are_shown() {
    let mut r = example();
    r.findings[0].decision = Some(Decision::Keep);
    r.findings[0].comment = Some("This duplicates rows across partitions.".into());
    r.findings[0].thread = vec![
        entry(
            ThreadRole::Reviewer,
            "Can this be hit with equal-size partitions?",
        ),
        entry(
            ThreadRole::Agent,
            "No — only when a later partition is smaller.",
        ),
    ];

    let out = render(&r, &opts(100));
    assert!(out.contains("This duplicates rows across partitions."));
    assert!(out.contains("Can this be hit"));
    assert!(out.contains("only when a later partition is smaller"));
    assert!(out.contains("keep"));
    assert!(!out.contains("awaiting an answer"), "thread was answered");
}

#[test]
fn unanswered_question_is_flagged_in_the_output() {
    let mut r = example();
    r.findings[0].decision = Some(Decision::FollowUp);
    r.findings[0].thread = vec![entry(ThreadRole::Reviewer, "Is the buffer per-thread?")];
    let out = render(&r, &opts(100));
    assert!(out.contains("awaiting an answer from the review agent"));
}

#[test]
fn sections_are_independently_selectable() {
    let r = example();
    let out = render(
        &r,
        &RenderOpts {
            sections: vec![Section::Explainer],
            ..opts(100)
        },
    );
    assert!(out.contains("WHAT THIS CHANGES"));
    assert!(!out.contains("FINDINGS") && !out.contains("WHAT TO DO"));
}

fn entry(role: ThreadRole, text: &str) -> caliper::report::ThreadEntry {
    caliper::report::ThreadEntry {
        role,
        text: text.to_string(),
    }
}
