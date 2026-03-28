# .claude/rules/auto-commit-on-success.md

---

paths: "\*_/_"
priority: high
related:

- conventional-commits.md
- branch-before-work.md
- pre-flight-checklist.md
- no-placeholder-implementations.md

---

# Auto-Commit on Success

Automatically commit after each successful task completion.

## Problem

Without auto-commits:

- Work is lost if session ends unexpectedly
- Large batched commits are harder to review/revert
- Progress isn't visible to user or other agents

## Required Behavior

After completing a discrete unit of work successfully, **automatically commit** without waiting for the user to ask.

## When to Auto-Commit

Commit automatically when **ALL** conditions are met:

| Condition      | Requirement                        |
| -------------- | ---------------------------------- |
| Task complete  | Feature/fix is fully implemented   |
| Tests pass     | All relevant tests pass            |
| Build succeeds | Project builds without errors      |
| Lint passes    | No linting errors in changed files |
| No secrets     | No credentials in staged changes   |

## When NOT to Auto-Commit

| Situation                | Action                      |
| ------------------------ | --------------------------- |
| Work in progress         | Wait until complete         |
| Tests failing            | Fix tests first             |
| Build broken             | Fix build first             |
| Exploring/researching    | Don't commit exploration    |
| User said they'll commit | Respect user preference     |
| Changes are experimental | Get user confirmation first |

## Workflow

```
1. Complete implementation
2. Run checks:
   - forge test -vvv (contracts)
   - bun run build && bun run lint (TypeScript)
3. If checks pass:
   git add -A
   git commit -m "<conventional commit message>"
4. VERIFY commit succeeded:
   git log -1 --oneline
   (If hook denied, commit did NOT happen - fix message and retry)
5. Inform user: "Committed: `<message>`"
6. Continue to next task
```

## ⚠️ Commit Hooks Can Deny Commits

This repo has PreToolUse hooks that can **deny** your commit:

- `enforce_conventional_commits.sh` - Rejects non-conventional commit messages

**When a commit is denied:**

- The commit **does NOT happen** (your changes are not saved)
- You see a "Denied:" message with the reason
- You MUST fix the issue and retry

**Always verify with `git log -1 --oneline` after committing.**

## Commit Granularity

| Principle             | Description                          |
| --------------------- | ------------------------------------ |
| One change per commit | Don't batch unrelated changes        |
| Atomic commits        | Each commit independently revertable |
| Small commits         | Multiple small > one large           |

## Examples

### Good: Auto-commit after feature

```
User: Add invoice cancellation to the contract

[Claude implements]
[Claude runs: forge test -vvv -> passes]
[Claude commits: feat(contracts): add invoice cancellation]

"Done! Committed: feat(contracts): add invoice cancellation"
```

### Good: Multiple commits for multi-part task

```
User: Add validation and tests for the endpoint

[Claude adds validation]
[Tests pass]
[Commits: feat(api): add request validation for invoice endpoint]

[Claude adds tests]
[Tests pass]
[Commits: test(api): add integration tests for invoice validation]

"Added validation and tests in 2 commits."
```

### Bad: Committing broken code

```
[Claude implements]
[Tests FAIL]
[Claude commits anyway]  ❌ WRONG
```

## No Attribution in Commits or PRs

Do NOT include:

- `Co-Authored-By: Claude` or similar co-author lines
- "Generated with Claude Code" or similar attribution text
- Emoji badges or AI-generated markers
- Any indication that the commit/PR was AI-assisted

Commits and PRs should look like normal human-authored contributions.

## Anti-patterns

❌ Committing without running tests
❌ Batching many changes into one commit
❌ Committing partial/placeholder implementations
❌ Committing when build is broken
❌ Adding "Co-Authored-By: Claude" to commits
❌ Adding "Generated with Claude Code" to PRs

## Good Patterns

✅ Run checks before every commit
✅ One logical change per commit
✅ Inform user what was committed
✅ Use conventional commit format

## Integration with Task Tracking

- Mark task `completed` only AFTER commit succeeds
- If commit fails (hook rejects), task stays `in_progress`

## Related Rules

- `conventional-commits.md` - Commit message format
- `branch-before-work.md` - Commits go to feature branch
- `pre-flight-checklist.md` - What checks to run
- `no-placeholder-implementations.md` - Only commit complete code
