#!/bin/bash
# Plan Mode 완료 후 /implement 스킬 사용 권장
set -e
INPUT=$(cat)
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')
MARKER="/tmp/toktrack-plan-exited-$SESSION_ID"
IMPLEMENT_MARKER="/tmp/toktrack-implement-started-$SESSION_ID"

# Plan 완료 마커가 없거나, 이미 implement 시작했으면 종료
[ ! -f "$MARKER" ] && exit 0
[ -f "$IMPLEMENT_MARKER" ] && exit 0

cat << 'EOF'
{
  "hookSpecificOutput": {
    "hookEventName": "UserPromptSubmit",
    "additionalContext": "## Plan Mode 완료 - /implement 실행 필수\n\nPlan이 승인되었습니다. 반드시 `/implement` 스킬을 사용하여 TDD 방식(RED→GREEN→REFACTOR)으로 구현을 진행하세요.\n\n```\n/implement\n```"
  }
}
EOF
