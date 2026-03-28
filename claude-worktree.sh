#!/bin/bash
# claude-worktree.sh - Manage parallel Claude Code sessions with Git worktrees
#
# Usage:
#   claude-worktree new <name> [spec] [--sandbox] [--fresh] [-p <prompt>]
#   claude-worktree attach <name>
#   claude-worktree done <name>
#   claude-worktree list
#
# Options:
#   --sandbox      Run Claude in Docker sandbox (AFK mode)
#   --fresh        Run full bun install + submodule init (skip symlink optimization)
#   -p <prompt>    Send an initial prompt after Claude starts
#
# Environment Variables:
#   CLAUDE_GH_PAT  GitHub PAT for gh CLI auth in sandbox (passed as GH_TOKEN)
#
# Example workflow:
#   claude-worktree new invoice-api
#   # ... attach, work in Claude, create PR ...
#   claude-worktree done invoice-api
#
# With spec (auto-starts implementation):
#   claude-worktree new invoice-api specs/payments/payment-history.md
#   # Claude starts and runs /implement-spec automatically
#
# With prompt (any initial command):
#   claude-worktree new api-key-ui --sandbox -p "/implement-spec @specs/mcp/api-key-ui.md"
#
# Sandbox mode (isolated container):
#   export CLAUDE_GH_PAT=ghp_xxx  # for gh CLI to push/create PRs
#   claude-worktree new invoice-api --sandbox
#
# Sandbox auth: The sandbox user is "agent" (HOME=/home/agent). We mount a
# temp .claude.json (host config + bypassPermissionsModeAccepted=true) and
# ~/.claude/.credentials.json into the container via -v flags. This skips
# onboarding, login, and bypass-permissions prompts.
#
# Technical note: Sandbox mode mounts the main repo's .git directory into the
# container so git worktree references resolve correctly.

set -e

# Find the repo root (works from any subdirectory)
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)
if [ -z "$REPO_ROOT" ]; then
  echo "Error: Not in a git repository"
  exit 1
fi

REPO_NAME=$(basename "$REPO_ROOT")
BASE_DIR=$(dirname "$REPO_ROOT")

# Parse arguments
ACTION=$1
NAME=$2
SPEC_PATH=""
USE_SANDBOX=false
USE_FRESH=false
INITIAL_PROMPT=""

# Parse remaining arguments (spec, --sandbox, -p can be in any order)
shift 2 2>/dev/null || true
while [ $# -gt 0 ]; do
  case $1 in
    --sandbox|-s)
      USE_SANDBOX=true
      ;;
    --fresh)
      USE_FRESH=true
      ;;
    -p|--prompt)
      shift
      INITIAL_PROMPT="$1"
      ;;
    *)
      # Assume it's the spec path if not a flag
      if [ -z "$SPEC_PATH" ]; then
        SPEC_PATH="$1"
      fi
      ;;
  esac
  shift
done

usage() {
  echo "Usage: claude-worktree {new|attach|done|list} [name] [spec] [--sandbox] [--fresh] [-p <prompt>]"
  echo ""
  echo "Commands:"
  echo "  new <name> [spec] [--sandbox] [--fresh] [-p <prompt>]"
  echo "                     Create worktree, tmux session, and start Claude"
  echo "                     If spec provided, sends /implement-spec automatically"
  echo "                     If -p provided, sends that prompt after Claude starts"
  echo "                     If --sandbox, runs in Docker (isolated container)"
  echo "  attach <name>      Attach to existing tmux session"
  echo "  done <name>        Remove worktree and kill tmux session"
  echo "  list               Show all worktrees and tmux sessions"
  echo ""
  echo "Options:"
  echo "  --sandbox, -s      Run Claude in Docker sandbox (AFK mode)"
  echo "                     - Isolated container (can't affect host machine)"
  echo "                     - Skips all permission prompts"
  echo "                     - Mounts .git dir so worktree git history works"
  echo "                     - Uses cached OAuth credentials from Docker volume"
  echo "  -p, --prompt       Send an initial prompt after Claude starts"
  echo "  --fresh            Run full bun install + submodule init instead of"
  echo "                     symlinking from main repo (use when branch changes deps)"
  echo ""
  echo "Environment Variables:"
  echo "  CLAUDE_GH_PAT      GitHub Personal Access Token for gh CLI in sandbox"
  echo "                     (passed as GH_TOKEN to Docker container)"
  echo ""
  echo "First-time sandbox setup:"
  echo "  1. docker sandbox run -t custom-claude-sandbox claude   # authenticate Claude via browser"
  echo "  2. export CLAUDE_GH_PAT=ghp_xxx # set GitHub PAT for gh CLI"
  echo ""
  echo "Examples:"
  echo "  claude-worktree new feature-auth"
  echo "  claude-worktree new payment-history specs/payments/payment-history.md"
  echo "  claude-worktree new export-pdf specs/invoices/export-pdf.md --sandbox"
  echo "  claude-worktree new api-key-ui --sandbox -p '/implement-spec @specs/mcp/api-key-ui.md'"
  echo "  claude-worktree attach payment-history"
  echo "  claude-worktree done payment-history"
}

case $ACTION in
  new)
    if [ -z "$NAME" ]; then
      echo "Error: Please provide a feature name"
      usage
      exit 1
    fi

    WORKTREE_DIR="$BASE_DIR/$REPO_NAME-$NAME"

    if [ -d "$WORKTREE_DIR" ]; then
      echo "Error: Directory already exists: $WORKTREE_DIR"
      exit 1
    fi

    # Validate spec file exists if provided
    if [ -n "$SPEC_PATH" ]; then
      if [ ! -f "$REPO_ROOT/$SPEC_PATH" ]; then
        echo "Error: Spec file not found: $SPEC_PATH"
        echo "  (looked for $REPO_ROOT/$SPEC_PATH)"
        exit 1
      fi
    fi

    # Always use worktree - it allows host interaction from other tmux panes
    echo "Creating worktree at $WORKTREE_DIR on branch '$NAME'..."
    git worktree add "$WORKTREE_DIR" -b "$NAME"

    # Speed optimization: symlink node_modules and Foundry libs from the
    # main repo instead of running bun install + git submodule update.
    # These are identical across worktrees (same lockfile, same submodule refs).
    # Use --fresh to skip this and do a real install (e.g. when branch changes deps).
    if [ "$USE_FRESH" = true ]; then
      echo "Fresh mode: running bun install and submodule init..."
      (cd "$WORKTREE_DIR" && bun install --frozen-lockfile)
      (cd "$WORKTREE_DIR" && git submodule update --init)
    elif [ "$USE_SANDBOX" = true ]; then
      # Docker can't follow symlinks outside the mounted dir — use bun install
      echo "Sandbox mode: running bun install (Docker can't follow host symlinks)..."
      (cd "$WORKTREE_DIR" && bun install --frozen-lockfile)
      # Submodules: copy instead of symlink for Docker compatibility
      if [ -d "$REPO_ROOT/packages/contracts/lib" ]; then
        mkdir -p "$WORKTREE_DIR/packages/contracts"
        cp -R "$REPO_ROOT/packages/contracts/lib" "$WORKTREE_DIR/packages/contracts/lib"
        echo "Copied packages/contracts/lib into worktree (sandbox mode)"
      fi
    else
      # Symlink root node_modules
      if [ -d "$REPO_ROOT/node_modules" ]; then
        ln -s "$REPO_ROOT/node_modules" "$WORKTREE_DIR/node_modules"
        echo "Linked node_modules → main repo (skips bun install)"
      else
        echo "Warning: No node_modules in main repo — run 'bun install' in main repo first"
      fi

      # Symlink Foundry libs (git submodules)
      if [ -d "$REPO_ROOT/packages/contracts/lib" ]; then
        mkdir -p "$WORKTREE_DIR/packages/contracts"
        ln -s "$REPO_ROOT/packages/contracts/lib" "$WORKTREE_DIR/packages/contracts/lib"
        echo "Linked packages/contracts/lib → main repo (skips submodule init)"
      fi
    fi

    # Symlink gitignored Claude settings into the worktree so it inherits
    # the accumulated permission grants from the main repo.
    #
    # .claude/settings.local.json is gitignored (user-specific permission
    # approvals). Without it, Claude Code starts with zero pre-approved
    # permissions and prompts for every tool use.
    #
    # We also link ~/.claude/projects/ so the worktree shares session memory
    # and trust state with the main repo.
    MAIN_LOCAL_SETTINGS="$REPO_ROOT/.claude/settings.local.json"
    WORKTREE_LOCAL_SETTINGS="$WORKTREE_DIR/.claude/settings.local.json"
    if [ -f "$MAIN_LOCAL_SETTINGS" ] && [ ! -e "$WORKTREE_LOCAL_SETTINGS" ]; then
      if [ "$USE_SANDBOX" = true ]; then
        # Docker can't follow symlinks outside the mounted dir — copy instead
        cp "$MAIN_LOCAL_SETTINGS" "$WORKTREE_LOCAL_SETTINGS"
        echo "Copied .claude/settings.local.json into worktree (sandbox mode)"
      else
        ln -s "$MAIN_LOCAL_SETTINGS" "$WORKTREE_LOCAL_SETTINGS"
        echo "Linked .claude/settings.local.json → main repo (inherits permissions)"
      fi
    fi

    CLAUDE_PROJECTS_DIR="$HOME/.claude/projects"
    MAIN_PROJECT_KEY=$(echo "$REPO_ROOT" | tr '/' '-')
    WORKTREE_PROJECT_KEY=$(echo "$WORKTREE_DIR" | tr '/' '-')
    if [ -d "$CLAUDE_PROJECTS_DIR/$MAIN_PROJECT_KEY" ] && [ ! -e "$CLAUDE_PROJECTS_DIR/$WORKTREE_PROJECT_KEY" ]; then
      ln -s "$CLAUDE_PROJECTS_DIR/$MAIN_PROJECT_KEY" "$CLAUDE_PROJECTS_DIR/$WORKTREE_PROJECT_KEY"
      echo "Linked Claude project context → main repo (shares memory + trust)"
    fi

    echo "Creating tmux session '$NAME'..."
    tmux new-session -d -s "$NAME" -c "$WORKTREE_DIR"

    # Start Claude (sandboxed or normal)
    if [ "$USE_SANDBOX" = true ]; then
      echo "Starting Claude Code in Docker sandbox..."
      echo "  📦 Isolated container (mounts host ~/.claude for auth)"

      # Build Docker args
      DOCKER_ARGS=""
      SANDBOX_HOME="/home/agent"

      # Create .claude.json with host config + bypass-permissions flag.
      # Written into the worktree (under /Users/) so Docker can access it —
      # macOS temp dirs (/var/folders/) aren't in Docker's shared paths.
      SANDBOX_CLAUDE_JSON="$WORKTREE_DIR/.claude/.sandbox-claude.json"
      if [ -f "$HOME/.claude.json" ]; then
        python3 -c "
import json, sys
with open(sys.argv[1]) as f:
    d = json.load(f)
d['bypassPermissionsModeAccepted'] = True
json.dump(d, sys.stdout)
" "$HOME/.claude.json" > "$SANDBOX_CLAUDE_JSON"
        echo "  🔑 Mounting .claude.json (skips onboarding + bypass prompt)"
        DOCKER_ARGS="$DOCKER_ARGS -v $SANDBOX_CLAUDE_JSON:$SANDBOX_HOME/.claude.json"
      fi

      # Mount OAuth credentials (skips login prompt)
      if [ -f "$HOME/.claude/.credentials.json" ]; then
        echo "  🔑 Mounting OAuth credentials (skips login)"
        DOCKER_ARGS="$DOCKER_ARGS -v $HOME/.claude/.credentials.json:$SANDBOX_HOME/.claude/.credentials.json"
      else
        echo "  ⚠️  No ~/.claude/.credentials.json found — sandbox will prompt for auth"
      fi

      # Mount the main repo's .git so worktree references work inside container
      echo "  📁 Mounting $REPO_ROOT/.git for worktree support"
      DOCKER_ARGS="$DOCKER_ARGS -v $REPO_ROOT/.git:$REPO_ROOT/.git"

      # Check for GH_TOKEN / CLAUDE_GH_PAT for gh CLI authentication
      GH_TOKEN_VAL="${CLAUDE_GH_PAT:-$GH_TOKEN}"
      if [ -n "$GH_TOKEN_VAL" ]; then
        echo "  🔑 GH_TOKEN will be passed for gh CLI authentication"
        DOCKER_ARGS="$DOCKER_ARGS -e GH_TOKEN=$GH_TOKEN_VAL"
      fi

      tmux send-keys -t "$NAME" "docker sandbox run -w $WORKTREE_DIR -t custom-claude-sandbox $DOCKER_ARGS claude" Enter

      # .sandbox-claude.json is cleaned up when worktree is removed via 'done'
    else
      # Start Claude interactively
      CLAUDE_CMD="claude --permission-mode acceptEdits"
      echo "Starting Claude Code..."
      tmux send-keys -t "$NAME" "$CLAUDE_CMD" Enter
    fi

    # Determine prompt to send (explicit -p takes precedence over spec path)
    PROMPT_TO_SEND=""
    if [ -n "$INITIAL_PROMPT" ]; then
      PROMPT_TO_SEND="$INITIAL_PROMPT"
    elif [ -n "$SPEC_PATH" ]; then
      PROMPT_TO_SEND="/implement-spec $SPEC_PATH"
    fi

    # Send prompt after Claude initializes
    if [ -n "$PROMPT_TO_SEND" ]; then
      if [ "$USE_SANDBOX" = true ]; then
        WAIT_SECONDS=20
      else
        WAIT_SECONDS=3
      fi
      echo "  Sending prompt in ${WAIT_SECONDS}s: $PROMPT_TO_SEND"
      sleep "$WAIT_SECONDS"
      tmux send-keys -t "$NAME" -l "$PROMPT_TO_SEND"
      sleep 1
      tmux send-keys -t "$NAME" Enter
    fi

    echo ""
    echo "✅ Done! Worktree and tmux session created: $NAME"
    if [ "$USE_SANDBOX" = true ]; then
      echo "  Mode: Sandbox (Docker isolated)"
    fi
    echo ""
    echo "  Attach with:  tmux attach -t $NAME"
    echo "  Or run:       claude-worktree attach $NAME"
    if [ -n "$PROMPT_TO_SEND" ]; then
      echo "  Prompt:       $PROMPT_TO_SEND (auto-sent)"
    fi
    echo ""
    ;;

  attach)
    if [ -z "$NAME" ]; then
      echo "Error: Please provide a session name"
      usage
      exit 1
    fi

    if ! tmux has-session -t "$NAME" 2>/dev/null; then
      echo "Error: No tmux session named '$NAME'"
      echo "Available sessions:"
      tmux list-sessions 2>/dev/null || echo "  (none)"
      exit 1
    fi

    tmux attach -t "$NAME"
    ;;

  done)
    if [ -z "$NAME" ]; then
      echo "Error: Please provide a feature name"
      usage
      exit 1
    fi

    WORKTREE_DIR="$BASE_DIR/$REPO_NAME-$NAME"

    echo "Cleaning up '$NAME'..."

    # Kill tmux session if it exists (this also stops any Docker sandbox running in it)
    if tmux has-session -t "$NAME" 2>/dev/null; then
      echo "Killing tmux session..."
      tmux kill-session -t "$NAME"
    else
      echo "No tmux session found (already closed)"
    fi

    # Remove symlinked dependencies before removing the worktree
    # (git worktree remove --force won't follow symlinks, but clean up anyway)
    if [ -L "$WORKTREE_DIR/node_modules" ]; then
      rm "$WORKTREE_DIR/node_modules"
    fi
    if [ -L "$WORKTREE_DIR/packages/contracts/lib" ]; then
      rm "$WORKTREE_DIR/packages/contracts/lib"
    fi

    # Remove Claude config files before removing the worktree
    WORKTREE_LOCAL_SETTINGS="$WORKTREE_DIR/.claude/settings.local.json"
    if [ -L "$WORKTREE_LOCAL_SETTINGS" ]; then
      rm "$WORKTREE_LOCAL_SETTINGS"
    fi
    rm -f "$WORKTREE_DIR/.claude/.sandbox-claude.json"
    CLAUDE_PROJECTS_DIR="$HOME/.claude/projects"
    WORKTREE_PROJECT_KEY=$(echo "$WORKTREE_DIR" | tr '/' '-')
    if [ -L "$CLAUDE_PROJECTS_DIR/$WORKTREE_PROJECT_KEY" ]; then
      rm "$CLAUDE_PROJECTS_DIR/$WORKTREE_PROJECT_KEY"
    fi

    # Remove worktree if it exists
    if [ -d "$WORKTREE_DIR" ]; then
      echo "Removing worktree..."
      git worktree remove "$WORKTREE_DIR" --force
    else
      echo "No worktree found at $WORKTREE_DIR"
    fi

    echo ""
    echo "Cleaned up: $NAME"
    echo ""
    echo "Note: Branch '$NAME' still exists. Delete with:"
    echo "  git branch -d $NAME    # if merged"
    echo "  git branch -D $NAME    # force delete"
    ;;

  list)
    echo "=== Git Worktrees ==="
    git worktree list
    echo ""
    echo "=== Tmux Sessions ==="
    tmux list-sessions 2>/dev/null || echo "(no active sessions)"
    echo ""
    echo "=== Docker Sandboxes ==="
    docker sandbox ls 2>/dev/null || echo "(no sandboxes)"
    ;;

  *)
    usage
    exit 1
    ;;
esac
