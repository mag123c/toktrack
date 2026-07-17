#!/bin/bash
# Track skill-chain state with marker files.
set -e
INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // ""')
SKILL_NAME=$(echo "$INPUT" | jq -r '.tool_input.skill // ""')
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')

CHAIN_DIR="/tmp/toktrack-chain-$SESSION_ID"
mkdir -p "$CHAIN_DIR"

if [ "$TOOL_NAME" = "Skill" ]; then
  case "$SKILL_NAME" in
    "implement")
      echo "implement" > "$CHAIN_DIR/current"
      rm -f "$CHAIN_DIR/verify-done" "$CHAIN_DIR/review-done"
      ;;
    "verify")
      echo "verify" > "$CHAIN_DIR/current"
      touch "$CHAIN_DIR/verify-started"
      ;;
    "review")
      echo "review" > "$CHAIN_DIR/current"
      touch "$CHAIN_DIR/review-started"
      ;;
    "wrap")
      echo "wrap" > "$CHAIN_DIR/current"
      touch "$CHAIN_DIR/wrap-started"
      ;;
  esac
fi

exit 0
