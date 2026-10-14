[English](/README.md) | [Русский](/README_RU.md)

<p align="center">
  <a href="https://github.com/13winged/arkha/releases"><img src="https://img.shields.io/github/v/release/13winged/arkha" alt="Release"></a>
  <a href="https://github.com/13winged/arkha/actions"><img src="https://img.shields.io/github/actions/workflow/status/13winged/arkha/ci.yml.svg" alt="Build"></a>
  <a href="#"><img src="https://img.shields.io/badge/Rust-1.85%2B-8B2BE6?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="#"><img src="https://img.shields.io/badge/React-19-61DAFB?style=flat-square&logo=react&logoColor=white" alt="React"></a>
  <a href="https://github.com/13winged/arkha/releases/latest"><img src="https://img.shields.io/github/downloads/13winged/arkha/total.svg" alt="Downloads"></a>
  <a href="https://www.apache.org/licenses/LICENSE-2.0"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg?longCache=true" alt="License"></a>
</p>

**ArkHa** is a local AI secretary for Windows. It runs entirely on your own machine — voice, access to your files and windows, documents, memory. No cloud, no telemetry, no account.

It is built as a Tauri 2 desktop app around `llama.cpp`, and it picks a model that fits your hardware automatically. A machine with 8 GB of RAM and an RTX 3060 gets a different model than one with 64 GB and an RTX 4090.

> [!IMPORTANT]
> ArkHa can read your files, control your windows and launch programs. It starts in **show-only** mode and asks before anything risky, but it is still software with real permissions. Read [What it may do](#what-it-may-do) before you grant access.

## Features

- **Honest capability registry** — every one of the 18 abilities carries the gate that verified it and the exact refusal it gives when it cannot do something. Unverified abilities are marked as such rather than quietly listed as working.
- **Self-choosing model** — the app detects CPU, RAM and VRAM, picks a profile from T1 to T5, and starts a model that fits. Nothing to configure by hand.
- **Access levels that mean something** — show-only by default, optional execution, and a choice of when to ask: risky actions only, everything, or nothing.
- **Voice you switch on yourself** — Qwen3-ASR for recognition, Qwen3-TTS for speech, through the same `llama-server`. Never turns itself on.
- **Window control without coordinates** — the agent finds elements through UI Automation and presses them **by their visible label**. Clicking blind pixels is not implemented and will not be.
- **Scoped file access** — pick individual folders or grant the whole disk, as an explicit separate decision rather than a folder list that silently means everything.
- **Integrated model browser** — type `gemma` and the whole family comes back with download counts. Filtered to GGUF, so everything listed can actually be downloaded and run.
- **A memory strip that tells the truth** — RAM and VRAM read live. If VRAM cannot be measured, it is hidden rather than guessed.
- **Memory-bar layout** — built after the inventory screen of a game HUD: flat, hairlines, cut corners, and colour used only where it means something.
- **2 interface languages** — Russian and English, switchable in one click, remembered between runs.

## Screenshots

<details>
<summary>Click to expand</summary>

The HUD is deliberately flat and low-contrast: a sterile surface with gold for action, red for danger and blue for data. The state of a thing is carried by weight and brightness, not by a palette — the same rule the reference interface uses to stay readable at low contrast.

</details>

## Quick Start

Download the installer from [releases](https://github.com/13winged/arkha/releases) and run it. Then press **Download** and fetch a model — the app will suggest one that fits your profile.

Building from source:

```bash
git clone https://github.com/13winged/arkha
cd arkha
npm install
npx tauri build --release
```

`llama.cpp` binaries are not in the repository — they are several hundred megabytes per architecture. The app looks for them next to the executable and in `data/bin`.

Tests: `cargo test --manifest-path src-tauri/Cargo.toml` (256 unit) · Gates run against live hardware and are started separately, see [Testing](#testing).

## How it decides what it may do

Three levels, and each one says plainly what it costs you.

| Level | What happens |
|---|---|
| **Show only** (`dry-run`) | The secretary writes what it is about to do and waits. Nothing happens without your word |
| **Ask for risky ones** (`actions`, default) | Safe actions run immediately; deleting, launching and overwriting files ask first |
| **Ask for nothing** (`never`) | It does everything allowed. A wrong command is not caught |

Filesystem access is a separate decision from execution: you choose either named folders or the whole disk, and the choice is shown as what it is. "Whole disk" is not smuggled in as `C:\` in a folder list.

## Supported hardware

**Minimum:** Windows 10 1809 or newer, 8 GB RAM. A GPU is not required.

**Profiles are picked automatically:**

| Profile | Needs | Typical model |
|---|---|---|
| **T1** | 8 GB RAM, no GPU | Qwen3.5 4B, 16K context |
| **T2** | 16 GB RAM | Qwen3.5 9B, 32K context |
| **T3** | 32 GB + 8 GB VRAM | Qwen3.5 9B, 64K context |
| **T4** | 32 GB + 12 GB VRAM | Qwen3.5 27B, 128K context |
| **T5** | 32 GB + 24 GB VRAM | Qwen3.5 27B, 256K context |

The table above is the plan. What your machine actually gets was measured on the development machine (Ryzen 7 8700F, 32 GB, RTX 5060 8 GB): **profile T3, Qwen3.5 9B Q5_K_M, all 33 layers on the GPU, 64K context**. Every number in `docs/10-замеры.md` comes from that machine and nowhere else.

## Voice

| Part | Model | Measured |
|---|---|---|
| Recognition | `Qwen3-ASR-0.6B-Q8_0` | **274 ms** per phrase, **143 MiB** in RAM |
| Speech | `Qwen3-TTS-1.7B-Q4_K_M` | **3.62×** real time, Apache-2.0 |

Recognition runs on the **processor, deliberately**. On the GPU it occupies 2594 MiB and answers in 86 ms; in RAM it occupies 143 MiB and answers in 274 ms. Giving back 2.4 GiB is worth 188 ms, especially when the model already has the card.

Both models are `Apache-2.0`, verified against the Hugging Face API on 14.10.2026. The GGUF packaging repositories declare no license of their own; see [docs/09-лицензии.md](docs/09-лицензии.md).

## Environment variables

| Variable | Description | Default |
|---|---|---|
| `ARKHA_BIN_DIR` | Folder holding `llama-server.exe` | searched near the executable |
| `ARKHA_MODELS_DIR` | Folder holding GGUF models | `%APPDATA%\ArkHa\models` |
| `ARKHA_BIN_DIR` / `ARKHA_MODELS_DIR` in tests | Required by gates that need a real model and a real engine | — |

## Supported Languages

The interface is available in **2** languages:

Русский · English

## Testing

Unit tests are fast and run anywhere:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Gates check the live machine — a real model, a real Chrome window, a real file on disk. They are the only place where claims like "works" mean anything:

| Gate | What it proves |
|---|---|
| `s3_gate` | Capability registry integrity |
| `s4_gate` | Onboarding collects settings correctly |
| `s6_gate` | File reading and skills |
| `s8_gate` | Window reading, launching and control on a live desktop |
| `uia_gate` | UI Automation tree and button press on a real Chrome |
| `vision_gate` | Image reading with a separate vision model |

## Contributing

Issues and pull requests are welcome. `cargo test --lib` and `npx tsc --noEmit` must stay green. Documents live in the repository under `docs/` — the Russian ones are the primary record and the English README mirrors them.

## Acknowledgment

Design of the interface follows the principles described in *UI Design in NieR:Automata* by Hisayoshi Kijima, in the [PlatinumGames official blog](https://www.platinumgames.com/official-blog/article/9624) — flat and systematic, colour used sparingly, state carried by weight rather than palette. No code was taken from it.

---

> 🇷🇺 Русская версия: [README_RU.md](./README_RU.md)