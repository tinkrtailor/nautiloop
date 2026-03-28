---
description: Review implementation compliance against a spec
agent: spec-reviewer
subtask: true
---

Review the current implementation against the spec at @$1.

Inputs to keep explicit:

- spec path: `$1`
- impl-plan path: derive `*-impl-plan.md` next to the spec if present

Requirements:

- determine the correct diff target explicitly
- search before concluding that anything is missing
- review requirements, acceptance criteria, ADR alignment, deviations, and testing gaps
- return an explicit `PASS` or `FAIL`
- include a replanning-ready findings section if the verdict is `FAIL`
