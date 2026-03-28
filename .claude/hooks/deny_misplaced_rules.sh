#!/usr/bin/env bash
set -euo pipefail

payload="$(cat)"

# Extract fields from hook JSON (no jq dependency)
eval "$(
  python3 - <<'PY' <<<"$payload"
import json, shlex, sys
d = json.load(sys.stdin)
ti = d.get("tool_input") or {}
tool = d.get("tool_name","")
file_path = ti.get("file_path") or ti.get("path") or ""
cwd = d.get("cwd") or ""
print("TOOL_NAME=" + shlex.quote(tool))
print("FILE_PATH=" + shlex.quote(file_path))
print("CWD=" + shlex.quote(cwd))
PY
)"

# Only enforce on Write/Edit tool calls
if [[ "${TOOL_NAME:-}" != "Write" && "${TOOL_NAME:-}" != "Edit" ]]; then
  exit 0
fi

# Only enforce for Cursor rule files
if [[ -z "${FILE_PATH:-}" || "${FILE_PATH}" != *.mdc ]]; then
  exit 0
fi

project_dir="${CLAUDE_PROJECT_DIR:-${CWD:-$(pwd)}}"
project_dir="${project_dir%/}"

# Normalize to a repo-relative-ish path for checking
fp="${FILE_PATH#./}"
if [[ "$fp" == /* ]]; then
  if [[ "$fp" == "$project_dir/"* ]]; then
    rel="${fp#"$project_dir/"}"
  else
    rel="$fp" # absolute path outside project -> deny
  fi
else
  rel="$fp"
fi

# Allow only .cursor/rules/**/*.mdc
if [[ "$rel" != .cursor/rules/* ]]; then
  export REASON
  REASON=$(
    cat <<EOF
Denied: Cursor rule files (*.mdc) must be created/edited under .cursor/rules/.
See rule: .claude/rules/cursor-mdc-location.md

Attempted: ${FILE_PATH}
Fix: use file_path=".cursor/rules/<kebab-name>.mdc"
EOF
  )

  python3 - <<'PY'
import json, os
print(json.dumps({
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": os.environ["REASON"]
  }
}))
PY
fi

exit 0

