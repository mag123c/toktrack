#!/bin/bash
# Remind the next workflow phase on UserPromptSubmit.
set -e
INPUT=$(cat)
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')

CHAIN_DIR="/tmp/toktrack-chain-$SESSION_ID"
[ ! -d "$CHAIN_DIR" ] && exit 0

CURRENT=$(cat "$CHAIN_DIR/current" 2>/dev/null || echo "")

case "$CURRENT" in
  "implement")
    if [ ! -f "$CHAIN_DIR/verify-started" ]; then
      cat << 'EOF'
{
  "hookSpecificOutput": {
    "hookEventName": "UserPromptSubmit",
    "additionalContext": "## Skill Chain: /verify 필요\n\n`/implement` 완료 후 반드시 `/verify` 호출.\n**사용자 확인 없이 즉시 실행하세요.**"
  }
}
EOF
      exit 0
    fi
    ;;
  "verify")
    if [ ! -f "$CHAIN_DIR/review-started" ]; then
      cat << 'EOF'
{
  "hookSpecificOutput": {
    "hookEventName": "UserPromptSubmit",
    "additionalContext": "## Skill Chain: /review 필요\n\n`/verify` 통과 후 반드시 `/review` 호출.\n**사용자 확인 없이 즉시 실행하세요.**"
  }
}
EOF
      exit 0
    fi
    ;;
  "review")
    if [ ! -f "$CHAIN_DIR/wrap-started" ]; then
      cat << 'EOF'
{
  "hookSpecificOutput": {
    "hookEventName": "UserPromptSubmit",
    "additionalContext": "## Skill Chain: /wrap 필요\n\n`/review` PASS 후 반드시 `/wrap` 호출.\n**사용자 확인 없이 즉시 실행하세요.**"
  }
}
EOF
      exit 0
    fi
    ;;
esac

echo '{}'
