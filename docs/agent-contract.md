# The agent contract

This is the normative spec for anything that produces a caliper report — today the
`/ipr` review agents, tomorrow whatever replaces them. `caliper schema` emits the
machine-readable version; this document explains the parts a schema cannot express.

## The shape

One JSON object per review run. Write it to a file; pass caliper the **path**, never
the payload. Agent-generated shell with a multi-kilobyte quoted JSON argument is a
reliability problem, not an interface.

```
$OUT/report.json   ->   caliper render $OUT/report.json
```

## What is required

Only what is needed to put a finding on screen:

| Field | Why it is required |
|---|---|
| `findings[].severity` | ranking is the whole point |
| `findings[].path`, `findings[].line` | a finding without a location is not reviewable |
| `findings[].claim` | the one-sentence assertion |

Everything else has a default. A thin report renders with gaps rather than failing —
losing a whole review to one malformed field is the worst possible outcome. Report
*quality* is enforced separately by `caliper validate`, which warns about missing
`consequence`, missing `agent`, absent `head_sha`, duplicate ids and unknown fields.

Run `caliper validate` before `caliper render`. Fix what it names.

## Severity means urgency, category means domain

This split is the one thing to get right.

**`severity`** is a closed set, ordered by how much it should interrupt the reviewer:

| | |
|---|---|
| `CRITICAL` | must fix before merge: data loss, security hole, crash on a production path |
| `BUG` | wrong behavior, missed edge case, logic error |
| `PERFORMANCE` | measurable: N+1, O(n²) in a hot path, unbounded allocation |
| `STYLE` | house-rule violation or a real readability problem |
| `TESTING` | new risk without coverage, or a test that cannot fail |
| `QUESTION` | genuinely unclear intent; ask rather than guess |
| `NICE` | a good pattern worth saying out loud |

Do not invent severities. An unrecognized value parses as `UNKNOWN` rather than
failing the report, but it renders as `UNKNOWN` and `validate` complains.

**`category`** is an open set, and it is where domain-specific concerns go. The
suggested values for data/ML infrastructure work:

`CORRECTNESS` · `DATA_CORRECTNESS` · `SCHEMA_COMPAT` · `NUMERICS` ·
`PERF_REGRESSION` · `RESOURCE` · `OBSERVABILITY` · `API_COMPAT` · `SECURITY`

New categories need no caliper change — a section appears in
`caliper render --group-by category` if and only if some finding carries the tag.
This is the axis to extend as the review prompt grows.

## What makes a finding

A finding needs an exact `path:line` in the post-change file and a concrete
consequence: the input, state or sequence that makes it go wrong, or — for a
non-defect — the specific improvement and why it is better. *"Consider adding error
handling"* is not a finding.

Drop it if you cannot name the consequence; if the concern is handled elsewhere in
the file, the caller, or a decorator you did not read; if the pattern is used
consistently across the codebase; or if you are reasoning about code the diff did not
touch. Silence beats a plausible-sounding guess — a false positive costs the reviewer
more credibility than a missed nit costs the author.

Never flag anything the repo's formatter or linter owns, vendored or generated files,
pure moves and renames, or missing tests on config-only changes.

`confidence` is 0-100 and it is load-bearing: below 60 the finding renders as a
*lead* rather than a claim, and `--min-confidence` filters on it. Omitting it assumes
50, which is a worse outcome than guessing honestly.

`corroborated_by` lists other agents that independently found the same thing. Fill it
in — independent agreement is the strongest signal a reviewer gets, and caliper
surfaces it as `×N agents`.

## Fields agents must leave alone

`decision`, `comment` and `thread` on a finding belong to the reviewer. `caliper
review` writes them and hands the report back. Two consequences:

- Reading a report that already has them, **preserve them**. A re-run that wipes the
  reviewer's triage is a bug.
- When `thread` ends with a `reviewer` entry, the reviewer asked a question and is
  waiting. Answer it by appending an `agent` entry with the answer, then relaunch
  caliper. `caliper validate` and the rendered output both flag unanswered questions.

## Evolving the contract

The prompts change more often than caliper does, so:

- **Adding a field** is always safe. Unknown fields are preserved through round trips
  via the `extra` catch-all, so a prompt can emit something new before caliper
  understands it and the value is not lost.
- **Renaming a field** ships as `#[serde(alias = "old_name")]`. Never rename bare.
- **Removing a field** is safe if it had a default.
- **Bump `schema_version`** only when an existing field changes *meaning*. caliper
  reads a higher version and warns rather than refusing.

The four tests in `crates/caliper/tests/report.rs` under *schema evolution* are this
promise in executable form. Breaking one of them is a breaking change to the pipeline.
