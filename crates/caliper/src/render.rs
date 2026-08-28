//! Static, non-interactive rendering of a `Report` to a terminal.
//!
//! This is the pipe-safe half of caliper: deterministic output, no cursor control,
//! colour only when the destination supports it.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use owo_colors::{OwoColorize, Style};

use crate::report::{ActionKind, CiState, Decision, Finding, Report, Severity, ThreadRole};

const MIN_WIDTH: usize = 60;
const MAX_WIDTH: usize = 110;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Section {
    Header,
    Explainer,
    Findings,
    Actions,
}

impl Section {
    pub const ALL: &'static [Section] = &[
        Section::Header,
        Section::Explainer,
        Section::Findings,
        Section::Actions,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GroupBy {
    Severity,
    Category,
}

#[derive(Debug, Clone)]
pub struct RenderOpts {
    pub sections: Vec<Section>,
    pub min_confidence: u8,
    pub max_severity: Severity,
    pub group_by: GroupBy,
    pub width: usize,
    pub color: bool,
    /// Show only findings the reviewer marked `keep` — the "what I am about to post" view.
    pub kept_only: bool,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            sections: Section::ALL.to_vec(),
            min_confidence: 0,
            max_severity: Severity::Nice,
            group_by: GroupBy::Severity,
            width: 100,
            color: false,
            kept_only: false,
        }
    }
}

/// Terminal width clamped to a readable range, or `MAX_WIDTH` when not a terminal.
pub fn detect_width() -> usize {
    terminal_size::terminal_size()
        .map_or(MAX_WIDTH, |(w, _)| usize::from(w.0))
        .clamp(MIN_WIDTH, MAX_WIDTH)
}

pub fn render(report: &Report, opts: &RenderOpts) -> String {
    let p = Paint { on: opts.color };
    let mut out = String::new();

    if opts.sections.contains(&Section::Header) {
        header(&mut out, report, opts, &p);
    }
    if opts.sections.contains(&Section::Explainer) {
        explainer(&mut out, report, opts, &p);
    }
    if opts.sections.contains(&Section::Findings) {
        findings(&mut out, report, opts, &p);
    }
    if opts.sections.contains(&Section::Actions) {
        actions(&mut out, report, opts, &p);
    }
    out
}

// ---------------------------------------------------------------- sections

fn header(out: &mut String, report: &Report, opts: &RenderOpts, p: &Paint) {
    let pr = &report.pr;
    let tag = format!("PR #{} ", pr.number);
    let title = wrap(&pr.title, opts.width, &" ".repeat(tag.len()));
    let _ = writeln!(
        out,
        "\n{}{}",
        p.s(&tag, Style::new().bold().cyan()),
        p.s(&reindent_first(&title, tag.len(), ""), Style::new().bold()),
    );

    let mut facts = vec![
        format!("@{}", pr.author),
        format!("{} \u{2190} {}", pr.base_ref, pr.head_ref),
        format!(
            "+{} \u{2212}{} in {} file{}",
            pr.additions,
            pr.deletions,
            pr.changed_files,
            if pr.changed_files == 1 { "" } else { "s" }
        ),
    ];
    if pr.draft {
        facts.push("draft".to_string());
    }
    let _ = writeln!(
        out,
        "{}",
        p.s(
            &wrap(&facts.join(" \u{b7} "), opts.width, ""),
            Style::new().dimmed()
        )
    );

    if let Some(ci) = pr.ci {
        let (text, style) = match ci {
            CiState::Passing => ("CI passing", Style::new().green()),
            CiState::Failing => ("CI failing", Style::new().red().bold()),
            CiState::Pending => ("CI pending", Style::new().yellow()),
            CiState::Unknown => ("CI unknown", Style::new().dimmed()),
        };
        let _ = writeln!(out, "{}", p.s(text, style));
    }
    let _ = writeln!(out, "{}", p.s(&pr.url, Style::new().dimmed().underline()));

    if !report.skipped_agents.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            p.s(
                &format!("skipped: {}", report.skipped_agents.join(", ")),
                Style::new().dimmed()
            )
        );
    }
    tally(out, report, opts, p);
}

/// One line the reviewer can read in a second: how much is here, and how bad.
fn tally(out: &mut String, report: &Report, opts: &RenderOpts, p: &Paint) {
    let kept = visible(report, opts);
    if kept.is_empty() {
        return;
    }
    let mut counts: BTreeMap<Severity, usize> = BTreeMap::new();
    for f in &kept {
        *counts.entry(f.severity).or_default() += 1;
    }
    let parts: Vec<(usize, String)> = counts
        .iter()
        .map(|(sev, n)| {
            let text = format!("{} {n}", sev.label().to_lowercase());
            (text.chars().count(), p.s(&text, sev_style(*sev)))
        })
        .collect();

    let _ = writeln!(
        out,
        "\n{}",
        join_wrapped(
            &parts,
            " \u{b7} ",
            &p.s(" \u{b7} ", Style::new().dimmed()),
            opts.width
        )
    );

    let leads = kept.iter().filter(|f| !f.is_confident()).count();
    if leads > 0 {
        let note = format!("{leads} below the confidence bar \u{2014} treat as leads, not claims");
        let _ = writeln!(
            out,
            "{}",
            p.s(&wrap(&note, opts.width, ""), Style::new().dimmed())
        );
    }
}

/// Joins pre-styled parts without letting ANSI codes confuse the wrap width: the
/// visible length is tracked separately from the emitted string.
fn join_wrapped(parts: &[(usize, String)], sep: &str, sep_styled: &str, width: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0;

    for (vis_len, styled) in parts {
        if cur.is_empty() {
            cur.push_str(styled);
            cur_len = *vis_len;
            continue;
        }
        if cur_len + sep.chars().count() + vis_len > width {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(styled);
            cur_len = *vis_len;
        } else {
            cur.push_str(sep_styled);
            cur.push_str(styled);
            cur_len += sep.chars().count() + vis_len;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines.join("\n")
}

fn explainer(out: &mut String, report: &Report, opts: &RenderOpts, p: &Paint) {
    let Some(ex) = &report.explainer else { return };
    rule(out, "WHAT THIS CHANGES", opts, p);

    let _ = writeln!(out, "{}", wrap(&ex.summary, opts.width, ""));
    if let Some(notes) = &ex.design_notes {
        let _ = writeln!(
            out,
            "\n{}\n{}",
            p.s("Design", Style::new().bold()),
            wrap(notes, opts.width, "")
        );
    }
    if ex.files.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n{}", p.s("Files", Style::new().bold()));
    let pad = ex.files.iter().map(|f| f.path.len()).max().unwrap_or(0);
    for f in &ex.files {
        let label = format!("  {:pad$}  ", f.path);
        let _ = writeln!(
            out,
            "{}{}",
            p.s(&label, Style::new().cyan()),
            wrap(&f.role, opts.width, &" ".repeat(label.len())).trim_start()
        );
    }
}

fn findings(out: &mut String, report: &Report, opts: &RenderOpts, p: &Paint) {
    rule(out, "FINDINGS", opts, p);
    let kept = visible(report, opts);
    if kept.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            p.s(
                "  Nothing to report at this threshold.",
                Style::new().dimmed()
            )
        );
        return;
    }
    for (group, items) in group(&kept, opts.group_by) {
        let _ = writeln!(
            out,
            "\n{}",
            p.s(
                &format!("{group}  ({})", items.len()),
                Style::new().bold().underline()
            )
        );
        for f in items {
            finding(out, f, opts, p);
        }
    }
}

fn finding(out: &mut String, f: &Finding, opts: &RenderOpts, p: &Paint) {
    let indent = "    ";
    let mut meta = vec![format!("{}%", f.confidence)];
    if !f.corroborated_by.is_empty() {
        meta.push(format!("\u{d7}{} agents", f.corroborated_by.len() + 1));
    }
    if !f.is_confident() {
        meta.push("lead".to_string());
    }
    if let Some(d) = f.decision {
        meta.push(
            match d {
                Decision::Keep => "keep",
                Decision::Drop => "dropped",
                Decision::FollowUp => "follow-up",
            }
            .to_string(),
        );
    }

    let _ = writeln!(
        out,
        "\n  {} {}  {}",
        p.s(f.severity.label(), sev_style(f.severity).bold()),
        p.s(&f.location(), Style::new().cyan()),
        p.s(&meta.join(" \u{b7} "), Style::new().dimmed()),
    );
    let _ = writeln!(out, "{}", wrap(&f.claim, opts.width, indent));
    let _ = writeln!(
        out,
        "{}",
        wrap_hanging(
            &format!("\u{2192} {}", f.consequence),
            opts.width,
            indent,
            2
        )
    );
    if let Some(c) = &f.comment {
        let _ = writeln!(
            out,
            "{}",
            p.s(
                &wrap_hanging(&format!("\u{270e} {c}"), opts.width, indent, 2),
                Style::new().bold()
            )
        );
    }
    if let Some(s) = &f.suggestion {
        let _ = writeln!(
            out,
            "{}",
            p.s(
                &wrap_hanging(&format!("\u{21b3} {s}"), opts.width, indent, 2),
                Style::new().green()
            )
        );
    }

    let mut trailer: Vec<String> = Vec::new();
    if let Some(c) = &f.category {
        trailer.push(c.clone());
    }
    trailer.push(f.agent.clone());
    trailer.extend(f.corroborated_by.iter().cloned());
    let _ = writeln!(
        out,
        "{}",
        p.s(
            &wrap(
                &format!("{}  [{}]", trailer.join(" \u{b7} "), f.id),
                opts.width,
                indent
            ),
            Style::new().dimmed()
        )
    );
    thread(out, f, opts, p);
}

fn thread(out: &mut String, f: &Finding, opts: &RenderOpts, p: &Paint) {
    let indent = "      ";
    for entry in &f.thread {
        let (tag, style) = match entry.role {
            ThreadRole::Reviewer => ("you", Style::new().cyan().bold()),
            ThreadRole::Agent => ("agent", Style::new().magenta().bold()),
        };
        let body = wrap(&entry.text, opts.width, &format!("{indent}       "));
        let _ = writeln!(
            out,
            "{indent}{} {}",
            p.s(&format!("{tag:>5}"), style),
            body.trim_start()
        );
    }
    if f.decision == Some(Decision::FollowUp)
        && f.thread
            .last()
            .is_some_and(|t| t.role == ThreadRole::Reviewer)
    {
        let _ = writeln!(
            out,
            "{indent}{}",
            p.s(
                "awaiting an answer from the review agent",
                Style::new().yellow()
            )
        );
    }
}

fn actions(out: &mut String, report: &Report, opts: &RenderOpts, p: &Paint) {
    if report.action_list.is_empty() && report.verdict.is_none() {
        return;
    }
    rule(out, "WHAT TO DO", opts, p);

    for (i, a) in report.action_list.iter().enumerate() {
        let bullet = format!("  {}.", i + 1);
        let tag = a
            .kind
            .map_or_else(String::new, |k| format!("[{}] ", k.label()));
        let style = match a.kind {
            Some(ActionKind::Blocker) => Style::new().red().bold(),
            Some(ActionKind::PushBack) => Style::new().yellow(),
            Some(ActionKind::Ask) => Style::new().cyan(),
            _ => Style::new().dimmed(),
        };
        // Wrap against a blank prefix of the real width, then splice the styled
        // prefix over it, so line 1 obeys `width` like every other line.
        let prefix_len = bullet.len() + 1 + tag.len();
        let body = wrap(&a.text, opts.width, &" ".repeat(prefix_len));
        let body = reindent_first(&body, prefix_len, &" ".repeat(bullet.len() + 1));
        let _ = writeln!(
            out,
            "{} {}{}",
            p.s(&bullet, Style::new().bold()),
            p.s(&tag, style),
            body
        );
    }
    if let Some(v) = &report.verdict {
        let _ = writeln!(
            out,
            "\n{} {}",
            p.s("Verdict", Style::new().bold()),
            wrap(v, opts.width, "        ").trim_start()
        );
    }
}

// ---------------------------------------------------------------- helpers

fn visible<'a>(report: &'a Report, opts: &RenderOpts) -> Vec<&'a Finding> {
    report
        .ranked()
        .into_iter()
        .filter(|f| f.confidence >= opts.min_confidence && f.severity <= opts.max_severity)
        .filter(|f| !opts.kept_only || f.decision == Some(Decision::Keep))
        .collect()
}

/// Groups preserve the ranked order of `visible`, so the first group is the worst.
fn group<'a>(kept: &[&'a Finding], by: GroupBy) -> Vec<(String, Vec<&'a Finding>)> {
    let mut order: Vec<String> = Vec::new();
    let mut buckets: BTreeMap<String, Vec<&'a Finding>> = BTreeMap::new();
    for f in kept {
        let key = match by {
            GroupBy::Severity => f.severity.label().to_string(),
            GroupBy::Category => f
                .category
                .clone()
                .unwrap_or_else(|| "UNCATEGORIZED".to_string()),
        };
        if !buckets.contains_key(&key) {
            order.push(key.clone());
        }
        buckets.entry(key).or_default().push(f);
    }
    order
        .into_iter()
        .filter_map(|k| buckets.remove(&k).map(|v| (k, v)))
        .collect()
}

fn sev_style(sev: Severity) -> Style {
    match sev {
        Severity::Critical => Style::new().bright_red(),
        Severity::Bug => Style::new().red(),
        Severity::Unknown => Style::new().red().italic(),
        Severity::Performance => Style::new().yellow(),
        Severity::Style => Style::new().blue(),
        Severity::Testing => Style::new().magenta(),
        Severity::Question => Style::new().cyan(),
        Severity::Nice => Style::new().green(),
    }
}

fn rule(out: &mut String, title: &str, opts: &RenderOpts, p: &Paint) {
    let bar = "\u{2500}".repeat(opts.width.saturating_sub(title.len() + 4));
    let _ = writeln!(
        out,
        "\n{}",
        p.s(
            &format!("\u{2500}\u{2500} {title} {bar}"),
            Style::new().dimmed()
        )
    );
}

/// Like `wrap`, but continuation lines get `hang` extra columns so they line up
/// under the text rather than under the leading marker.
fn wrap_hanging(text: &str, width: usize, indent: &str, hang: usize) -> String {
    let cont = format!("{indent}{}", " ".repeat(hang));
    let opts = textwrap::Options::new(width)
        .initial_indent(indent)
        .subsequent_indent(&cont);
    textwrap::fill(text, &opts)
}

/// Drops `prefix_len` leading spaces from the first line and re-indents the rest.
fn reindent_first(body: &str, prefix_len: usize, cont_indent: &str) -> String {
    let mut lines = body.lines();
    let first = lines.next().unwrap_or_default();
    let mut out = first[prefix_len.min(first.len())..].to_string();
    for line in lines {
        out.push('\n');
        out.push_str(cont_indent);
        out.push_str(line.trim_start());
    }
    out
}

fn wrap(text: &str, width: usize, indent: &str) -> String {
    let opts = textwrap::Options::new(width)
        .initial_indent(indent)
        .subsequent_indent(indent);
    text.split('\n')
        .map(|para| {
            if para.trim().is_empty() {
                String::new()
            } else {
                textwrap::fill(para, &opts)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Colour gate. Every styled write goes through this so `--no-color` and non-tty
/// output are handled in exactly one place.
struct Paint {
    on: bool,
}

impl Paint {
    fn s(&self, text: &str, style: Style) -> String {
        if self.on {
            format!("{}", text.style(style))
        } else {
            text.to_string()
        }
    }
}
