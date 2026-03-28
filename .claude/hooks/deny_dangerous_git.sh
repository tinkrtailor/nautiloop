#!/usr/bin/env bash
set -euo pipefail

payload="$(cat)"

# Extract fields from hook JSON
eval "$(
  python3 - <<'PY' <<<"$payload"
import json, shlex, sys
d = json.load(sys.stdin)
ti = d.get("tool_input") or {}
tool = d.get("tool_name","")
command = ti.get("command") or ""
print("TOOL_NAME=" + shlex.quote(tool))
print("COMMAND=" + shlex.quote(command))
PY
)"

# Only enforce on Bash tool calls
if [[ "${TOOL_NAME:-}" != "Bash" ]]; then
  exit 0
fi

# Only enforce on git commands
if [[ "${COMMAND:-}" != *"git "* ]]; then
  exit 0
fi

# Dangerous git patterns
# Note: git push (normal) is intentionally allowed
# Note: git checkout -b (branch creation) is intentionally allowed
DANGEROUS_PATTERNS=(
  'git reset --hard'
  'git checkout \.\s*$'
  'git checkout \.$'
  'git checkout -- \.'
  'git restore \.\s*$'
  'git restore \.$'
  'git restore --staged \.'
  'git clean -[a-z]*f'
  'git branch -D'
  'git push --force'
  'git push -f\b'
  'git push [^ ]*--force'
  'git push [^ ]*-f\b'
  'git push.*--force-with-lease'
)

for pattern in "${DANGEROUS_PATTERNS[@]}"; do
  if echo "$COMMAND" | grep -qE "$pattern"; then
    export MATCHED_PATTERN="$pattern"
    export MATCHED_COMMAND="$COMMAND"

    python3 - <<'PY'
import json, os

pattern = os.environ.get("MATCHED_PATTERN", "")
command = os.environ.get("MATCHED_COMMAND", "")
reason = f"""Denied: Destructive git command blocked.

Command: {command}
Matched pattern: {pattern}

This hook prevents dangerous git operations that can cause data loss.
If you need to run this command, ask the user for explicit confirmation first.

See rule: .claude/rules/git-guardrails.md"""

print(json.dumps({
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": reason
  }
}))
PY
    exit 0
  fi
done

exit 0
