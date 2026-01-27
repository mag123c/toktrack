#!/bin/bash
# Plan Mode 종료(ExitPlanMode) 감지 → 상태 파일 생성
set -e
INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // ""')
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')
MARKER="/tmp/toktrack-plan-completed-$SESSION_ID"

# ExitPlanMode 도구 호출 감지
if [ "$TOOL_NAME" = "ExitPlanMode" ]; then
  touch "$MARKER"
fi

# 항상 허용
echo '{"decision": "allow"}'
