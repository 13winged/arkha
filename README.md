# ArkHa 1.3

[![Build](https://img.shields.io/github/actions/workflow/status/13winged/arkha/ci.yml)](https://github.com/13winged/arkha/actions)
[![Release](https://img.shields.io/github/v/release/13winged/arkha)](https://github.com/13winged/arkha/releases)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

ArkHa is a local AI secretary for Windows. It runs models through `llama.cpp`, keeps data on the local machine, and exposes file, window, memory, voice, document and skill tools behind explicit permissions.

## 1.3 highlights

- **Documents**: `docs.read`, `docs.info` and `docs.write` cover docx, pdf, xlsx and pptx through pinned ArtCraft command-line tools over MCP. Every archive and executable is verified by SHA-256; GridCraft is opened by ArkHa itself and exposed without path-taking tools.
- **Separate downloads**: document CLIs are installed from Settings → Documents with a progress bar. Tools never download on their own, so a missing CLI produces a clear refusal instead of a silent multi-minute wait.
- **ArtCraft-style interface**: square surfaces, flat panels, grouped sidebar, model status under the model, memory as discrete cells.
- **Customisable sidebar**: reorder items by dragging or with arrows, hide what you do not use, reset to the grouped default.
- **Observability**: token budget ring across the panel, live CPU/RAM/VRAM cards and a two-minute graph, plus a model speed test in the right dock.
- **OpenCode skills**: skills from `%USERPROFILE%\.agents\skills` appear in the Skills tab; external skills stay disabled until enabled one by one.
- **Glitch levels**: off, soft, normal and hard. Normal and hard stay horizontal, hard adds vertical streaks.

## 1.2 highlights

- Runtime Manager with CPU/CUDA/Vulkan packages, compatibility checks and SHA-256 installation.
- Local model library grouped by family and quantization.
- Skills Registry with enable/disable controls, permissions and `SKILL.md` viewer.
- Agent context builder with local RAG context, profile context and bounded tool history.
- Local attachments stored as SQLite metadata plus files under `%APPDATA%\ArkHa\attachments`.
- Text attachments are indexed into the local RAG corpus.

## 1.0 highlights

- Cyberpunk terminal UI shared by the main window, RAG and agent windows.
- Static model dock with model status, GPU placement, launch/stop controls and voice controls.
- Agent chat with persistent in-session history and visible tool trace.
- RAG corpus window for local search and document context.
- Agent modes: Plan, Think and Build, each filtering tools available to the model.
- Scoped file access, text reading and opening files with the Windows-associated application.
- Hidden console windows for `llama-server.exe`, `nvidia-smi` and voice services.
- WebView2 orphan cleanup and production CSP.
- Russian and English UI with persisted font-size preference.

## Requirements

- Windows 10 1809 or newer.
- Node.js 20+.
- Rust stable MSVC toolchain and Visual Studio C++ Build Tools.
- WebView2 Evergreen Runtime.
- A compatible `llama-server.exe` and GGUF model. They are not committed to the repository.
- Document support is optional: the ArtCraft CLIs are downloaded on demand from their signed GitHub releases.

## Development

```powershell
npm install
npm run app:dev
```

## Build

```powershell
npm run app:build
```

The release executable is written to `target/release/arkha.exe`. Installers are produced under `target/release/bundle/`.

## Verification

```powershell
npm run check
npm run test:rust
npm run check:i18n
```

The Rust library suite currently contains 374 passing tests and 3 ignored hardware-dependent tests. The i18n check scans every `.tsx` screen, not only the main one.

Attachments are stored locally. Binary files are not placed into SQLite; SQLite stores metadata and SHA-256 while file contents remain in the attachments directory.

## Permissions and safety

ArkHa starts conservatively. File access is limited to folders selected by the user. Risky operations such as writing files, launching programs or controlling windows are governed by the selected execution mode and approval policy. UI control uses accessibility references, not coordinate clicks.

Document tools follow the same rules: paths are checked before the CLI is started, the CLI receives only the file's own folder as a read or write root, and the pinned version plus both SHA-256 hashes are verified. GridCraft's own MCP server cannot confine folders, so ArkHa opens the workbook itself and never exposes path-taking tools to the model.

## Repository hygiene

Generated build output belongs in `target/` and `dist/`; both are ignored by Git. Temporary archives are placed in `trash/` and are also ignored. Models, local databases and downloaded engines are excluded from version control.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE). The ArtCraft command-line tools are third-party software under Apache-2.0/MIT; their licence files are copied alongside the downloaded binaries.
