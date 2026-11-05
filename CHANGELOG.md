# Changelog

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
