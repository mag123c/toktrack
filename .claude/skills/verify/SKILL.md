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
    └── On fail: fix and retry (notify user after 3 same failures)
```

## Commands
```bash
cargo test --quiet
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

## Self-Healing
- Fail → analyze error → fix code → retry
- Same error 3 times → notify user

## Rules
- Required before commit
- Order: test → clippy → fmt
- All must pass to proceed

## Next Step
On all pass → immediately call `/review`
