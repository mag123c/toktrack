# Changelog

## [0.1.11](https://github.com/mag123c/toktrack/compare/v0.1.10...v0.1.11) (2026-01-28)


### Documentation

* **docs:** convert AI context to English, remove redundant comments ([53ceb52](https://github.com/mag123c/toktrack/commit/53ceb5270c60cf9137a0f45233040ab3ff20eb8c))

## [0.1.10](https://github.com/mag123c/toktrack/compare/v0.1.9...v0.1.10) (2026-01-28)


### Bug Fixes

* **ci:** support manual release build for specific tags ([eae4972](https://github.com/mag123c/toktrack/commit/eae4972fb01e319b2b0056c4379647bd4b0bd64f))

## [0.1.9](https://github.com/mag123c/toktrack/compare/v0.1.8...v0.1.9) (2026-01-28)


### Bug Fixes

* **ci:** add workflow_dispatch trigger to release workflow ([8429f69](https://github.com/mag123c/toktrack/commit/8429f69b79d39204f7ef0370f9bb5bb63e132455))

## [0.1.8](https://github.com/mag123c/toktrack/compare/v0.1.7...v0.1.8) (2026-01-28)


### Bug Fixes

* **ci:** trigger release workflow on GitHub release events ([cc5a8c3](https://github.com/mag123c/toktrack/commit/cc5a8c3ffe8ccd45065d42340dd9780972c39c97))

## [0.1.7](https://github.com/mag123c/toktrack/compare/v0.1.6...v0.1.7) (2026-01-28)


### Bug Fixes

* **parser:** restore ParserRegistry in TUI and filter synthetic models ([aa6abc3](https://github.com/mag123c/toktrack/commit/aa6abc328638d1adca84e16de8bc971dcda7ac1f))

## [0.1.6](https://github.com/mag123c/toktrack/compare/v0.1.5...v0.1.6) (2026-01-28)


### Features

* **tui:** display version in loading screen and help popup ([af33c6a](https://github.com/mag123c/toktrack/commit/af33c6a9ca0341fe574c4fdfb5706e0cc8854fbc))

## [0.1.5](https://github.com/mag123c/toktrack/compare/v0.1.4...v0.1.5) (2026-01-28)


### Bug Fixes

* **tui:** reverse daily view sort order and initial scroll position ([cd137db](https://github.com/mag123c/toktrack/commit/cd137dbe53ead5e5fa6f82981d0ed2b32f155e47))

## [0.1.4](https://github.com/mag123c/toktrack/compare/v0.1.3...v0.1.4) (2026-01-28)


### Features

* add workflow skills and hooks ([5942686](https://github.com/mag123c/toktrack/commit/59426864d69604056eab4134025f2d306cbad4ed))
* **parser:** add CLIParser trait and ClaudeCodeParser ([b0ea9a6](https://github.com/mag123c/toktrack/commit/b0ea9a697c8d5b6f41daa30e99df449340da829e))
* **parser:** add multi-CLI support (Codex + Gemini) ([#12](https://github.com/mag123c/toktrack/issues/12)) ([dae499e](https://github.com/mag123c/toktrack/commit/dae499e04da775791d3a34d6a5678f7a35c34633))
* **parser:** CLIParser trait and ClaudeCodeParser implementation ([ecaad8b](https://github.com/mag123c/toktrack/commit/ecaad8b020f0517636856e4213099a269cd69ee1))
* **skills:** add /next skill for session start ([1bf5b25](https://github.com/mag123c/toktrack/commit/1bf5b25599a89885ee4badcc126579caf7a0ef82))
* **tui:** complete TUI implementation with CLI commands and npm wrapper ([#11](https://github.com/mag123c/toktrack/issues/11)) ([09ff79c](https://github.com/mag123c/toktrack/commit/09ff79c6b1aaeba3950025690b6f529a9a17df2d))
* **tui:** improve UI responsiveness and alignment ([53f838f](https://github.com/mag123c/toktrack/commit/53f838fe45171cb376ce69d01ffd02cd91050274))


### Bug Fixes

* **ci:** extract PR number from release-please JSON output ([ac3d6c7](https://github.com/mag123c/toktrack/commit/ac3d6c7a78d3faf8299b867aa0bc50ecbc3b5095))
* **deps:** use rustls-tls for cross-compile compatibility ([1213118](https://github.com/mag123c/toktrack/commit/1213118dc738148a3e9bd6d936d90c9a406239a0))
* **tui:** use ANSI 256 colors and center-align heatmap ([98fc984](https://github.com/mag123c/toktrack/commit/98fc984dd6320eb866d3290d3c60d74bee15f950))


### Performance

* **parser:** parser benchmark and optimization ([#10](https://github.com/mag123c/toktrack/issues/10)) ([c1fe0e9](https://github.com/mag123c/toktrack/commit/c1fe0e962f856dce5b359f5b5e5a3bf26bd30f18))


### Documentation

* add CI/CD workflow documentation ([0ec07c7](https://github.com/mag123c/toktrack/commit/0ec07c783fdafcaa0a223ffb717474c7df8c7027))
* add commit rules for AI co-authorship ([f5ab12c](https://github.com/mag123c/toktrack/commit/f5ab12c40125c30755db1e649bdf7934e7eada35))
* add planning doc update to wrap workflow ([39c1dcf](https://github.com/mag123c/toktrack/commit/39c1dcfaa537f3db79824fe088b87fe8c025c2b2))
* add workflow auto-transition rules ([ecf96c5](https://github.com/mag123c/toktrack/commit/ecf96c53b54385d5aed66b99fc92d52cc3f1b809))
* add wrap and ai-context update to workflow rules ([868bdb8](https://github.com/mag123c/toktrack/commit/868bdb82df5a2311bebcc774c74683f4a9cfc0fe))
* **architecture:** add file_pattern to CLIParser trait ([4456c43](https://github.com/mag123c/toktrack/commit/4456c430b4b412eb505d7951076ba5639bd23a8b))
* slim down CLAUDE.md to entry point ([51af3f4](https://github.com/mag123c/toktrack/commit/51af3f43837cb85c07a96d8acaa26a058f190fbf))
* update ai-context and CLAUDE.md with workflow ([26fe00b](https://github.com/mag123c/toktrack/commit/26fe00b898a9ab1f1a80e63915d10ba78ffab0c1))

## [0.1.3](https://github.com/mag123c/toktrack/compare/toktrack-v0.1.2...toktrack-v0.1.3) (2026-01-28)


### Features

* **tui:** improve UI responsiveness and alignment ([53f838f](https://github.com/mag123c/toktrack/commit/53f838fe45171cb376ce69d01ffd02cd91050274))

## [0.1.2](https://github.com/mag123c/toktrack/compare/toktrack-v0.1.1...toktrack-v0.1.2) (2026-01-28)


### Features

* add workflow skills and hooks ([5942686](https://github.com/mag123c/toktrack/commit/59426864d69604056eab4134025f2d306cbad4ed))
* **parser:** add CLIParser trait and ClaudeCodeParser ([b0ea9a6](https://github.com/mag123c/toktrack/commit/b0ea9a697c8d5b6f41daa30e99df449340da829e))
* **parser:** add multi-CLI support (Codex + Gemini) ([#12](https://github.com/mag123c/toktrack/issues/12)) ([dae499e](https://github.com/mag123c/toktrack/commit/dae499e04da775791d3a34d6a5678f7a35c34633))
* **parser:** CLIParser trait and ClaudeCodeParser implementation ([ecaad8b](https://github.com/mag123c/toktrack/commit/ecaad8b020f0517636856e4213099a269cd69ee1))
* **skills:** add /next skill for session start ([1bf5b25](https://github.com/mag123c/toktrack/commit/1bf5b25599a89885ee4badcc126579caf7a0ef82))
* **tui:** complete TUI implementation with CLI commands and npm wrapper ([#11](https://github.com/mag123c/toktrack/issues/11)) ([09ff79c](https://github.com/mag123c/toktrack/commit/09ff79c6b1aaeba3950025690b6f529a9a17df2d))


### Bug Fixes

* **deps:** use rustls-tls for cross-compile compatibility ([1213118](https://github.com/mag123c/toktrack/commit/1213118dc738148a3e9bd6d936d90c9a406239a0))
* **tui:** use ANSI 256 colors and center-align heatmap ([98fc984](https://github.com/mag123c/toktrack/commit/98fc984dd6320eb866d3290d3c60d74bee15f950))


### Performance

* **parser:** parser benchmark and optimization ([#10](https://github.com/mag123c/toktrack/issues/10)) ([c1fe0e9](https://github.com/mag123c/toktrack/commit/c1fe0e962f856dce5b359f5b5e5a3bf26bd30f18))


### Documentation

* add commit rules for AI co-authorship ([f5ab12c](https://github.com/mag123c/toktrack/commit/f5ab12c40125c30755db1e649bdf7934e7eada35))
* add planning doc update to wrap workflow ([39c1dcf](https://github.com/mag123c/toktrack/commit/39c1dcfaa537f3db79824fe088b87fe8c025c2b2))
* add workflow auto-transition rules ([ecf96c5](https://github.com/mag123c/toktrack/commit/ecf96c53b54385d5aed66b99fc92d52cc3f1b809))
* add wrap and ai-context update to workflow rules ([868bdb8](https://github.com/mag123c/toktrack/commit/868bdb82df5a2311bebcc774c74683f4a9cfc0fe))
* **architecture:** add file_pattern to CLIParser trait ([4456c43](https://github.com/mag123c/toktrack/commit/4456c430b4b412eb505d7951076ba5639bd23a8b))
* slim down CLAUDE.md to entry point ([51af3f4](https://github.com/mag123c/toktrack/commit/51af3f43837cb85c07a96d8acaa26a058f190fbf))
* update ai-context and CLAUDE.md with workflow ([26fe00b](https://github.com/mag123c/toktrack/commit/26fe00b898a9ab1f1a80e63915d10ba78ffab0c1))
