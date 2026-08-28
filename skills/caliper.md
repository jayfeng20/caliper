---
description: Render an /ipr review run through caliper — validates the structured report the review agents produced, then prints it as a ranked, grouped terminal review.
allowed-tools: Bash(caliper *), Bash(cargo run *), Read
argument-hint: <path to report.json>
---

# caliper

The `/ipr` agents produce a structured report; caliper owns how it is presented. Do
not hand-format review output when this tool is available — the ranking, grouping,
confidence bar and width handling are its job, and doing it by hand produces
inconsistent results run to run.

## Steps

1. **Validate first.** `caliper validate $ARGUMENTS`

   Every warning is a defect in the report, not in caliper. Fix the report and
   re-validate before rendering. Common ones:

   - *no consequence* — the finding does not meet the contract; drop it.
   - *severity not recognized* — use a known severity; put the domain in `category`.
   - *head_sha missing* — capture `gh pr view --json headRefOid` in phase 0.
   - *unknown field* — harmless and preserved, but check it was not a typo.

2. **Render.** `caliper render $ARGUMENTS`

   Print the output as-is. Do not summarize it, re-order it, or drop sections —
   the reviewer asked for the full picture and the ordering carries meaning.

3. **Answer anything pending.** If the output says *awaiting an answer from the
   review agent*, the reviewer asked a follow-up question during triage. Read the
   `thread` on that finding, investigate the code properly, append an `agent` entry
   with the answer, and re-render. Do not guess; the reviewer asked because the
   original finding was not self-explanatory.

## Useful variations

| Need | Command |
|---|---|
| Just the blockers | `caliper render --max-severity bug <report>` |
| Drop the low-confidence leads | `caliper render --min-confidence 60 <report>` |
| Group by data/ML concern | `caliper render --group-by category <report>` |
| What the reviewer decided to post | `caliper render --kept <report>` |
| One section only | `caliper render --section explainer <report>` |

## Boundaries

- **Never post to GitHub.** caliper renders; the reviewer decides. No `gh pr review`,
  no `gh pr comment`.
- **Never approve a PR.**
- **Never edit `decision`, `comment` or `thread`** except to append an `agent` answer
  to a thread. Those fields are the reviewer's, and overwriting them destroys triage
  work.

See `docs/agent-contract.md` for the full report contract.
