# .claude/rules/no-placeholder-implementations.md

---

paths: "\*_/_.{ts,tsx,sol,js,jsx}"
priority: critical
related:

- search-before-implementing.md
- auto-commit-on-success.md

---

# No Placeholder Implementations

**CRITICAL RULE:** Every implementation must be complete and production-ready.

## Problem

LLMs have an inherent bias toward minimal implementations that "just make tests pass" or satisfy the immediate request. This creates technical debt and causes future failures when the placeholder is encountered in production.

## Required Behavior

### Never create:

- `// TODO: implement later`
- `// FIXME: incomplete`
- `throw new Error("Not implemented")`
- `throw new Error("TODO")`
- `console.log("placeholder")`
- Empty function bodies
- Stubbed functions with hardcoded return values
- Minimal implementations that only handle the happy path

### Always create:

- Complete implementations that handle all specified cases
- Proper error handling for edge cases
- Full validation of inputs
- Appropriate logging/observability

## Examples

### Bad - Placeholder implementations

```typescript
// ❌ Empty stub
async function calculateFee(amount: bigint): Promise<bigint> {
  // TODO: implement fee calculation
  return 0n;
}

// ❌ Minimal implementation
async function getInvoice(id: string): Promise<Invoice> {
  const invoice = await db.find(id);
  return invoice; // Missing: null check, error handling, validation
}

// ❌ Hardcoded return
function validateInput(data: unknown): boolean {
  return true; // TODO: add validation
}
```

### Good - Complete implementations

```typescript
// ✅ Full implementation
async function calculateFee(amount: bigint): Promise<bigint> {
  if (amount <= 0n) {
    throw new ValidationError('Amount must be positive');
  }
  const fee = (amount * FEE_BASIS_POINTS) / 10000n;
  return fee > MAX_FEE ? MAX_FEE : fee;
}

// ✅ Complete with error handling
async function getInvoice(id: string): Promise<Invoice> {
  if (!isValidUUID(id)) {
    throw new ValidationError(`Invalid invoice ID: ${id}`);
  }
  const invoice = await db.find(id);
  if (!invoice) {
    throw new NotFoundError(`Invoice not found: ${id}`);
  }
  return invoice;
}
```

## When You Cannot Complete an Implementation

If you cannot fully implement something:

1. **STOP** - Do not create a placeholder
2. **Document** the blocker in the impl-plan.md
3. **Ask** the user for clarification or guidance
4. **Wait** for resolution before proceeding

## Anti-patterns

❌ "I'll add a basic version and improve it later"
❌ "This is enough to make the tests pass"
❌ "The user can fill in the details"
❌ Creating stubs to "unblock" other work

## Good Patterns

✅ Implement fully or not at all
✅ Ask for clarification when requirements are unclear
✅ Document blockers instead of creating placeholders
✅ Handle all error cases from the start

## Enforcement

If you find yourself writing TODO/FIXME/placeholder:

1. Stop and reconsider
2. Is the requirement clear enough to implement fully?
3. If yes → implement fully
4. If no → ask for clarification, don't create placeholder

## Related Rules

- `search-before-implementing.md` - Search for existing implementations first
- `auto-commit-on-success.md` - Only commit complete, working code
