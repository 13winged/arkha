# ArkHa 1.0

[![Build](https://img.shields.io/github/actions/workflow/status/13winged/arkha/ci.yml)](https://github.com/13winged/arkha/actions)
[![Release](https://img.shields.io/github/v/release/13winged/arkha)](https://github.com/13winged/arkha/releases)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

ArkHa is a local AI secretary for Windows. It runs models through `llama.cpp`, keeps data on the local machine, and exposes file, window, memory, voice and skill tools behind explicit permissions.

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

The Rust library suite currently contains 328 passing tests and 2 ignored hardware-dependent tests.

## Permissions and safety

ArkHa starts conservatively. File access is limited to folders selected by the user. Risky operations such as writing files, launching programs or controlling windows are governed by the selected execution mode and approval policy. UI control uses accessibility references, not coordinate clicks.

## Repository hygiene

Generated build output belongs in `target/` and `dist/`; both are ignored by Git. Temporary archives are placed in `trash/` and are also ignored. Models, local databases and downloaded engines are excluded from version control.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
