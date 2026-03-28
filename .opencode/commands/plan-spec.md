---
description: Create or revise an implementation plan for a spec
agent: spec-planner
subtask: true
---

Create or revise the implementation plan for the spec at @$1.

Inputs to keep explicit:

- spec path: `$1`
- impl-plan path: derive `*-impl-plan.md` next to the spec

Requirements:

- analyze the spec and codebase deeply before planning
- search for existing implementations and patterns before adding plan items
- create or revise the impl plan in place
- keep the plan priority-ordered and execution-ready
- only surface true blockers; do not ask for routine approval
