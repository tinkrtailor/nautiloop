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

# Only enforce on git commit commands
if [[ "${COMMAND:-}" != *"git commit"* ]]; then
  exit 0
fi

# Extract commit message from the command
# Handles: git commit -m "message", git commit -m 'message', HEREDOC patterns
commit_msg=""

# Try to extract from HEREDOC pattern (cat <<'EOF' ... EOF)
if [[ "$COMMAND" =~ cat\ \<\<[\'\"]?EOF[\'\"]? ]]; then
  # Extract content between EOF markers
  commit_msg=$(echo "$COMMAND" | sed -n '/<<.*EOF/,/EOF/p' | sed '1d;$d' | head -1)
fi

# Try to extract from -m "message" or -m 'message'
if [[ -z "$commit_msg" ]]; then
  if [[ "$COMMAND" =~ -m\ \"([^\"]+)\" ]]; then
    commit_msg="${BASH_REMATCH[1]}"
  elif [[ "$COMMAND" =~ -m\ \'([^\']+)\' ]]; then
    commit_msg="${BASH_REMATCH[1]}"
  elif [[ "$COMMAND" =~ -m\ ([^[:space:]]+) ]]; then
    commit_msg="${BASH_REMATCH[1]}"
  fi
fi

# If we couldn't extract a message, allow (might be --amend without -m, etc.)
if [[ -z "$commit_msg" ]]; then
  exit 0
fi

# Get just the first line (the subject)
subject=$(echo "$commit_msg" | head -1)

# Conventional commit regex:
# type(scope)!: description  OR  type!: description  OR  type(scope): description  OR  type: description
# Types: feat|fix|docs|style|refactor|perf|test|build|ci|chore
conv_regex='^(feat|fix|docs|style|refactor|perf|test|build|ci|chore)(\([a-z0-9_-]+\))?\!?: [a-z].*[^.]$'

if ! echo "$subject" | grep -qE "$conv_regex"; then
  export REASON
  export SUBJECT="$subject"

  python3 - <<'PY'
import json, os

subject = os.environ.get("SUBJECT", "")
reason = f"""Denied: Commit message does not follow Conventional Commits format.
See rule: .claude/rules/conventional-commits.md

Your message: {subject}

Required format: <type>(<scope>): <description>

Valid types: feat, fix, docs, style, refactor, perf, test, build, ci, chore

Examples:
  feat(contracts): add invoice cancellation
  fix(sdk-ts): correct fee calculation
  chore: update dependencies

Rules:
  - Start with valid type
  - Scope is optional, lowercase, in parentheses
  - Use ! before : for breaking changes
  - Description starts lowercase, no period at end
"""

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

exit 0
