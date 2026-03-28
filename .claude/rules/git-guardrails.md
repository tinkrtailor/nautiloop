# .claude/rules/git-guardrails.md

---

paths: "\*_/_"
priority: critical
enforced_by: .claude/hooks/deny_dangerous_git.sh
related:

- branch-before-work.md
- auto-commit-on-success.md

---

# Git Guardrails

Prevent destructive git commands that can cause data loss.

## Problem

Destructive git commands can:

- Destroy uncommitted work (`git reset --hard`, `git checkout .`)
- Overwrite remote history (`git push --force`)
- Delete untracked files permanently (`git clean -f`)
- Remove branches with unmerged work (`git branch -D`)

## Blocked Commands

| Command                       | Risk                                |
| ----------------------------- | ----------------------------------- |
| `git reset --hard`            | Discards all uncommitted changes    |
| `git checkout .`              | Discards all working tree changes   |
| `git checkout -- .`           | Discards all working tree changes   |
| `git restore .`               | Discards all working tree changes   |
| `git restore --staged .`      | Unstages everything                 |
| `git clean -f` (any variant)  | Permanently deletes untracked files |
| `git branch -D`               | Force-deletes branch                |
| `git push --force`            | Overwrites remote history           |
| `git push -f`                 | Overwrites remote history           |
| `git push --force-with-lease` | Overwrites remote history           |

## Allowed Commands

| Command              | Why allowed                        |
| -------------------- | ---------------------------------- |
| `git push`           | Normal push, safe                  |
| `git push -u`        | Push with upstream tracking, safe  |
| `git checkout -b`    | Creates a new branch, safe         |
| `git reset` (soft)   | Soft/mixed reset, keeps changes    |
| `git branch -d`      | Safe delete (only merged branches) |
| `git restore <file>` | Single file restore, intentional   |

## Enforcement

This rule is enforced by hook: `.claude/hooks/deny_dangerous_git.sh`

Blocked commands are denied automatically. If a destructive command is truly needed, ask the user for explicit confirmation first.

## Related Rules

- `branch-before-work.md` - Work on branches, not main
- `auto-commit-on-success.md` - Commit frequently to avoid data loss
