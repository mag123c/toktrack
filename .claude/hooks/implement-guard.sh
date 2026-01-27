#!/bin/bash
# Edit/Write 호출 전 implement 스킬 실행 여부 체크
# Plan Mode 종료 후 implement 스킬 없이 직접 코드 수정 시 차단
set -e
INPUT=$(cat)
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')

PLAN_EXITED_MARKER="/tmp/toktrack-plan-exited-$SESSION_ID"
IMPLEMENT_STARTED_MARKER="/tmp/toktrack-implement-started-$SESSION_ID"

# Plan Mode 종료됐는데 implement 스킬 미실행 상태면 차단
if [ -f "$PLAN_EXITED_MARKER" ] && [ ! -f "$IMPLEMENT_STARTED_MARKER" ]; then
  echo "Plan Mode 종료 후 /implement 스킬을 먼저 실행해야 합니다. TDD 워크플로우를 따르세요." >&2
  exit 2
fi

exit 0
