# Shared Claude and Codex workflow

Read the personal `~/.claude/workflow/contract.md` and current phase `~/.claude/skills/<phase>/SKILL.md` when installed. Repository requirements remain applicable. Without personal files, use this portable contract.

- Establish scope, constraints, and acceptance from the request. A clear direct prompt replaces a separate clarify call. Ask only material unresolved decisions. Investigation or design requests do not authorize coding.
- Record a PLAN for complex or cross-session work; a small bounded change can use conversation and diff. No particular Plan Mode or slash-command history is required.
- Use a feature branch/worktree and preserve unrelated edits. Check actual file destinations, including other worktrees. Do not edit main/master directly.
- Implement → verify → review → wrap within the authorized scope. Preserve repository TDD for implementation: meaningful behavior tests first. Documentation-only changes need relevant document/config validation rather than invented behavior tests.
- Read architecture and conventions. Preserve graceful partial-source failure, ToktrackError, English code comments/test names, and no agent attribution.
- Run `make check` (fmt, clippy, tests) before committing, as required by AGENTS.md. Add focused checks for changed hooks. Skipped tests, mock-only coverage, and unavailable runtime checks remain explicitly unverified.
- Review the complete task diff from its correct base, including committed/pending changes and relevant untracked files. Use independent review for safety boundaries; state when independence is unavailable.
- Fix confirmed defects within scope, rerun affected checks, and review the incremental fixes. Review cannot turn a missing test result into a pass.
- Wrap reconciles requirements, evidence, gaps, and necessary documentation. Only perform authorized commit/push/PR actions; do not auto-merge, deploy, or remove worktrees.
- Follow repository commit rules and current PR conventions. Report the actual remote revision/status after publication; a branch push is not a release.
