//! The caliper report — the JSON contract between review agents and this tool.
//!
//! Agents never build terminal output. They emit one [`Report`] per review run and
//! caliper owns every decision about presentation, ordering and filtering. The
//! reviewer's triage is written back into the *same* document, so a review survives
//! any number of round trips between human and agent.
//!
//! # Evolving this schema
//!
//! The prompts that produce these reports change far more often than caliper does,
//! so the contract is built to bend rather than break. Four rules make that work:
//!
//! 1. **Parsing is permissive; `caliper validate` is strict.** Only the fields
//!    needed to render a finding at all are required (`severity`, `path`, `line`,
//!    `claim`). Everything else defaults, and a thin report renders with gaps
//!    instead of failing. Quality complaints are `validate`'s job.
//! 2. **Unknown fields are preserved, not dropped.** `Report` and `Finding` carry an
//!    `extra` catch-all, so a prompt can start emitting a new field before caliper
//!    knows about it and the value still survives a `review` round trip.
//! 3. **Open sets where the taxonomy is domain-specific.** `category` is a free
//!    string precisely because ML/data-infra concerns keep growing. `severity` stays
//!    closed because it means *urgency*, which is stable, and because ranking needs
//!    a total order — but an unrecognized value degrades to
//!    [`Severity::Unknown`] rather than failing the report.
//! 4. **Renames ship as aliases.** Add `#[serde(alias = "old_name")]` and old
//!    reports keep working; never rename a field bare.
//!
//! Bump [`SCHEMA_VERSION`] only when a field changes *meaning*. Additions and
//! removals are absorbed by the rules above.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Bumped only when an existing field changes meaning — not for additions.
pub const SCHEMA_VERSION: u32 = 1;

/// Findings at or above this confidence are presented as claims; below it they are
/// presented as leads for the reviewer to confirm.
pub const CONFIDENT_THRESHOLD: u8 = 60;

/// Confidence assumed when an agent omits it: enough to show, not enough to trust.
const ASSUMED_CONFIDENCE: u8 = 50;

/// Categories the review agents are asked to prefer. Deliberately *not* enforced —
/// `category` is an open set so a new domain never needs a caliper release. A
/// section appears in the output if and only if some finding carries its tag.
pub const SUGGESTED_CATEGORIES: &[&str] = &[
    "CORRECTNESS",
    "DATA_CORRECTNESS",
    "SCHEMA_COMPAT",
    "NUMERICS",
    "PERF_REGRESSION",
    "RESOURCE",
    "OBSERVABILITY",
    "API_COMPAT",
    "SECURITY",
];

/// Unknown JSON keys, kept so they survive a `review` round trip.
pub type Extra = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub pr: Pr,
    #[serde(default)]
    pub explainer: Option<Explainer>,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub action_list: Vec<Action>,
    /// The synthesizer's merge recommendation, as prose. Deliberately not an enum:
    /// caliper does not model "approve".
    #[serde(default)]
    pub verdict: Option<String>,
    /// Agents the run intentionally skipped, e.g. on a small diff.
    #[serde(default)]
    pub skipped_agents: Vec<String>,

    #[serde(flatten, default)]
    pub extra: Extra,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Report {
    /// Fills in what agents commonly omit. Called once after loading so the rest of
    /// the code never has to reason about blank ids.
    pub fn normalize(&mut self) {
        for (i, f) in self.findings.iter_mut().enumerate() {
            if f.id.trim().is_empty() {
                f.id = format!("f{}", i + 1);
            }
        }
    }

    /// Findings ranked the way a reviewer wants to read them: worst first, and within
    /// a severity the best-evidenced first.
    pub fn ranked(&self) -> Vec<&Finding> {
        let mut out: Vec<&Finding> = self.findings.iter().collect();
        out.sort_by(|a, b| {
            a.severity
                .cmp(&b.severity)
                .then(b.confidence.cmp(&a.confidence))
                .then(a.path.cmp(&b.path))
                .then(a.line.cmp(&b.line))
        });
        out
    }

    /// Findings the reviewer marked for submission.
    pub fn kept(&self) -> Vec<&Finding> {
        self.ranked()
            .into_iter()
            .filter(|f| f.decision == Some(Decision::Keep))
            .collect()
    }

    /// Findings waiting on an answer from the calling agent. While this is non-empty
    /// the review is unfinished — the agent should answer, then relaunch caliper.
    pub fn awaiting_agent(&self) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| {
                f.decision == Some(Decision::FollowUp)
                    && f.thread
                        .last()
                        .is_some_and(|t| t.role == ThreadRole::Reviewer)
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Pr {
    #[serde(default)]
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub base_ref: String,
    #[serde(default)]
    pub head_ref: String,
    /// Head commit OID (`gh pr view --json headRefOid`). Optional to parse, but
    /// inline comments cannot be anchored without it, so `validate` warns when it is
    /// missing.
    #[serde(default)]
    pub head_sha: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub additions: u32,
    #[serde(default)]
    pub deletions: u32,
    #[serde(default)]
    pub changed_files: u32,
    #[serde(default)]
    pub ci: Option<CiState>,

    #[serde(flatten, default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CiState {
    Passing,
    Failing,
    Pending,
    Unknown,
}

/// How urgent a finding is. Declaration order *is* the ranking, so the derived `Ord`
/// is what sorts the output.
///
/// This set is closed because severity means urgency, which does not vary by domain
/// — domain lives in the open-ended `category` field instead. An unrecognized value
/// still parses, as [`Severity::Unknown`], so a prompt change can never invalidate a
/// whole report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, JsonSchema)]
pub enum Severity {
    Critical,
    Bug,
    /// A severity this caliper does not know. Ranked here — above the cosmetic
    /// levels, below the confirmed ones — so it is impossible to miss but does not
    /// outrank a real defect. `caliper validate` names the offending values.
    Unknown,
    Performance,
    Style,
    Testing,
    Question,
    Nice,
}

impl Severity {
    /// The recognized severities, in ranking order. `Unknown` is excluded: it is a
    /// parse fallback, not something an agent should emit.
    pub const ALL: &'static [Severity] = &[
        Severity::Critical,
        Severity::Bug,
        Severity::Performance,
        Severity::Style,
        Severity::Testing,
        Severity::Question,
        Severity::Nice,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Severity::Critical => "CRITICAL",
            Severity::Bug => "BUG",
            Severity::Unknown => "UNKNOWN",
            Severity::Performance => "PERFORMANCE",
            Severity::Style => "STYLE",
            Severity::Testing => "TESTING",
            Severity::Question => "QUESTION",
            Severity::Nice => "NICE",
        }
    }

    /// True for severities that should block a merge until resolved.
    pub fn blocking(self) -> bool {
        matches!(self, Severity::Critical | Severity::Bug)
    }

    /// Recognized spelling, or `None`. Case-insensitive.
    pub fn parse_known(s: &str) -> Option<Severity> {
        Severity::ALL
            .iter()
            .copied()
            .find(|sev| sev.label().eq_ignore_ascii_case(s))
    }
}

impl std::str::FromStr for Severity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Severity::parse_known(s).ok_or_else(|| {
            let names: Vec<_> = Severity::ALL
                .iter()
                .map(|s| s.label().to_lowercase())
                .collect();
            format!(
                "unknown severity `{s}` (expected one of: {})",
                names.join(", ")
            )
        })
    }
}

impl Serialize for Severity {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.label())
    }
}

/// Falls back to [`Severity::Unknown`] instead of erroring, so one unrecognized
/// value cannot cost the reviewer the entire report.
impl<'de> Deserialize<'de> for Severity {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(de)?;
        Ok(Severity::parse_known(&raw).unwrap_or(Severity::Unknown))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Finding {
    /// Stable within a run; the handle the reviewer uses to keep, drop or cite it.
    /// Filled in by [`Report::normalize`] when an agent omits it.
    #[serde(default)]
    pub id: String,
    pub severity: Severity,
    #[serde(default)]
    pub category: Option<String>,
    /// 0-100. See [`CONFIDENT_THRESHOLD`].
    #[serde(default = "default_confidence")]
    pub confidence: u8,
    /// Repo-relative path in the post-change tree.
    pub path: String,
    pub line: u32,
    #[serde(default)]
    pub end_line: Option<u32>,
    /// One sentence, no hedging.
    pub claim: String,
    /// The input, state or sequence that makes it go wrong — or, for non-defects,
    /// the specific improvement and why it is better.
    #[serde(default)]
    pub consequence: String,
    #[serde(default)]
    pub suggestion: Option<String>,
    /// Which review agent produced this.
    #[serde(default)]
    pub agent: String,
    /// Other agents that independently found the same thing — a confidence signal
    /// worth surfacing to the reviewer.
    #[serde(default)]
    pub corroborated_by: Vec<String>,

    // --- reviewer state. Agents leave these empty; `caliper review` fills them in
    // and writes the report back out, so the same document survives round trips.
    #[serde(default)]
    pub decision: Option<Decision>,
    /// The reviewer's post-ready wording. Overrides `claim`/`consequence` when the
    /// finding is submitted.
    #[serde(default)]
    pub comment: Option<String>,
    /// Follow-up conversation about this finding, oldest first.
    #[serde(default)]
    pub thread: Vec<ThreadEntry>,

    #[serde(flatten, default)]
    pub extra: Extra,
}

fn default_confidence() -> u8 {
    ASSUMED_CONFIDENCE
}

impl Finding {
    pub fn is_confident(&self) -> bool {
        self.confidence >= CONFIDENT_THRESHOLD
    }

    /// `path:line` or `path:line-end_line`.
    pub fn location(&self) -> String {
        match self.end_line {
            Some(end) if end > self.line => format!("{}:{}-{}", self.path, self.line, end),
            _ => format!("{}:{}", self.path, self.line),
        }
    }
}

/// What the reviewer decided about a finding. `None` means untriaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Keep for submission.
    Keep,
    /// Not a real problem, or not worth the author's time.
    Drop,
    /// The reviewer asked something; the calling agent owes an answer before this
    /// finding can be decided.
    #[serde(alias = "follow-up")]
    FollowUp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ThreadEntry {
    pub role: ThreadRole,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadRole {
    Reviewer,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Action {
    pub text: String,
    #[serde(default)]
    pub kind: Option<ActionKind>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Blocker,
    Verify,
    /// Accepts `push_back` or `push-back`; agents produce both.
    #[serde(alias = "push-back")]
    PushBack,
    Ask,
}

impl ActionKind {
    pub fn label(self) -> &'static str {
        match self {
            ActionKind::Blocker => "blocker",
            ActionKind::Verify => "verify",
            ActionKind::PushBack => "push back",
            ActionKind::Ask => "ask author",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Explainer {
    /// Markdown prose: the problem this PR solves and the design chosen.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub design_notes: Option<String>,
    #[serde(default)]
    pub files: Vec<FileNote>,

    #[serde(flatten, default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileNote {
    pub path: String,
    /// How this file serves the design.
    #[serde(default)]
    pub role: String,
}
