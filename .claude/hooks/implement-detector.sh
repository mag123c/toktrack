#!/bin/bash
# Skill(implement) 호출 감지 → implement-started 마커 생성
set -e
INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // ""')
SKILL_NAME=$(echo "$INPUT" | jq -r '.tool_input.skill // ""')
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')

IMPLEMENT_STARTED_MARKER="/tmp/toktrack-implement-started-$SESSION_ID"
PLAN_EXITED_MARKER="/tmp/toktrack-plan-exited-$SESSION_ID"

# Skill 도구에서 implement 스킬 호출 감지
if [ "$TOOL_NAME" = "Skill" ] && [ "$SKILL_NAME" = "implement" ]; then
  touch "$IMPLEMENT_STARTED_MARKER"
  # plan-exited 마커 제거 (정상 흐름 완료)
  rm -f "$PLAN_EXITED_MARKER"
fi

exit 0
