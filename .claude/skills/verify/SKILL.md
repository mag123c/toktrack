---
name: verify
description: Self-healing verification loop (test → clippy → fmt)
required_context: []
---

# Verify

## Flow
```
cargo test → cargo clippy → cargo fmt --check
    │            │              │
    └── 실패 시 수정 후 재검증 (3회 실패 시 사용자 알림)
```

## Commands
```bash
cargo test --quiet
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

## Self-Healing
- 실패 → 에러 분석 → 코드 수정 → 재검증
- 동일 에러 3회 반복 시 사용자에게 알림

## Rules
- 커밋 전 필수 실행
- 순서: test → clippy → fmt
- 모두 통과해야 다음 단계 진행
