#!/bin/bash
# Track workflow markers when skills start.
set -e
INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // ""')
SKILL_NAME=$(echo "$INPUT" | jq -r '.tool_input.skill // ""')
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')

IMPLEMENT_STARTED_MARKER="/tmp/toktrack-implement-started-$SESSION_ID"
PLAN_EXITED_MARKER="/tmp/toktrack-plan-exited-$SESSION_ID"
CLARIFY_IN_PROGRESS_MARKER="/tmp/toktrack-clarify-in-progress-$SESSION_ID"

if [ "$TOOL_NAME" = "Skill" ]; then
  case "$SKILL_NAME" in
    "implement")
      touch "$IMPLEMENT_STARTED_MARKER"
      rm -f "$PLAN_EXITED_MARKER"
      rm -f "$CLARIFY_IN_PROGRESS_MARKER"
      ;;
    "clarify")
      touch "$CLARIFY_IN_PROGRESS_MARKER"
      ;;
  esac
fi

exit 0
