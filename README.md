# caliper

Structured terminal review of agent-generated PR findings, for data and ML
infrastructure work.

A fleet of review agents can read a pull request far faster than you can. What they
cannot do is decide which of their twenty-five findings deserve your reviewer's
credibility. caliper is the layer in between: agents emit a **structured report**,
caliper owns **how it is presented**, and you own **what gets said**.

```
review agents ──► report.json ──► caliper render ──► you
                       ▲                              │
                       └──────── triage, rewording ────┘
```

## Why this exists

Agent review output arrives as prose. Prose cannot be ranked, filtered, diffed, or
handed back. Three things follow from that, and caliper fixes all three:

- **Ordering is meaning.** A critical data-correctness bug and a naming nit should not
  arrive as equal bullets. Severity is a total order and the renderer respects it.
- **Confidence is load-bearing.** A 55%-confidence hunch presented like a 91% finding
  is how reviewers learn to distrust the tool. Findings below the bar render as
  *leads*, not claims.
- **Review is a loop, not a report.** You will reword findings before posting them,
  and you will want to ask the agent a follow-up. That needs a document that survives
  round trips — not a terminal scrollback.

## Status

**v0.1 — the schema and the renderer.** `caliper render` is useful today. The
interactive triage TUI and GitHub submission are the next two milestones; see
[Roadmap](#roadmap).

## Install

```bash
cargo install --git https://github.com/jayfeng20/caliper caliper
```

Or from a clone: `cargo build --release` → `target/release/caliper`.

## Use

```bash
caliper validate examples/report.json   # is the report well-formed?
caliper render   examples/report.json   # print the review
caliper schema                          # the JSON Schema, for agents to emit against
```

`render` reads a path or stdin (`-`), and detects terminal width and colour support,
so it is safe to pipe into a pager or a file.

| Flag | Effect |
|---|---|
| `--max-severity bug` | blockers only |
| `--min-confidence 60` | drop the low-confidence leads |
| `--group-by category` | group by data/ML concern instead of severity |
| `--kept` | only findings you marked `keep` — what you are about to post |
| `--section explainer` | one section; repeatable and order-preserving |
| `--width`, `--no-color`, `--color` | override the auto-detected terminal |

## What the output looks like

```
PR #1842 Spill the feature-join shuffle to disk above a threshold
@rkumar · main ← feat/shuffle-spill · +431 −96 in 11 files
CI failing

critical 1 · bug 2 · performance 1 · style 1 · testing 1 · question 1 · nice 1
1 below the confidence bar — treat as leads, not claims

── FINDINGS ─────────────────────────────────────────────────────────────────

CRITICAL  (1)

  CRITICAL src/join/spill.rs:142-158  91% · ×2 agents
    The spill buffer is reused across partitions without being cleared.
    → fill_from() writes into buf starting at index 0 but truncates to the new
      batch length only on the success path. When partition N+1 produces fewer
      rows than N, the readback sees N's trailing rows appended to N+1's data,
      so the join silently emits duplicated feature rows.
    ↳ Call buf.clear() at the top of fill_from rather than truncating at the end.
    DATA_CORRECTNESS · pr-bug-hunter · pr-blast-radius  [bug-1]
```

`×2 agents` is the signal to trust most: two agents found it independently.

## The report

One JSON object per review run. Agents write it to a file and pass caliper the
**path** — never the payload as an argument.

The design that matters: **`severity` is closed, `category` is open.** Severity means
urgency, which is stable across domains and needs a total order. Category means
domain, which for data and ML infrastructure keeps growing — `DATA_CORRECTNESS`,
`SCHEMA_COMPAT`, `NUMERICS`, `PERF_REGRESSION`, `RESOURCE`, `OBSERVABILITY`. New
categories need no caliper release; a section appears under `--group-by category` if
and only if some finding carries the tag.

`decision`, `comment` and `thread` belong to *you*, not the agents. They are how
triage, your reworded text, and follow-up Q&A ride along in the same document, which
is what makes the loop above possible.

Because the prompts that generate these reports change far more often than caliper
does, the schema is built to bend: only `severity`, `path`, `line` and `claim` are
required, unknown fields are preserved through round trips rather than dropped, an
unrecognized severity degrades to `UNKNOWN` instead of failing the report, and
renames ship as serde aliases. Parsing is permissive; `caliper validate` is where
strictness lives. Those guarantees are pinned by the *schema evolution* tests in
`crates/caliper/tests/report.rs`.

Full spec: [`docs/agent-contract.md`](docs/agent-contract.md).

## The review pipeline

The agents that produce these reports live in this repo, so it is the whole pipeline
rather than half of one:

```
.claude/
  commands/ipr.md     the /ipr review command — resolves the PR, fans out, synthesizes
  commands/caliper.md validate, render, and answer follow-up questions
  agents/pr-explainer.md          what is this and why?
  agents/pr-bug-hunter.md         is it correct?
  agents/pr-code-quality.md       is it well written?
  agents/pr-blast-radius.md       what else does it touch, and is that proven safe?
  agents/pr-review-synthesizer.md what should the reviewer actually do?
```

Because they sit in `.claude/`, they are live whenever you work inside this repo — so
caliper reviews its own pull requests.

To use them in your other repos, symlink rather than copy, so this repo stays the
source of truth:

```bash
./scripts/install-claude-code.sh
```

## Roadmap

1. **`caliper review` — the interactive TUI** (ratatui). `j/k` through findings,
   `k`eep / `d`rop, `o` to open the file at the line, `e` to reword inline, `E` to
   reword in `$EDITOR`, `?` to ask a follow-up. Reads a report and writes a report,
   so triage is resumable and reviewable in git.
2. **`caliper submit`** — batch the kept findings into a single GitHub review with
   inline comments, via `gh api`. Shelling out to `gh` reuses your existing auth
   instead of handling tokens.
3. **Follow-up answers.** `?` records the question in the report and exits; the
   calling agent — which already has the repo context — answers and relaunches.
   caliper never talks to a model itself.

## Prior art

[**tuicr**](https://github.com/agavra/tuicr) is the closest neighbour and worth using
if it fits: a mature ratatui diff reviewer with line comments and multi-forge submit.
The difference is the direction of travel — tuicr is for *you* reading a diff, caliper
is for *triaging findings an agent fleet already produced*. If you want to read the
diff yourself, use tuicr. [gh-dash](https://github.com/dlvhdr/gh-dash) covers PR
navigation, and `reviewdog` covers posting linter output from CI.

## License

MIT
