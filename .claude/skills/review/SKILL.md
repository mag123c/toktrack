---
name: review
description: 부정적 관점 코드 리뷰 (컨벤션, 품질, 버그, 성능)
required_context:
  - .claude/ai-context/architecture.md
  - .claude/ai-context/conventions.md
---

# Review

## Chain (MUST)
```
/clarify → Plan Mode → /implement → /verify → /review → /wrap
                                              ^^^^^^^^
                                              현재 단계
```
| 이전 | 현재 | 다음 (자동 호출) |
|------|------|------------------|
| `/verify` | `/review` | `/wrap` (PASS 시 즉시) |

## Purpose
구현 완료 후 **부정적 관점**에서 검토. 문제를 찾는 것이 목표.

## Checklist

### 1. Convention Violations
- [ ] CLAUDE.md 규칙 위반
- [ ] 네이밍 컨벤션 위반
- [ ] 아키텍처 레이어 위반

### 2. Code Quality
- [ ] 불필요한 복잡성
- [ ] 중복 코드
- [ ] 매직 넘버/문자열
- [ ] 에러 핸들링 누락

### 3. Potential Bugs
- [ ] 엣지 케이스 미처리
- [ ] Off-by-one 에러
- [ ] Null/None 체크 누락
- [ ] 경쟁 조건

### 4. Performance
- [ ] 불필요한 할당/복사
- [ ] O(n²) 이상 복잡도
- [ ] 캐시 미활용

### 5. Test Coverage
- [ ] 누락된 테스트 케이스
- [ ] 경계값 테스트
- [ ] 에러 경로 테스트

## Output
```markdown
## Review Result

### Issues Found
1. [CRITICAL/WARNING/INFO] 설명

### Recommendations
- 권장 수정사항

### Verdict
- [ ] PASS: 커밋 가능 → /wrap 자동 호출
- [ ] FAIL: 수정 필요 → /implement로 복귀
```

## Rules
- 문제를 찾는 것이 목적 (칭찬 금지)
- 발견된 문제는 수정 후 재검증
- FAIL 시 /implement로 돌아가 수정

## NEXT STEP (자동 실행)
- **PASS**: **사용자 확인 없이 즉시** `/wrap` 호출
- **FAIL**: `/implement`로 복귀하여 수정 후 체인 재시작
