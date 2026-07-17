#!/bin/bash
# Warn when code edits bypass the clarify-to-implement workflow.
set -e
INPUT=$(cat)
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // "unknown"')
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // ""')

PLAN_EXITED_MARKER="/tmp/toktrack-plan-exited-$SESSION_ID"
IMPLEMENT_STARTED_MARKER="/tmp/toktrack-implement-started-$SESSION_ID"
CLARIFY_IN_PROGRESS_MARKER="/tmp/toktrack-clarify-in-progress-$SESSION_ID"

CODE_EXTENSIONS="rs|ts|tsx|js|jsx|py|go|java|kt|swift|c|cpp|h|hpp"

is_code_file() {
  echo "$1" | grep -qE "\.($CODE_EXTENSIONS)$"
}

if ! is_code_file "$FILE_PATH"; then
  exit 0
fi

# Warn without blocking while clarification is still in progress.
if [ -f "$CLARIFY_IN_PROGRESS_MARKER" ]; then
  echo "⚠️ /clarify 후 Plan Mode 진행 권장 (코드 파일 수정)" >&2
  exit 0
fi

# Warn without blocking when implementation has not started.
if [ -f "$PLAN_EXITED_MARKER" ] && [ ! -f "$IMPLEMENT_STARTED_MARKER" ]; then
  echo "⚠️ 코드 구현은 /implement 스킬 사용 권장 (TDD 워크플로우)" >&2
  exit 0
fi

exit 0
