# Changelog

## [1.3.0-9] - 2026-11-12

Тестовая сборка. Не релиз.

### Fixed

- Dragging menu items now works: Tauri's native drag-and-drop handler intercepted HTML5 dragging
  inside the WebView. Disabled it for the main window, since ArkHa does not accept dropped files.

## [1.3.0-8] - 2026-11-12

Тестовая сборка. Не релиз.

### Added

- Menu items can be reordered by dragging, including moving an item between the top zone and the
  bottom zone next to the agent mode. Arrow buttons remain for precise moves.

## [1.3.0-7] - 2026-11-12

Тестовая сборка. Не релиз.

### Changed

- Download tab no longer repeats the local model library: it only downloads models, the library
  stays in the Models window.
- Engine and runtime panels moved from Download into Settings, together with font size, sound and
  glitch; the former Profile tab is now Settings and sits at the bottom, above the agent mode.
- Capabilities moved to the bottom under Settings.
- Local user panel removed: the account is created before login anyway.
- Onboarding moved into the Secretary tab and restyled as flat cards.
- Sidebar order and visibility are user-configurable (Settings → Menu), stored locally; a reset
  restores the grouped default.

## [1.3.0-6] - 2026-11-12

Тестовая сборка. Не релиз.

### Changed

- Sidebar is grouped by meaning: work, system, setup, observation.
- New tabs: Documents (CLI downloads) and Appearance (font size, sound, glitch), split out of
  Settings.
- Agent mode select now explains the current mode in one line; the duplicate agent window entry is
  gone.
- RAG moved down next to the agent mode as a separate field.
- Model dock heading is left-aligned again.
- Removed the footer hint about 1–8 tabs: no such shortcuts existed.

## [1.3.0-5] - 2026-11-12

Тестовая сборка. Не релиз.

### Changed

- RAG and agent windows moved from the footer to the left sidebar, without emoji.
- Whole interface follows the ArtCraft-style surface: square corners, hairline borders, small
  uppercase section labels with a single green marker instead of glowing headings.
- Glitch is horizontal again, netrunner-style: narrow bands jump between rows in discrete steps with
  chromatic split; smoother than the first version but no flicker storm.

## [1.3.0-4] - 2026-11-12

Тестовая сборка для проверки документов. Не релиз.

### Changed

- Documents panel follows the ArtCraft card layout: thin border, small uppercase kicker with
  version, size and state, description, action at the bottom. Other ArtCraft apps use the same grid.
- Section headings inside panels are small uppercase labels separated by a rule.
- Glitch effect is vertical and quieter: narrow left and right columns instead of horizontal slices,
  vertical displacement instead of sideways shake, longer cycles. Aggressive mode dropped from
  5 px / 0.2 s to 2 px / 1.2 s, subtle from 1 px / 5 s to 1 px / 8 s.

## [1.3.0-3] - 2026-11-12

Тестовая сборка для проверки документов. Не релиз.

### Added

- Every document CLI now says what it is: WordCraft is a Word counterpart, PdfCraft an Acrobat
  counterpart, GridCraft an Excel counterpart, DeckCraft a PowerPoint counterpart.
- Catalog of the other ArtCraft applications with their purpose, plus a one-click download of the
  official ArtCraft Launcher installer into Downloads.

### Verification

- 372 Rust library tests pass; 3 hardware-dependent tests remain ignored.
- ArtCraft Launcher 0.19.2 downloaded by the app, size and SHA-256 matched the pinned values.

## [1.3.0-2] - 2026-11-11

Тестовая сборка для проверки документов. Не релиз.

### Added

- `docs.read`, `docs.info`, `docs.write`: reading and writing docx, pdf, xlsx and pptx through
  pinned ArtCraft command-line tools over MCP.
- `documents::install`: finds an installed CLI or downloads the pinned version, verifying the
  archive and executable by SHA-256, and extracts only the executable and licence files.
- Separate download in Settings → Documents: per-CLI version, size, button and progress events.
  Tools never download on their own; a missing CLI produces a refusal that names the app, version
  and size.
- GridCraft is opened by ArkHa itself and exposed without path-taking tools, because its MCP server
  cannot confine folders.

### Verification

- 370 Rust library tests pass; 3 hardware-dependent tests remain ignored.
- Live gates: handshake and `tools/list` on all four CLIs; sandbox rejection on WordCraft,
  PdfCraft and DeckCraft; docx write-read round trip; xlsx and pptx reading; overwrite refusal;
  outside-path refusal; download with 50 progress events and a matching full size.

## [1.2.3-1] - 2026-11-06

### Fixed

- Moved the sidebar collapse control above the language switch at the bottom of the left dock.
- Kept the language switch visible in both expanded and collapsed sidebar states.
- Moved the current profile and logout control to the bottom of the model dock.

### Verification

- TypeScript, i18n and Windows NSIS/MSI production builds pass.

## [1.2.2] - 2026-11-06

### Added

- Local account list on the login screen with password-confirmed account deletion.
- Account deletion removes that user's sessions and private data directory while preserving shared models and runtimes.

### Fixed

- Secondary application windows reuse the current in-process session and no longer request another login.

### Verification

- 348 Rust library tests pass; 3 hardware-dependent tests remain ignored.
- Selected integration gates: 56 pass; 8 model/network-dependent tests are ignored or skip without local assets. `uia_gate` remains unverified in the current Chrome accessibility environment.
- TypeScript, i18n and Windows NSIS/MSI production builds pass.

## [1.2.1] - 2026-11-05

### Added

- Local account registration, password login and isolated guest sessions.
- Explicit legacy-data import with category preview, conflict protection and SQLite online backup.
- Legacy import can be claimed by only one account; shared models, runtimes and package recipes are excluded.
- Windows DPAPI-backed Remember Me, shared across ArkHa windows and expiring after 30 days.
- Per-account settings, memory, RAG, attachments, permissions, profiles, audio preferences and user skills.
- Protection against reading another account's files or the unclaimed legacy data through filesystem, RAG and voice commands.

### Changed

- Shared models and runtimes remain machine-wide; models cannot be deleted and runtime versions cannot be overwritten or deleted from an account.
- Legacy browser preferences are only imported after the user selects them in the migration screen.

### Fixed

- Secondary ArkHa windows now reuse the active in-process session instead of prompting for another login.

### Verification

- 347 Rust library tests pass; 3 hardware-dependent tests remain ignored.
- Selected integration gates: 56 pass; 8 model/network-dependent tests are ignored or skip without local assets. `uia_gate` remains unverified because Chrome did not expose page content to the accessibility tree in this environment.
- TypeScript, i18n and Windows NSIS/MSI production builds pass.

## [1.2.0] - 2026-11-05

### Added

- Finalized Skills Registry, permissions confirmation and `SKILL.md` viewing.
- Agent context builder with RAG and attachment metadata.
- Local attachments for chat and agent requests with SQLite metadata, SHA-256 and indexed text.
- Agent timeout, cancellation and expanded trace events.
- Model family grouping, quantization selection and safe model deletion.
- Separate Model Library window.

### Verification

- 333 Rust tests pass; 3 hardware-dependent tests remain ignored.
- TypeScript, Vite and Tauri production builds pass.

## [1.2.0-beta] - 2026-11-04

### Added

- Agent context builder runs before every model turn.
- Tool-call/tool-result pairs remain intact during context compaction.
- Agent events: `started`, `context_ready`, `tool_result` and `error`.
- Expanded agent trace state for UI diagnostics.

### Verification

- 332 Rust tests pass; 3 hardware-dependent tests remain ignored.
- TypeScript, Vite and Tauri production builds pass.

## [1.2.0-alpha] - 2026-11-04

### Added

- Manifest-aware Skills Registry for legacy and directory-based skills.
- Skills list, enable and disable commands with persistent settings.
- Skills management panel in the settings screen.
- Enabled skills are filtered before entering the agent tool registry.
- Skill metadata includes source, version, tools and permissions.

### Verification

- Full Rust library tests and TypeScript/production build pass.

## [1.1.0] - 2026-11-04

### Added

- Persistent SQLite application settings with legacy browser-setting migration.
- Runtime Manager with installed runtime registry and active runtime selection.
- Runtime compatibility checks for executables, required files and CUDA.
- Managed runtime packages with reveal and safe deletion controls.
- HTTPS runtime ZIP installer with SHA-256 verification and atomic installation.
- Manifest-based runtime metadata for CPU, CUDA and Vulkan backends.
- Launcher integration with the selected runtime executable and library paths.

### Verification

- 331 Rust library tests pass; 3 hardware-dependent tests remain ignored.
- TypeScript, Vite and Tauri production builds pass.

## [1.0.0] - 2026-11-03

### Added

- Unified cyberpunk interface across the main, RAG and agent windows.
- Persistent right-side model dock with model diagnostics and voice controls.
- Persistent font-size preference applied during startup.
- Combined profile and UI/audio settings screen.
- Persistent secretary chat history while switching tabs.
- `fs.open` tool for opening permitted files with their Windows-associated application.
- Production Content Security Policy for Tauri WebView2.

### Fixed

- Hidden console windows for model, voice and NVIDIA helper processes.
- White child windows for RAG and agent mode selection.
- Orphaned WebView2 crashpad handlers.
- Missing i18n entry for the reasoning setting.

### Verification

- TypeScript and Vite production build pass.
- 328 Rust library tests pass; 2 hardware-dependent tests remain ignored.
