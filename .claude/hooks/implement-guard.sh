#!/bin/bash
# Edit/Write 호출 전 워크플로우 체크
# 1. Plan Mode 종료 후 implement 스킬 없이 직접 코드 수정 시 차단
# 2. clarify 진행 중 Plan Mode 없이 직접 코드 수정 시 차단
set -e
INPUT=$(cat)
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')

PLAN_EXITED_MARKER="/tmp/toktrack-plan-exited-$SESSION_ID"
IMPLEMENT_STARTED_MARKER="/tmp/toktrack-implement-started-$SESSION_ID"
CLARIFY_IN_PROGRESS_MARKER="/tmp/toktrack-clarify-in-progress-$SESSION_ID"

# clarify 진행 중인데 Plan Mode 없이 직접 수정 시도 시 차단
if [ -f "$CLARIFY_IN_PROGRESS_MARKER" ]; then
  echo "/clarify 후 반드시 Plan Mode(EnterPlanMode)를 거쳐야 합니다." >&2
  exit 2
fi

# Plan Mode 종료됐는데 implement 스킬 미실행 상태면 차단
if [ -f "$PLAN_EXITED_MARKER" ] && [ ! -f "$IMPLEMENT_STARTED_MARKER" ]; then
  echo "Plan Mode 종료 후 /implement 스킬을 먼저 실행해야 합니다. TDD 워크플로우를 따르세요." >&2
  exit 2
fi

exit 0
