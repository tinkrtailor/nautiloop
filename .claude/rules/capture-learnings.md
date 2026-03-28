# .claude/rules/capture-learnings.md

---

paths: "\*_/_"
priority: high
related:

- testing.md
- auto-commit-on-success.md

---

# Capture Learnings

Persist discoveries for future sessions and agents.

## Problem

Each Claude session starts fresh. Valuable discoveries about:

- How to build/test the codebase
- Non-obvious patterns and gotchas
- Commands that work (or don't)

...are lost when the session ends, causing repeated mistakes and wasted effort.

## Required Behavior

When you discover something useful, **immediately** update `.claude/learnings.md`.

### What to Capture

| Category           | Examples                                     |
| ------------------ | -------------------------------------------- |
| **Build commands** | Working build commands, required flags       |
| **Test commands**  | How to run specific tests, required env vars |
| **Environment**    | Required env vars, setup steps               |
| **Patterns**       | Codebase patterns, naming conventions        |
| **Gotchas**        | Non-obvious issues, common mistakes          |
| **Solutions**      | Commands/approaches that solved problems     |

### When to Capture

Capture learnings when you:

1. **Run a command multiple times** before getting it right
2. **Discover** a non-obvious requirement (env var, setup step)
3. **Find** a pattern that future work should follow
4. **Encounter** a gotcha that wasted time
5. **Solve** a tricky problem

## Format

Update `.claude/learnings.md` with concise entries:

```markdown
## Gotchas

- `forge test` requires running from `packages/contracts/` directory
- Tests fail without `MASTER_ENCRYPTION_KEY` env var set
- Subgraph codegen must run after contract ABIs are generated

## Commands That Work

- Run single contract test: `forge test --match-test testFunctionName -vvv`
- Rebuild subgraph after ABI changes: `bun run codegen --cwd packages/subgraph`
```

## Examples

### Bad - Not capturing

```
[Claude runs `bun test` - fails]
[Claude runs `bun test` with different flags - fails]
[Claude discovers needs MASTER_ENCRYPTION_KEY]
[Claude runs test successfully]
[Session ends - learning lost]

[Next session]
[Claude makes same mistakes again...]
```

### Good - Capturing immediately

```
[Claude runs `bun test` - fails]
[Claude discovers needs MASTER_ENCRYPTION_KEY]
[Claude updates .claude/learnings.md]:

  ## Environment
  - Tests require `MASTER_ENCRYPTION_KEY` env var to be set

[Claude runs test successfully]
[Future sessions benefit from this learning]
```

## What NOT to Capture

- Temporary debugging steps
- Session-specific context
- Information already in README/docs
- Obvious commands

## Integration with Workflow

1. **Start of session**: Read `.claude/learnings.md` first
2. **During work**: Update when discoveries are made
3. **End of work**: Review and add any missed learnings

## Learnings File Structure

```markdown
# Learnings

## Build & Test

(Package-specific build/test commands)

## Environment

(Required env vars, setup)

## Patterns

(Codebase patterns to follow)

## Gotchas

(Non-obvious issues)

## Commands That Work

(Specific commands that solved problems)
```

## Related Rules

- `testing.md` - Test patterns and documentation
- `auto-commit-on-success.md` - Commit learnings updates too
