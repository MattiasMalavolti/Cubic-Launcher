<div align="center"><img src="assets/icon.png" width="180" alt="Cubic Launcher" /></div>

# Cubic Launcher

**A rule-based Minecraft launcher and modlist manager.**

<p align="center">
  <a href="LICENSE"><img alt="License: GPLv3" src="https://img.shields.io/badge/License-GPLv3-blue.svg"></a>
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white">
  <img alt="SolidJS" src="https://img.shields.io/badge/SolidJS-1.9-2C4F7C?logo=solid&logoColor=white">
  <img alt="TypeScript" src="https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-backend-000000?logo=rust&logoColor=white">
  <img alt="Platform: Windows and Linux" src="https://img.shields.io/badge/Platform-Windows%20%7C%20Linux-7c3aed">
</p>

Build, maintain, and launch rule-driven Minecraft modlists with explicit choices, reusable local caches, and Modrinth integration.

## Dependency philosophy

**You manage dependencies.** When you add any mod—including a dependency another mod needs—the launcher always selects the latest release tagged for the exact Minecraft version in play. Declared dependency requirements appear as informational notices; Cubic Launcher never auto-downloads dependencies behind your back and never auto-excludes mods.

- Deterministic, user-controlled modlists.
- No hidden downloads or surprise version drift.

## Verified support

- Minecraft **1.6.4 → latest**
- **Vanilla, Fabric, Forge, and NeoForge**
- **Windows and Linux**

Support is validated by an automated launch matrix, including Linux launch runs.

## Java runtimes

Cubic Launcher manages Java runtimes automatically. If the exact required Java major is unavailable, it uses the nearest higher available major.

## Install

Download the latest build from [GitHub Releases](https://github.com/MattiasMala/Cubic-Launcher/releases). Cubic Launcher shows in-app update notifications and downloads and installs an update only after you confirm.

## Quick start for development

Requires **Rust**, **Node.js**, and the platform prerequisites for Tauri 2.

```sh
git clone https://github.com/MattiasMala/Cubic-Launcher.git
cd Cubic-Launcher
npm install
npm run tauri dev
```

## Contributing

See [Architecture and development guide](docs/ARCHITECTURE.md) for the data model, backend and frontend structure, storage, authentication, and release process.

## License

Cubic Launcher is distributed under the [GNU General Public License v3.0](LICENSE).
