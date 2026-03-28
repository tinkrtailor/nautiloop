---
description: Full spec implementation cycle via orchestrated plan, execute, review loops
argument-hint: [spec-path-relative-to-repo]
---

Use the `spec-orchestrator` subagent to own the workflow for the spec at @$1 end-to-end.

The orchestrator is responsible for autonomous execution from A to Z:

- invoke `spec-planner`
- invoke `spec-executor`
- invoke `spec-reviewer`
- if review fails or finds gaps, pass the findings back into `spec-planner` and repeat

The orchestrator must keep handoffs explicit on every loop:

- spec path
- impl-plan path
- reviewer findings when replanning

It must stop only when:

1. `spec-reviewer` passes, or
2. a true blocker requires user clarification

Do not pause for intermediate approvals.

Require the full `plan -> execute -> review` loop to continue until the implementation is compliant or genuinely blocked.

After successful completion, ask only for the final PR decision.
