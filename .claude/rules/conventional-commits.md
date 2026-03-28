# .claude/rules/conventional-commits.md

---

paths: "\*_/_"
priority: critical
enforced_by: .claude/hooks/enforce_conventional_commits.sh
related:

- auto-commit-on-success.md
- branch-before-work.md

---

# Conventional Commits

All commits must follow the Conventional Commits specification.

## Problem

Inconsistent commit messages:

- Make history hard to read
- Break automated changelog generation
- Complicate release management
- Make it hard to find specific changes

## Required Format

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

## Types

| Type       | Description      | Example                                |
| ---------- | ---------------- | -------------------------------------- |
| `feat`     | New feature      | `feat(api): add invoice endpoint`      |
| `fix`      | Bug fix          | `fix(sdk): correct fee calculation`    |
| `docs`     | Documentation    | `docs(readme): update setup guide`     |
| `style`    | Formatting only  | `style(web): fix indentation`          |
| `refactor` | Code restructure | `refactor(api): extract service layer` |
| `perf`     | Performance      | `perf(db): add query index`            |
| `test`     | Tests            | `test(contracts): add fuzz tests`      |
| `build`    | Build system     | `build: update dependencies`           |
| `ci`       | CI config        | `ci: add coverage report`              |
| `chore`    | Maintenance      | `chore: clean up unused files`         |

## Scope

The scope indicates the affected area:

| Scope Type | Examples                                              |
| ---------- | ----------------------------------------------------- |
| Package    | `contracts`, `sdk-ts`, `web`, `api`, `subgraph`, `ui` |
| Feature    | `invoice`, `auth`, `registry`, `nft`                  |
| Omit       | If change is truly global                             |

## Rules

| Rule                 | Requirement                    |
| -------------------- | ------------------------------ |
| Type required        | Always start with valid type   |
| Description required | Concise, imperative mood       |
| Lowercase            | Type, scope, description       |
| No period            | Don't end description with `.` |
| Breaking changes     | Add `!` after type/scope       |

## Examples

### Good

```
feat(contracts): add invoice cancellation with partial refund
fix(sdk-ts): correct fee calculation for small amounts
docs(api): update authentication endpoint examples
refactor(web): extract invoice list into separate component
test(contracts): add fuzz tests for Registry
chore: update dependencies
feat(invoice)!: change status enum values
```

### Bad

```
Updated the code                     ❌ no type, vague
feat: Added new feature.             ❌ past tense, period
FIX(API): Fix bug                    ❌ uppercase, vague
feature(contracts): add thing        ❌ wrong type name
```

## Multi-line Commits

For complex changes:

```
feat(contracts): add batch invoice creation

- Support creating up to 50 invoices in single transaction
- Optimize gas by batching storage writes
- Add event for batch creation tracking

Closes #123
```

## Breaking Changes

```
feat(api)!: remove deprecated v1 endpoints

BREAKING CHANGE: The /v1/* endpoints have been removed.
Migrate to /v2/* endpoints before upgrading.
```

## Enforcement

This rule is enforced by a git hook:

- `.claude/hooks/enforce_conventional_commits.sh`
- Commits that don't match the format are rejected

## Anti-patterns

❌ Vague descriptions ("fix stuff", "update code")
❌ Past tense ("added", "fixed")
❌ Missing type
❌ Ending with period

## Good Patterns

✅ Imperative mood ("add", "fix", "update")
✅ Specific descriptions
✅ Appropriate type and scope
✅ Breaking changes marked with `!`

## Related Rules

- `auto-commit-on-success.md` - When to commit
- `branch-before-work.md` - Branch names match commit types
