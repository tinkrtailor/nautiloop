# .claude/rules/branch-before-work.md

---

paths: "\*_/_"
priority: high
related:

- auto-commit-on-success.md
- conventional-commits.md
- pre-flight-checklist.md

---

# Branch Before Work

Always create a branch before beginning implementation work.

## Problem

Working directly on main:

- Pollutes main branch history
- Makes it hard to abandon failed approaches
- Blocks parallel work
- Complicates code review

## Required Behavior

**NEVER commit or push directly to main.** All changes — regardless of size — must go through a branch and PR.

When starting a new feature, task, or spec implementation:

1. **Create a branch** with a descriptive name
2. **Push the branch** to remote
3. **Then start working**
4. **Open a PR** to merge into main

## Branch Naming

Use `<type>/<short-description>` format:

| Type        | Use For           |
| ----------- | ----------------- |
| `feat/`     | New features      |
| `fix/`      | Bug fixes         |
| `refactor/` | Code refactoring  |
| `docs/`     | Documentation     |
| `chore/`    | Maintenance tasks |
| `test/`     | Test additions    |

### Examples

```
feat/invoice-cancellation
fix/fee-calculation-overflow
refactor/registry-storage
docs/api-authentication
chore/update-dependencies
```

## Workflow

### Step 1: Create and push branch

```bash
git checkout -b feat/my-feature
git push -u origin feat/my-feature
```

### Step 2: Implement

Work on the feature. Auto-commits go to this branch.

### Step 3: Create PR when ready

```bash
gh pr create --title "feat: my feature" --body "..."
```

## When This Applies

| Situation                 | Create Branch? |
| ------------------------- | -------------- |
| New feature (any size)    | ✅ Yes         |
| Bug fix                   | ✅ Yes         |
| Refactoring               | ✅ Yes         |
| Spec implementation       | ✅ Yes         |
| Multi-file changes        | ✅ Yes         |
| Single-line change        | ✅ Yes         |
| Already on feature branch | ❌ No          |
| User says work on current | ❌ No          |

## Integration with Worktrees

When using `claude-worktree new <name>`:

- Branch is created automatically (via `-b` flag)
- Just push: `git push -u origin <name>`

## Examples

### Good: Branch before feature

```
User: Implement invoice cancellation

Claude:
1. git checkout -b feat/invoice-cancellation
2. git push -u origin feat/invoice-cancellation
3. "Created branch. Starting implementation..."
4. [implements with auto-commits]
5. "Done! Ready to create PR."
```

### Bad: Working on main

```
User: Implement invoice cancellation

Claude:
1. [starts implementing on main]  ❌ WRONG
```

## Anti-patterns

❌ Committing or pushing directly to main — NO EXCEPTIONS
❌ Implementing directly on main
❌ Creating branch after making changes
❌ Using vague branch names (`fix/stuff`)
❌ Not pushing branch to remote

## Good Patterns

✅ Create branch before any implementation
✅ Use descriptive, conventional names
✅ Push immediately after creation
✅ One feature per branch

## Related Rules

- `auto-commit-on-success.md` - Commits go to feature branch
- `conventional-commits.md` - Branch name matches commit type
- `pre-flight-checklist.md` - Run before creating PR
