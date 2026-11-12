# Changelog

## [1.3.1] - 2026-11-12

### Fixed

- Model downloads no longer stop around 1.2 GB. The request used a 120-second total timeout, which
  cut the body at roughly that size; it is replaced with an idle read timeout. Interrupted transfers
  now resume automatically, up to six attempts with growing pauses.

### Added

- Models: add an existing `.gguf` file by its full path, without downloading it again.
- Models outside the recipe catalog can be started: they use the profile's context and KV-cache,
  and their speed is measured on launch. A downloaded file no longer has to match a recipe name.
- Models: a **Test** button starts the model, measures speed on a short run and saves the recipe to
  `recipes.user.json`, so the model becomes usable without editing the catalog.
- Chat: each assistant message has a **Speak** button with speed modes 0.75×, 1×, 1.25× and 1.5×.
- Voice commands: speech recognized by push-to-talk is sent to the agent instead of only being shown as
  text. Enabling voice now arms the hotkey; disabling it removes the hook. App launching also looks in
  the Windows `App Paths` registry, so names like `chrome` resolve to their install folder.
- A Windows notification appears when an agent request finishes, so the result is visible even when the
  window is minimised or covered by another app.

### Changed

- Settings now has a Permissions panel: execution mode, approval mode and checkboxes for risky
  capabilities. Previously they could only be set during onboarding, so `apps.launch` stayed
  disabled with no way to enable it later.
- Permission changes apply immediately: the running agent uses the updated settings instead of a copy
  read at login.
- The approval prompt is a highlighted card with clear **allow** and **deny** buttons. A decision applies
  to one question only.
- A disabled capability or dry-run no longer asks for approval: the tool reports the real reason instead
  of the user denying a question that could not be executed anyway.
- A voice app launch does not stop at the approval dialog when `apps.launch` is enabled and the mode is
  not "always ask"; the spoken command is treated as the confirmation.
- Push-to-talk hotkey is now `Ctrl+G`; an old saved `Ctrl+Alt+M` is treated as the new default.
- App launching can find almost any installed program: Windows `App Paths`, Start Menu shortcuts
  (including `.lnk` launch) and a cached walk of the install folders.
- A toast appears when an agent request finishes, showing the answer or the stop reason.

### Fixed

- Observability was empty because nothing wrote to it. Chat now records a trace, a span and metrics
  (tokens used, prompt tokens, latency, tokens per second); average latency falls back to metrics.
- An approval decision was not cleared after it was used, so one "deny" was silently reused for the next
  call in the same run; the model saw two denials while the user answered once.

## [1.3.0] - 2026-11-12

### Added

- Documents through pinned ArtCraft command-line tools over MCP: `docs.read`, `docs.info` and
  `docs.write` for docx, pdf, xlsx and pptx.
- Separate CLI download with progress in Settings → Documents; tools never download on their own.
- Documents tab, OpenCode skills from `%USERPROFILE%\.agents\skills`, and a fully customisable
  sidebar with drag-and-drop reordering.
- Observability: token budget ring, live CPU/RAM/VRAM cards and a two-minute graph, plus a model
  speed test in the right dock.

### Changed

- ArtCraft-style surfaces: square corners, flat panels, grouped sidebar, model status under the
  model, memory shown as discrete cells.
- Glitch levels: off, soft, normal and hard; normal and hard keep the error horizontal, hard adds
  vertical streaks.

### Fixed

- CPU reading in observability, model selection for models without a recipe, stale quant selection
  after changing the repository, duplicated memory indicator, chat hint position and the white
  onboarding input.

## [1.3.0-18] - 2026-11-12

Тестовая сборка. Не релиз.

### Added

- OpenCode skills from `%USERPROFILE%\.agents\skills` appear in Skills; external skills are disabled
  by default and enabled one by one, so hundreds of descriptions do not flood the prompt.
- Observability: token budget ring across the full panel with limit, spent and remaining; live
  CPU/RAM/VRAM cards and a two-minute graph; CPU sampling moved to a background thread.

### Fixed

- Models without a recipe can now be selected; the missing recipe is reported when starting.
- Chat hint moved from the message list down above the input.
- Onboarding answer field no longer paints the browser-white input background.

## [1.3.0-17] - 2026-11-12

Тестовая сборка. Не релиз.

### Changed

- One selection language across the app: green frame, pulsing glow and a corner check badge on
  profile tiers, context chips, voice toggles, model rows, glitch buttons, onboarding choices and
  the wide-disk option.
- Errors share one look too: red frame, pulsing glow, an exclamation badge and a stepped
  broken-pixel grid animation.

## [1.3.0-16] - 2026-11-12

Тестовая сборка. Не релиз.

### Fixed

- Switching the repository now clears the file list and the selected file, so downloading cannot ask
  for a file that exists only in the previous repository. A missing selection is reported before the
  request is sent.

### Changed

- The chosen quant is explicit: pulsing green frame, `selected` badge, and a line with the selected
  file name above the download button.

## [1.3.0-15] - 2026-11-12

Тестовая сборка. Не релиз.

### Changed

- The speed test is called "Model speed test" and now carries a reminder under it: the model is
  chosen in the Models section, with a link that opens that window.

## [1.3.0-14] - 2026-11-12

Тестовая сборка. Не релиз.

### Fixed

- Observability showed no CPU: the frontend expected snake_case fields while the command serialises
  camelCase, and two snapshots are now taken around sysinfo's minimum interval so the value is real
  instead of zero. The duplicate command registration is gone.

### Added

- Speed test button under the model in the right dock: one short request to the running model,
  showing approximate tokens per second from llama-server timings (fallback: tokens over elapsed
  time).

## [1.3.0-13] - 2026-11-12

Тестовая сборка. Не релиз.

### Changed

- Removed the duplicated RAM/VRAM indicator from the top strip: memory lives only in the bottom-left
  sidebar. The top strip keeps the system label and is now slim.

## [1.3.0-12] - 2026-11-12

Тестовая сборка. Не релиз.

### Changed

- Removed the duplicated profile/model status from the top strip; it now sits in the right dock
  immediately under the model, with speed when the model runs.
- RAM and VRAM use six discrete squares instead of a bar.

## [1.3.0-11] - 2026-11-12

Тестовая сборка. Не релиз.

### Changed

- RAM and VRAM are two centred rows, each `label / bar / percent`.
- Glitch: normal and hard no longer move text vertically. Both keep horizontal RGB fringing to the
  right; hard is stronger (6 px, 0.7 s, saturated copies plus vertical streaks).

## [1.3.0-10] - 2026-11-12

Тестовая сборка. Не релиз.

### Fixed

- RAM and VRAM indicators collapsed because they reused the decorative `.bar` class, which has no
  width or fill. They now have their own bar and fill and wrap in the narrow sidebar.

### Changed

- Four glitch levels: off, soft, normal, hard. Normal is more visible than before, hard adds a
  vertical streak overlay and stronger chromatic split; the old `aggressive` value reads as hard.

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
