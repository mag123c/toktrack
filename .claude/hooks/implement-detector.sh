#!/bin/bash
# Skill 호출 감지 → 마커 관리
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
      # plan-exited 마커 제거 (정상 흐름 완료)
      rm -f "$PLAN_EXITED_MARKER"
      rm -f "$CLARIFY_IN_PROGRESS_MARKER"
      ;;
    "clarify")
      # clarify 시작 → Plan Mode 필요 상태
      touch "$CLARIFY_IN_PROGRESS_MARKER"
      ;;
  esac
fi

exit 0
