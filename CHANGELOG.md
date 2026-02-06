# Changelog

## [0.1.41](https://github.com/mag123c/toktrack/compare/v0.1.40...v0.1.41) (2026-02-06)


### Features

* **tui:** improve spike colors, tab order, and daily view UX ([0b01efa](https://github.com/mag123c/toktrack/commit/0b01efa96b94974feebbae7fb84b19b1d8ab59ff))
* **tui:** visual spike detection in daily view cost column ([75cf377](https://github.com/mag123c/toktrack/commit/75cf377f38aa5faeeac96e4abd60e4c5fd8511f9)), closes [#46](https://github.com/mag123c/toktrack/issues/46)

## [0.1.40](https://github.com/mag123c/toktrack/compare/v0.1.39...v0.1.40) (2026-02-05)


### Bug Fixes

* **tui:** color-separate model count - primary=accent, count=muted ([5354a12](https://github.com/mag123c/toktrack/commit/5354a12ad93fc5126e4212ab37af5ed75a9a992c))
* **tui:** filter zero-token models in breakdown popup and daily list ([fb56e3c](https://github.com/mag123c/toktrack/commit/fb56e3cf75f0e799872b033a94d4b49bdddd3f8f))

## [0.1.39](https://github.com/mag123c/toktrack/compare/v0.1.38...v0.1.39) (2026-02-05)


### Bug Fixes

* **tui:** update popup styling - LightGreen selection, two-line hints ([85a175b](https://github.com/mag123c/toktrack/commit/85a175bf2efab097ccd041fcab871c8f524f9323))

## [0.1.38](https://github.com/mag123c/toktrack/compare/v0.1.37...v0.1.38) (2026-02-05)


### Bug Fixes

* **tui:** QA feedback - footer keys, help, quit colors, cache migration ([8adf0cd](https://github.com/mag123c/toktrack/commit/8adf0cd0cb9b72eaae319222c05054f2130763ce))
* **tui:** QA Round 2 - quit popup, daily columns, model display ([2ecee53](https://github.com/mag123c/toktrack/commit/2ecee532eafa431e3dd3065d1c387b589c7947f4))

## [0.1.37](https://github.com/mag123c/toktrack/compare/v0.1.36...v0.1.37) (2026-02-05)


### Features

* **tui:** add display_name() for human-readable model names ([7b21c19](https://github.com/mag123c/toktrack/commit/7b21c19e24386d5fddad36e4cbe04f18441586e4))
* **tui:** add model breakdown popup for Daily tab ([6b8ac4a](https://github.com/mag123c/toktrack/commit/6b8ac4ac2cb7015faf059c40bdcc62fd9ca750a3))


### Bug Fixes

* **parser:** use message-level model in Gemini parser ([d58d119](https://github.com/mag123c/toktrack/commit/d58d11946d07b71a3c3f43f7770b869873f69e5e))
* **tui:** remove q/Esc quit triggers, use Ctrl+C only ([0a24603](https://github.com/mag123c/toktrack/commit/0a246030338081344bdc15c50d301f515355fea0))


### Refactoring

* **services:** add DataLoaderService to consolidate data loading ([9d42fa3](https://github.com/mag123c/toktrack/commit/9d42fa31cfd5829de310be4943740eb69bac9624))

## [0.1.36](https://github.com/mag123c/toktrack/compare/v0.1.35...v0.1.36) (2026-02-05)


### Features

* **tui:** add quit confirmation popup ([21a0f12](https://github.com/mag123c/toktrack/commit/21a0f12b6377fd4b0e0a80207b604038ad17627a))

## [0.1.35](https://github.com/mag123c/toktrack/compare/v0.1.34...v0.1.35) (2026-02-05)


### Features

* **services:** add model normalizer and source usage aggregation ([ad1eab8](https://github.com/mag123c/toktrack/commit/ad1eab814a238e39f69aae9beff55a511c277dae))


### Refactoring

* **assets:** replace logo with pixel-style SVG ([f8addcd](https://github.com/mag123c/toktrack/commit/f8addcdf3fb29f67a7148ec86ab6986040f22ee4))

## [0.1.34](https://github.com/mag123c/toktrack/compare/v0.1.33...v0.1.34) (2026-02-05)


### Bug Fixes

* **parser:** use XDG standard path for OpenCode data directory ([9f48c99](https://github.com/mag123c/toktrack/commit/9f48c993837faaf84b4c7fd8f814173ec3de3232))

## [0.1.33](https://github.com/mag123c/toktrack/compare/v0.1.32...v0.1.33) (2026-02-05)


### Features

* **parser:** add OpenCode CLI support ([cd503da](https://github.com/mag123c/toktrack/commit/cd503da77623fa9e57b9c512c661131a47007e18))


### Bug Fixes

* **docs:** correct performance stack description (simd-json + rayon, not ratatui) ([dd80b13](https://github.com/mag123c/toktrack/commit/dd80b13e1d3ef2e0f90bdd5f6b10cf78b7ecf12a))
* **parser:** use Unix timestamp for OpenCode time.created ([4d0e28f](https://github.com/mag123c/toktrack/commit/4d0e28fb8696455ea01c89c2bef022d25520186e))

## [0.1.32](https://github.com/mag123c/toktrack/compare/v0.1.31...v0.1.32) (2026-02-04)


### Bug Fixes

* **docs:** improve README with 1000x performance highlight and data preservation warning ([b2c81b5](https://github.com/mag123c/toktrack/commit/b2c81b587feeb24b65ece15a54bd76138c41c592))

## [0.1.31](https://github.com/mag123c/toktrack/compare/v0.1.30...v0.1.31) (2026-02-03)


### Features

* **tui:** add dim overlay for update popups ([dfa13ba](https://github.com/mag123c/toktrack/commit/dfa13babc1d88f8a8a37b4116013213e8b5095d2))

## [0.1.30](https://github.com/mag123c/toktrack/compare/v0.1.29...v0.1.30) (2026-02-03)


### Bug Fixes

* **docs:** trigger release for readme updates ([25840e8](https://github.com/mag123c/toktrack/commit/25840e82a92091924c3eae62dcedfd9aa35475d9))

## [0.1.29](https://github.com/mag123c/toktrack/compare/v0.1.28...v0.1.29) (2026-02-02)


### Bug Fixes

* **ci:** prefer post-merge tag in release outputs and remove crates.io badge ([b25e303](https://github.com/mag123c/toktrack/commit/b25e303da3f7cd058d09774852d92e2676a1968e))

## [0.1.28](https://github.com/mag123c/toktrack/compare/v0.1.27...v0.1.28) (2026-02-02)


### Bug Fixes

* **ci:** handle GITHUB_TOKEN merge not triggering new workflows ([46a1a82](https://github.com/mag123c/toktrack/commit/46a1a82cd395a5541e961ac70bce4c73fb8f1943))

## [0.1.27](https://github.com/mag123c/toktrack/compare/v0.1.26...v0.1.27) (2026-02-02)


### Features

* **tui:** add theme auto-detection and responsive daily columns ([d2c162a](https://github.com/mag123c/toktrack/commit/d2c162a72c75bc5a6a0fe029c7becd1e6cbe4f48))

## [0.1.26](https://github.com/mag123c/toktrack/compare/v0.1.25...v0.1.26) (2026-02-01)


### Documentation

* **conventions:** remove insta snapshot reference ([087419a](https://github.com/mag123c/toktrack/commit/087419a42c26289145b3a061fcca8d26eaa37ff1))

## [0.1.25](https://github.com/mag123c/toktrack/compare/v0.1.24...v0.1.25) (2026-02-01)


### Documentation

* **perf:** update benchmark numbers to current measurements ([8570f9c](https://github.com/mag123c/toktrack/commit/8570f9cfeccf9b7aa373b6289672e9a01b965705))

## [0.1.24](https://github.com/mag123c/toktrack/compare/v0.1.23...v0.1.24) (2026-02-01)


### Documentation

* **seo:** optimize repo metadata and README for discoverability ([2660ba5](https://github.com/mag123c/toktrack/commit/2660ba5e051fa44f06467aca026b82443e1f4ab4))

## [0.1.23](https://github.com/mag123c/toktrack/compare/v0.1.22...v0.1.23) (2026-01-31)


### Documentation

* add logo and center-align README header ([d95b5fc](https://github.com/mag123c/toktrack/commit/d95b5fcd527fe3846bec93ea0822fe5db41019bc))

## [0.1.22](https://github.com/mag123c/toktrack/compare/v0.1.21...v0.1.22) (2026-01-30)


### Documentation

* **skills:** add adaptive depth routing to clarify skill ([2c8d030](https://github.com/mag123c/toktrack/commit/2c8d0301e6dac83ccfc5e58e9ee276702107e55b))

## [0.1.21](https://github.com/mag123c/toktrack/compare/v0.1.20...v0.1.21) (2026-01-30)


### Bug Fixes

* **parser:** use last cumulative total_token_usage for Codex sessions ([db3b1ca](https://github.com/mag123c/toktrack/commit/db3b1cabf587bd994f2eadbf19f93e5f7bf6459a))

## [0.1.20](https://github.com/mag123c/toktrack/compare/v0.1.19...v0.1.20) (2026-01-30)


### Documentation

* add development motivation tagline to all READMEs ([5d2b1f4](https://github.com/mag123c/toktrack/commit/5d2b1f4f4c85f1b42f0a56757a899ff19384a664))

## [0.1.19](https://github.com/mag123c/toktrack/compare/v0.1.18...v0.1.19) (2026-01-30)


### Documentation

* overhaul README with badges, architecture diagram, and updated benchmarks ([b204492](https://github.com/mag123c/toktrack/commit/b20449235b7f8a2a63b1edcf524d1ecb364ad326))

## [0.1.18](https://github.com/mag123c/toktrack/compare/v0.1.17...v0.1.18) (2026-01-29)


### Features

* **tui:** replace update popup key hints with arrow-select UI ([48ab39c](https://github.com/mag123c/toktrack/commit/48ab39c19642294f69383baa1defd750cfb07765))

## [0.1.17](https://github.com/mag123c/toktrack/compare/v0.1.16...v0.1.17) (2026-01-29)


### Features

* **cli:** add weekly/monthly subcommands, TUI-first dispatch ([5a6ef44](https://github.com/mag123c/toktrack/commit/5a6ef44df5d249eb4425de854f5ba202fc14698c))


### Performance

* **services:** cache-first loading pipeline for TUI and CLI ([04cd85e](https://github.com/mag123c/toktrack/commit/04cd85ea09a963ee5287dbafbf10df883936399a))

## [0.1.16](https://github.com/mag123c/toktrack/compare/v0.1.15...v0.1.16) (2026-01-29)


### Features

* **tui:** move update notification to in-TUI overlay popup ([f646a55](https://github.com/mag123c/toktrack/commit/f646a5576b666d936e2680cea791b86eda88ae88))

## [0.1.15](https://github.com/mag123c/toktrack/compare/v0.1.14...v0.1.15) (2026-01-29)


### Bug Fixes

* **ci:** use workflow_call instead of workflow_dispatch for release chain ([6d36aa1](https://github.com/mag123c/toktrack/commit/6d36aa1f67ef1cc385d99fd6b92081a00eedcdab))

## [0.1.14](https://github.com/mag123c/toktrack/compare/v0.1.13...v0.1.14) (2026-01-29)


### Documentation

* **docs:** update CI/CD docs, README, and add blog draft ([493d2e9](https://github.com/mag123c/toktrack/commit/493d2e982fa41c44225320d0b2a3b7439dccb574))

## [0.1.13](https://github.com/mag123c/toktrack/compare/v0.1.12...v0.1.13) (2026-01-29)


### Bug Fixes

* **ci:** trigger release workflow from release-please and fix cross build ([81f88b2](https://github.com/mag123c/toktrack/commit/81f88b2d1df779668be140c1708af91597ccf173))

## [0.1.12](https://github.com/mag123c/toktrack/compare/v0.1.11...v0.1.12) (2026-01-29)


### Features

* **services:** add npm update checker for TUI startup ([247a8d7](https://github.com/mag123c/toktrack/commit/247a8d74d3157761bc96f78acf035689607bc7d4))
* **tui:** add weekly/monthly view modes to Daily tab ([45eab98](https://github.com/mag123c/toktrack/commit/45eab98a5dd5cd7c2d9b7802bcfb7a2fbabe201a))


### Documentation

* **npm:** add README and expand keywords for npm discovery ([b0acdb1](https://github.com/mag123c/toktrack/commit/b0acdb1510b63cd165582c1610767d0d56ee0e9d))

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
