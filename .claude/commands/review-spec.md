---
description: Review implementation compliance against a spec
argument-hint: [spec-path-relative-to-repo]
---

Use the spec-reviewer subagent to review the current implementation against the spec at @$1.

Read the spec, determine the appropriate diff (local changes, branch vs main, or PR), and produce a structured compliance report.

This is a **read-only review** - do not make any code changes.
