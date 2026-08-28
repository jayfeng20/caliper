---
description: Incoming PR review — understand a PR (context, design, file-by-file), then fan out parallel bug / code-quality / blast-radius agents and finish with a reviewer's action list. Terminal output only; never posts to GitHub.
allowed-tools: Bash(gh *), Bash(git *), Bash(date *), Bash(jq *), Bash(mkdir *), Bash(cat *), Bash(ls *), Bash(wc *), Bash(find *), Read, Grep, Glob, Task
argument-hint: <PR number | PR URL> [extra focus notes]
---

# Incoming PR review — `/ipr`

You are helping **the reviewer**, not the author. A teammate has asked for a review of a PR
the reviewer has not seen before. The job has two halves and they matter equally:

1. **Understanding** — what problem this solves, what design was chosen, how each changed
   file serves that design.
2. **Judgment** — the real defects, the real blast radius, and the specific things worth
   pushing back on or asking about.

Half 1 is not a warm-up for half 2. A reviewer who understands the change writes far better
comments than one handed a findings list.

## Hard boundaries

- **Never post to GitHub.** No `gh pr review`, no `gh pr comment`, no `POST` to
  `/reviews` or `/comments`. Everything goes to the terminal.
- **Never approve a PR**, no matter how clean it looks.
- **Never mutate the working tree.** No checkout, no stash, no fetch onto HEAD. The PR's
  code is read with `git show origin/<headRef>:<path>`. The reviewer may have uncommitted
  work; do not disturb it.
- **Defer to the repo.** If this repo already has its own review command or skill that
  posts to GitHub, this command does not replace it — say so and stay local.

---

## Phase 0 — Resolve the PR

`$ARGUMENTS` is a PR number, a PR URL, or a number followed by free-text focus notes
("focus on the loader changes"). Extract the number; keep any leftover text as **reviewer
focus** and pass it verbatim to every subagent.

If `$ARGUMENTS` has no number, fall back to the current branch's PR
(`gh pr view --json number -q .number`). If that fails too, stop and ask.

```bash
PR=<resolved number>
gh pr view "$PR" --json number,title,url,state,isDraft,author,baseRefName,headRefName,additions,deletions,changedFiles,body,labels
```

Stop early and say so if the PR is `MERGED` or `CLOSED`. If it is a draft, note it and
continue. Then fetch the refs without touching HEAD:

```bash
git fetch origin <baseRefName> <headRefName>
BASE=origin/<baseRefName>
HEAD_REF=origin/<headRefName>
OUT=$(git rev-parse --show-toplevel)/.git/ipr/$PR   # inside .git, so never dirties the repo
mkdir -p "$OUT"
```

Every diff below is `git diff $BASE...$HEAD_REF`. Every file read is
`git show $HEAD_REF:<path>` (use `$BASE:<path>` for the before-state).

## Phase 0.5 — Discover this repo's conventions

Do this every run. It is what lets one command work across repos, and it is how the
subagents learn to judge by *this* codebase's standards rather than generic ones. Spend
30 seconds; do not skip it.

```bash
ROOT=$(git rev-parse --show-toplevel)
ls "$ROOT"/CLAUDE.md "$ROOT"/AGENTS.md "$ROOT"/CONTRIBUTING.md 2>/dev/null
git diff --name-only $BASE...$HEAD_REF | xargs -n1 dirname | sort -u | while read d; do
  while [ "$d" != "." ]; do
    ls "$ROOT/$d/CLAUDE.md" "$ROOT/$d/AGENTS.md" 2>/dev/null
    d=$(dirname "$d")
  done
done | sort -u
ls "$ROOT"/.claude/skills 2>/dev/null          # repo-local skills — some may be rule packs
find "$ROOT/docs" -maxdepth 3 -iname '*style*' -o -maxdepth 3 -iname '*rules*' \
  -o -maxdepth 3 -iname '*testing*' -o -maxdepth 3 -iname '*known-issues*' 2>/dev/null | head -20
ls "$ROOT"/.pre-commit-config.yaml "$ROOT"/ruff.toml "$ROOT"/.eslintrc* 2>/dev/null
```

Assemble a **conventions manifest** — a plain list of paths that exist, each labelled with
what it covers:

- root and nested `CLAUDE.md` / `AGENTS.md` / `CONTRIBUTING.md`
- house style, testing, error-handling and known-issues docs, if the repo has them
- repo-local skills whose name or description suggests a review rule pack for an area this
  diff touches (read the skill's frontmatter `description` to decide; if one matches, the
  relevant agents must load its `SKILL.md` and cite its rule IDs in findings)
- the linter/formatter config — whatever it owns, **no agent may report as a finding**

Pass this manifest to every subagent. Include only paths that actually exist: a subagent
told to read a file that is not there wastes a turn and starts inventing.

## Phase 1 — Cheap snapshot (main thread only)

Keep the main thread light — the subagents pull their own diffs. Gather only:

```bash
git diff --stat $BASE...$HEAD_REF
git log --oneline $BASE..$HEAD_REF
gh pr view $PR --comments        # existing discussion — do not repeat points already made
gh pr checks $PR                 # CI state; a red build changes what is worth reviewing
```

Classify the changed files by area (top-level package or directory). Print a 3-line
orientation for the reviewer — title, author, size, area — so they see something within
seconds, then start Phase 2.

## Phase 2 — Fan out by task type

Launch these **four agents in a single message so they run in parallel**. Each is
read-only, gets its own context, and writes its report to a file. Give each the same
header block:

- PR number, title, URL, author, draft state
- `$BASE` and `$HEAD_REF` names, and the exact diff/read commands above
- the diffstat and file→area classification
- **the conventions manifest from Phase 0.5**
- the reviewer's focus notes, if any
- its output path

| Agent | Question it answers | Output |
|---|---|---|
| `pr-explainer` | What is this and why? | `$OUT/explainer.md` |
| `pr-bug-hunter` | Is it correct? | `$OUT/bugs.md` |
| `pr-code-quality` | Is it well written? | `$OUT/quality.md` |
| `pr-blast-radius` | What else does it touch, and is that proven safe? | `$OUT/blast-radius.md` |

Scale effort to the PR: for a diff under ~50 lines touching one area, run `pr-explainer`
and `pr-bug-hunter` only, and say you skipped the other two.

## Phase 3 — Synthesize

When all four return, launch **one** `pr-review-synthesizer`. Give it the four output
paths, the PR metadata, the CI state, and the existing PR comments. It reads the reports
itself — do not paste them into its prompt.

It writes `$OUT/action-list.md`: what the reviewer should actually do, sized to the PR. A
clean 40-line PR gets two lines; a 900-line refactor with a schema change gets a real
list. It does not pad to hit a number.

## Phase 4 — Present

Print to the terminal, in this order:

1. **The explainer's output, in full.** This is the part the reviewer reads first and it
   is the reason the command exists. Do not compress it into bullets.
2. **Findings**, merged from the three judgment agents, grouped by severity, highest
   first. Drop anything the synthesizer marked as not-real. Each line:
   `[SEVERITY] path:line — one-sentence claim` followed by the concrete failure or
   improvement. Note when two agents independently flagged the same thing — that is a
   confidence signal.
3. **The action list**, verbatim from the synthesizer.
4. One line: where the reports were written, and that nothing was posted to GitHub.

Then stop. Do not offer to fix the code — it is not the reviewer's branch.

---

## Shared finding contract

Every agent that reports a finding obeys this. Repeat it in each subagent prompt.

**A finding must have** an exact `path:line` in the post-change file, and a concrete
consequence — the input, state, or sequence that makes it go wrong, or the specific
improvement and why it is better. "Consider adding error handling" is not a finding.

**Drop it if** you cannot name the consequence; the concern is handled elsewhere in the
file, the caller, or a decorator you did not read; the pattern is used consistently
elsewhere in the codebase; or you are reasoning about code the diff did not touch.
Silence beats a plausible-sounding guess — a false positive costs the reviewer more
credibility than a missed nit costs the author.

**Severities:**

- `CRITICAL` — must fix before merge: data loss, security hole, crash on a production path
- `BUG` — wrong behavior, missed edge case, logic error
- `PERFORMANCE` — measurable: N+1, O(n²) in a hot path, unbounded allocation
- `STYLE` — house-rule violation or a real readability/idiom problem
- `TESTING` — new risk without coverage, or a test that cannot fail
- `QUESTION` — genuinely unclear intent; ask the author instead of guessing
- `NICE` — a good pattern worth saying out loud

Attach a confidence 0–100. Below 60, put it in an `## Uncertain` section with the evidence
for and against rather than in the main list.

**Never flag** anything the repo's formatter or linter owns, vendored/`third_party`
directories, generated or lock files, pure moves/renames, or missing tests on trivial
config-only changes.

$ARGUMENTS
