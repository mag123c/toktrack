#!/bin/bash
set -eu
HOOK_DIR="$(cd -- "$(dirname -- "$0")" && pwd)"
exec python3 "$HOOK_DIR/../../.claude/hooks/edit-target-guard.py"
