#!/bin/bash
# PreToolUse hook: Block git force-push and other destructive git operations

# Read the tool input from stdin
COMMAND=$(jq -r '.tool_input.command // empty')

if [ -z "$COMMAND" ]; then
  exit 0
fi

# Block force push and destructive git commands
if echo "$COMMAND" | grep -qE '(git\s+push\s+.*--force|git\s+push\s+-f\b|git\s+reset\s+--hard|git\s+clean\s+-f)'; then
  jq -n '{
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: "BLOCKED: Destructive git command detected. Force push, hard reset, and force clean are forbidden. Use safer alternatives."
    }
  }'
else
  exit 0
fi
