#!/bin/bash
# Plan Mode 종료(ExitPlanMode) 감지 → 상태 파일 생성
set -e
INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // ""')
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')

PLAN_EXITED_MARKER="/tmp/toktrack-plan-exited-$SESSION_ID"
IMPLEMENT_STARTED_MARKER="/tmp/toktrack-implement-started-$SESSION_ID"

# ExitPlanMode 도구 호출 감지
if [ "$TOOL_NAME" = "ExitPlanMode" ]; then
  touch "$PLAN_EXITED_MARKER"
  # implement 마커 초기화 (새로운 plan cycle 시작)
  rm -f "$IMPLEMENT_STARTED_MARKER"
fi

exit 0
