#!/bin/bash
# PreToolUse hook: Block destructive Docker commands
# Returns JSON with permissionDecision "deny" to block the tool call

# Read the tool input from stdin
COMMAND=$(jq -r '.tool_input.command // empty')

if [ -z "$COMMAND" ]; then
  exit 0
fi

# Block destructive Docker commands
if echo "$COMMAND" | grep -qE '(docker\s+rm\b|docker\s+compose\s+rm|docker\s+system\s+prune|docker\s+container\s+prune|docker\s+volume\s+prune|docker\s+image\s+prune)'; then
  jq -n '{
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: "BLOCKED: Destructive Docker command detected. This project forbids: docker rm, docker compose rm, docker system prune, docker container prune, docker volume prune, docker image prune."
    }
  }'
else
  exit 0
fi
