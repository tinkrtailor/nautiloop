---
description: Execute an implementation spec with the dedicated spec executor
agent: spec-executor
subtask: true
---

Implement the spec at @$1.

Inputs to keep explicit:

- spec path: `$1`
- impl-plan path: derive `*-impl-plan.md` next to the spec, or create it if missing

Requirements:

- run autonomously
- satisfy branch-before-work rules
- search before implementing each step
- update the impl plan as work progresses
- run relevant tests and checks
- auto-commit successful units of work with verified conventional commits
- stop only for a true blocker
