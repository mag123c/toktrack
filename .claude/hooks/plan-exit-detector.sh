#!/bin/bash
# Plan Mode 관련 도구 감지 → 상태 파일 관리
set -e
INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // ""')
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')

PLAN_EXITED_MARKER="/tmp/toktrack-plan-exited-$SESSION_ID"
IMPLEMENT_STARTED_MARKER="/tmp/toktrack-implement-started-$SESSION_ID"
CLARIFY_IN_PROGRESS_MARKER="/tmp/toktrack-clarify-in-progress-$SESSION_ID"

case "$TOOL_NAME" in
  "EnterPlanMode")
    # Plan Mode 진입 → clarify 완료 상태
    rm -f "$CLARIFY_IN_PROGRESS_MARKER"
    ;;
  "ExitPlanMode")
    # Plan Mode 종료 → implement 필요 상태
    touch "$PLAN_EXITED_MARKER"
    # implement 마커 초기화 (새로운 plan cycle 시작)
    rm -f "$IMPLEMENT_STARTED_MARKER"
    ;;
esac

exit 0
