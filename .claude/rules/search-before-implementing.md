# .claude/rules/search-before-implementing.md

---

paths: "\*_/_"
priority: critical
related:

- no-placeholder-implementations.md
- capture-learnings.md

---

# Search Before Implementing

**CRITICAL:** Never implement functionality without first searching the codebase.

## Problem

A common failure mode:

1. Quick search (ripgrep) with one term
2. Conclude "not implemented"
3. Create duplicate implementation
4. Cause conflicts and wasted effort

Code search is **non-deterministic** - different search terms yield different results.

## Required Behavior

Before implementing ANY new functionality:

### 1. Search Extensively

Use multiple search strategies:

| Strategy                | Example                              |
| ----------------------- | ------------------------------------ |
| Exact name              | `cancelInvoice`                      |
| Regex patterns          | `cancel.*invoice`, `invoice.*cancel` |
| Related keywords        | `void`, `refund`, `terminate`        |
| TODOs/FIXMEs            | `TODO.*invoice`, `FIXME.*cancel`     |
| Partial implementations | Functions with empty bodies          |

### 2. Use Subagents for Search

Delegate search to subagents to:

- Preserve main context window
- Run multiple searches in parallel
- Get comprehensive results

### 3. Verify Before Proceeding

Only implement if you have **high confidence** it doesn't exist.

If uncertain: "I found X which seems related. Should I extend it or create new?"

## When You Find Existing Code

| Finding               | Action                            |
| --------------------- | --------------------------------- |
| Fully implemented     | Don't re-implement. Reference it. |
| Partially implemented | Extend it, don't duplicate.       |
| Placeholder/TODO      | Replace with full implementation. |
| Similar but different | Ask user: extend or create new?   |

## Examples

### Bad: Single search

```
"I searched for 'cancelInvoice' and found nothing. Creating new function."
❌ Only searched one term
```

### Good: Multiple strategies

```
"I searched using multiple strategies:
 - 'cancelInvoice' - no exact match
 - 'cancel.*invoice' - found partial in utils.ts
 - 'invoice.*cancel' - found TODO in service.ts
 - 'void.*invoice' - found existing voidInvoice()

 Found partial implementation in service.ts:45 with TODO.
 Recommend: Complete the existing TODO rather than duplicate."
✅ Thorough search, found existing code
```

## Anti-patterns

❌ Single ripgrep search → "not found" → implement
❌ Searching only exact name matches
❌ Assuming different naming = not implemented
❌ Ignoring partial/placeholder implementations
❌ Creating duplicates because search missed code

## Good Patterns

✅ Multiple search strategies
✅ Search for patterns, not just names
✅ Check for TODOs and placeholders
✅ Use subagents for parallel search
✅ Ask when uncertain

## Integration with Agents

This rule is enforced in:

- **spec-executor**: Search before each step
- **spec-planner**: Search before adding plan items
- **spec-reviewer**: Flag duplicate implementations

## Remember

> "Do NOT assume an item is not implemented - verify first."
>
> If you create duplicates because you didn't search thoroughly, you have failed.

## Related Rules

- `no-placeholder-implementations.md` - Don't create placeholders either
- `capture-learnings.md` - Document where things are found
