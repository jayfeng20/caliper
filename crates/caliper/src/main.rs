use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use caliper::render::{self, GroupBy, RenderOpts, Section};
use caliper::report::{Report, Severity, SCHEMA_VERSION, SUGGESTED_CATEGORIES};

#[derive(Parser)]
#[command(
    name = "caliper",
    version,
    about = "Structured terminal review of agent-generated PR findings",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render a report to the terminal.
    Render {
        #[command(flatten)]
        input: Input,
        #[command(flatten)]
        filter: Filter,

        /// Which sections to print, in the order given. Defaults to all.
        #[arg(long, value_enum, value_delimiter = ',')]
        section: Vec<Section>,

        /// Group findings by severity (worst first) or by category.
        #[arg(long, value_enum, default_value = "severity")]
        group_by: GroupBy,

        /// Show only findings marked `keep` — what you are about to post.
        #[arg(long)]
        kept: bool,

        /// Wrap width. Defaults to the terminal width, clamped to a readable range.
        #[arg(long)]
        width: Option<usize>,

        #[arg(long, conflicts_with = "color", help = "Disable colour")]
        no_color: bool,
        #[arg(long, help = "Force colour even when stdout is not a terminal")]
        color: bool,
    },

    /// Check that a report satisfies the contract. Exits non-zero if it does not.
    Validate {
        #[command(flatten)]
        input: Input,
    },

    /// Print the JSON Schema for a report, for agents to emit against.
    Schema,
}

#[derive(Args)]
struct Input {
    /// Path to a report JSON file, or `-` for stdin.
    #[arg(default_value = "-")]
    report: PathBuf,
}

#[derive(Args)]
struct Filter {
    /// Hide findings below this confidence (0-100).
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=100))]
    min_confidence: u8,

    /// Hide findings less severe than this. One of: critical, bug, performance,
    /// style, testing, question, nice.
    #[arg(long, default_value = "nice")]
    max_severity: Severity,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Render {
            input,
            filter,
            section,
            group_by,
            kept,
            width,
            no_color,
            color,
        } => {
            let report = load(&input.report)?;
            let opts = RenderOpts {
                sections: if section.is_empty() {
                    Section::ALL.to_vec()
                } else {
                    section
                },
                min_confidence: filter.min_confidence,
                max_severity: filter.max_severity,
                group_by,
                width: width.unwrap_or_else(render::detect_width),
                color: color || (!no_color && std::io::stdout().is_terminal()),
                kept_only: kept,
            };
            print!("{}", render::render(&report, &opts));
            Ok(())
        }

        Command::Validate { input } => {
            let (report, raw) = load_with_source(&input.report)?;
            let warnings = lint(&report, &raw);
            let mut err = std::io::stderr();
            for w in &warnings {
                writeln!(err, "warning: {w}")?;
            }
            println!(
                "ok: schema v{}, {} finding(s), {} kept",
                report.schema_version,
                report.findings.len(),
                report.kept().len()
            );
            Ok(())
        }

        Command::Schema => {
            let schema = schemars::schema_for!(Report);
            println!("{}", serde_json::to_string_pretty(&schema)?);
            Ok(())
        }
    }
}

fn load(path: &Path) -> Result<Report> {
    load_with_source(path).map(|(r, _)| r)
}

fn load_with_source(path: &Path) -> Result<(Report, String)> {
    let raw = if path == Path::new("-") {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading report from stdin")?;
        buf
    } else {
        std::fs::read_to_string(path)
            .with_context(|| format!("reading report from {}", path.display()))?
    };

    let mut report: Report = serde_json::from_str(&raw)
        .context("report does not match the caliper schema (see `caliper schema`)")?;
    report.normalize();
    Ok((report, raw))
}

/// Contract violations that are not type errors. Parsing is deliberately permissive
/// so a thin report still renders; this is where the report gets judged.
fn lint(report: &Report, raw: &str) -> Vec<String> {
    let mut out = Vec::new();

    if report.schema_version > SCHEMA_VERSION {
        out.push(format!(
            "report declares schema v{} but this caliper speaks v{SCHEMA_VERSION}; \
             newer fields will be preserved but ignored",
            report.schema_version
        ));
    }
    match &report.pr.head_sha {
        None => out.push(
            "pr.head_sha is missing; findings cannot be submitted as inline comments without it"
                .into(),
        ),
        Some(sha) if sha.len() < 7 => {
            out.push(format!("pr.head_sha `{sha}` looks truncated"));
        }
        Some(_) => {}
    }

    // A severity caliper does not recognize parses as UNKNOWN, which loses the
    // original spelling — recover it from the source so the message is actionable.
    for name in unknown_severities(raw) {
        out.push(format!(
            "severity `{name}` is not recognized and will render as UNKNOWN; \
             use one of the known severities or add the variant to caliper"
        ));
    }

    let mut ids = std::collections::HashSet::new();
    for f in &report.findings {
        if !ids.insert(&f.id) {
            out.push(format!("duplicate finding id `{}`", f.id));
        }
        if f.confidence > 100 {
            out.push(format!(
                "finding `{}` has confidence {} > 100",
                f.id, f.confidence
            ));
        }
        if f.consequence.trim().is_empty() {
            out.push(format!(
                "finding `{}` has no consequence; per the contract it should be dropped",
                f.id
            ));
        }
        if f.agent.trim().is_empty() {
            out.push(format!(
                "finding `{}` does not say which agent produced it",
                f.id
            ));
        }
        if let Some(c) = &f.category {
            if !SUGGESTED_CATEGORIES.contains(&c.as_str()) {
                out.push(format!(
                    "finding `{}` uses non-standard category `{c}` (allowed, but it gets its own section)",
                    f.id
                ));
            }
        }
        if f.end_line.is_some_and(|e| e < f.line) {
            out.push(format!("finding `{}` has end_line before line", f.id));
        }
        if !f.extra.is_empty() {
            let keys: Vec<&str> = f.extra.keys().map(String::as_str).collect();
            out.push(format!(
                "finding `{}` carries unknown field(s) {}; preserved but not rendered",
                f.id,
                keys.join(", ")
            ));
        }
    }
    out
}

/// Severity strings in the source JSON that caliper does not recognize.
fn unknown_severities(raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let mut seen = std::collections::BTreeSet::new();
    for f in value
        .get("findings")
        .and_then(|f| f.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(s) = f.get("severity").and_then(|s| s.as_str()) {
            if Severity::parse_known(s).is_none() {
                seen.insert(s.to_string());
            }
        }
    }
    seen.into_iter().collect()
}
