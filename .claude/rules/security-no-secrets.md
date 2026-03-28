# .claude/rules/security-no-secrets.md

---

paths: "\*_/_"
priority: critical
related:

- auto-commit-on-success.md
- pre-flight-checklist.md

---

# Security: No Secrets in Code

Never commit secrets, credentials, or sensitive data.

## Problem

Secrets committed to version control:

- Are exposed to anyone with repo access
- Persist in git history even after deletion
- Can be scraped by automated tools
- Cause security incidents and credential rotation

## Required Behavior

### Never Commit

| Type                   | Examples                                  |
| ---------------------- | ----------------------------------------- |
| **API Keys**           | `sk-...`, `pk_live_...`, `AKIA...`        |
| **Private Keys**       | Ethereum private keys, SSH keys, PGP keys |
| **Passwords**          | Database passwords, service passwords     |
| **Tokens**             | JWT secrets, session secrets, auth tokens |
| **Connection Strings** | Database URLs with credentials            |
| **Environment Files**  | `.env`, `.env.local`, `.env.production`   |

### Files to Never Commit

```gitignore
# These should be in .gitignore
.env
.env.local
.env.*.local
*.pem
*.key
credentials.json
secrets.json
```

### Safe Patterns

```typescript
// ✅ Good - Read from environment
const apiKey = process.env.API_KEY;

// ✅ Good - Use placeholder in examples
const exampleConfig = {
  apiKey: 'your-api-key-here',
};

// ✅ Good - Reference .env.example
// See .env.example for required environment variables
```

### Unsafe Patterns

```typescript
// ❌ Bad - Hardcoded secret
const apiKey = 'sk-1234567890abcdef';

// ❌ Bad - Hardcoded private key
const privateKey = '0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80';

// ❌ Bad - Connection string with password
const dbUrl = 'postgres://user:password123@localhost:5432/db';
```

## Before Every Commit

Check for secrets:

```bash
# Review staged changes for secrets
git diff --cached | grep -E "(sk-|pk_|AKIA|password|secret|0x[a-fA-F0-9]{64})"

# Or use a secrets scanner
# gitleaks detect --staged
```

## What to Do If You Find a Secret

### In Staged Changes

```bash
# Unstage the file
git reset HEAD <file>

# Remove the secret from the file
# Use environment variable instead

# Re-stage and commit
```

### Already Committed (Not Pushed)

```bash
# Amend the commit (if it's the last one)
git commit --amend

# Or reset and recommit
git reset HEAD~1
# Fix the file, then commit again
```

### Already Pushed

1. **Immediately rotate the credential** - Assume it's compromised
2. Remove from code and commit the fix
3. Consider using `git filter-branch` or BFG to remove from history
4. Notify the team

## Environment Variables

### Required Setup

```bash
# .env.example (committed - template only)
DATABASE_URL=postgres://user:pass@localhost:5432/db
API_KEY=your-api-key-here
MASTER_ENCRYPTION_KEY=generate-a-secure-key

# .env.local (not committed - real values)
DATABASE_URL=postgres://realuser:realpass@localhost:5432/mydb
API_KEY=sk-abc123...
MASTER_ENCRYPTION_KEY=actual-secure-key
```

### Validation

```typescript
// Validate required env vars at startup
import { z } from 'zod';

const envSchema = z.object({
  DATABASE_URL: z.string().url(),
  API_KEY: z.string().min(1),
});

export const env = envSchema.parse(process.env);
```

## Warning Signs

If you see yourself writing:

- A long hex string (private key)
- `sk-`, `pk_`, `AKIA` prefixes (API keys)
- `password=` or `secret=` in code
- Anything that looks like a credential

**STOP** and use an environment variable instead.

## Related Rules

- `auto-commit-on-success.md` - Check for secrets before committing
- `pre-flight-checklist.md` - Include secrets check in pre-flight
