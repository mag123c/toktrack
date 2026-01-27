#!/bin/bash
# 플랜이 직접 제공되었을 때 /implement 스킬 사용 강제
set -e
INPUT=$(cat)
PROMPT=$(echo "$INPUT" | jq -r '.prompt // ""')

# 플랜 제공 패턴 감지:
# 1. "plan" 또는 "플랜" 키워드
# 2. "implement" 또는 "구현" 키워드
# 3. 마크다운 헤더 (##, ###) 또는 체크박스 (- [ ])
HAS_PLAN_KEYWORD=$(echo "$PROMPT" | grep -iE "(plan|플랜)" || true)
HAS_IMPL_KEYWORD=$(echo "$PROMPT" | grep -iE "(implement|구현)" || true)
HAS_STRUCTURE=$(echo "$PROMPT" | grep -E "(^#{1,3} |^- \[)" || true)

# 플랜 키워드 + (구현 키워드 또는 구조화된 내용)이 있으면 플랜 제공으로 간주
if [[ -n "$HAS_PLAN_KEYWORD" && (-n "$HAS_IMPL_KEYWORD" || -n "$HAS_STRUCTURE") ]]; then
  cat << 'EOF'
{
  "hookSpecificOutput": {
    "hookEventName": "UserPromptSubmit",
    "additionalContext": "## 플랜 직접 제공 감지 - /implement 실행 필수\n\n플랜이 직접 제공되었습니다. CLAUDE.md 규칙에 따라 반드시 `/implement` 스킬을 **첫 액션**으로 호출하세요.\n\n**금지**: 스킬 호출 없이 바로 코드 수정\n**필수**: Skill tool → /implement → TDD 사이클"
  }
}
EOF
else
  echo '{}'
fi
