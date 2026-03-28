---
description: Run the full spec workflow through the dedicated orchestrator
agent: spec-orchestrator
subtask: true
---

Run the full implementation workflow for the spec at @$1 through the dedicated `spec-orchestrator` agent.

The orchestrator owns the workflow end to end and must run autonomously from A to Z.

Requirements:

- invoke `spec-planner`
- invoke `spec-executor`
- invoke `spec-reviewer`
- if review fails or finds gaps, pass reviewer findings back into `spec-planner` and repeat

Handoffs must stay explicit on every loop:

- spec path
- impl-plan path
- reviewer findings when replanning

Stop only when:

1. the reviewer passes, or
2. a true blocker requires user clarification

Ask the user only for true blockers and the final PR decision after success.
