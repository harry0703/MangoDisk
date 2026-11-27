<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk application icon"> MangoDisk
</h1>

<p align="center">Disk cleanup and storage analysis for <b>macOS</b>, <b>Windows</b>, and <b>Linux</b></p>

<p align="center">
  English · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a> · <a href="README.ru.md">Русский</a>
</p>

<p align="center">
  <a href="https://github.com/harry0703/MangoDisk/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/harry0703/MangoDisk?display_name=tag&sort=semver"></a>
  <img alt="macOS supported" src="https://img.shields.io/badge/macOS-supported-111827?logo=apple&logoColor=white">
  <img alt="Windows supported" src="https://img.shields.io/badge/Windows-supported-2563eb?logo=windows&logoColor=white">
  <img alt="Linux supported" src="https://img.shields.io/badge/Linux-supported-f59e0b?logo=linux&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri&logoColor=white">
  <img alt="Rust Core" src="https://img.shields.io/badge/core-Rust-b7410e?logo=rust&logoColor=white">
</p>

<p align="center">
  <a href="https://mangodisk.app/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-00-overview.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-00-overview.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-00-overview.jpg?ver=2.1.1" width="1200" alt="MangoDisk Overview dashboard">
    </picture>
  </a>
</p>

## What MangoDisk Can Do

> **Storage**

### 1. Deep Cleanup

Find caches, build artifacts, and local AI models left by apps and developer tools, even in hidden folders.

- **System and user caches**: Clear temporary system files, diagnostic data, and caches that can be rebuilt.
- **Application caches**: Clear caches, logs, update packages, and temporary files left by apps.
- **Browser data**: Clear caches and temporary web data from Chrome, Edge, Firefox, Brave, Arc, Opera, and other browsers.
- **Developer tools and Xcode**: Clear caches from package managers, IDEs, and compilers, as well as Xcode development data.
- **Container caches**: Remove unused build caches and other data that Docker and similar tools can regenerate.
- **Project build artifacts**: Clean up regenerable dependencies, caches, and build directories from Node.js, Rust, Gradle, Swift, Python, .NET, Godot, CMake, and other projects.
- **AI tools**: Remove caches, old logs, and temporary downloads from ChatGPT, Codex, Claude, and similar apps.
- **AI models**: Check how much space local models in Ollama, LM Studio, Hugging Face, and ComfyUI use. Choose which models to remove. You’ll need to download any deleted models you want to use later.
- **Application optimization**: Reduce the size of supported apps while keeping them usable.

Start with the recommended items or review them one by one. See how much space you can free before you clean.

### 2. Large File Cleanup

Find the files taking up the most space, from old installers to videos and archives, without digging through folders.

### 3. Duplicate File Cleanup

MangoDisk compares file contents to find exact duplicates, even when filenames differ. Smart selection keeps at least one copy in each group.

### 4. Disk Space Analysis

#### Space Distribution

See where your storage goes. Switch between a **treemap** and a **sunburst chart**, set the folder depth, and use the file list to drill down to the largest folders and files.

#### Space Changes

Wondering where your free space went? Turn on scan history and compare two scans of the same drive or folder to see what grew or shrank. Scan again after cleaning to check where you freed up space.

> **Privacy & Security**

### 5. Privacy Cleanup

Clear browsing and search history, cookies, recent items, and clipboard contents to remove traces of activity left by browsers, apps, and the system.

> **System Tools**

### 6. Application Uninstall and Cleanup

Remove apps along with their caches, settings, and leftover files. MangoDisk takes extra care with locations that may contain personal files.

### 7. Startup Item Management

Disable startup items you don’t need to shorten startup times and reduce background activity. You can enable them again whenever you need them.

### 8. System Optimization

Adjust system settings for better performance, more privacy, or a setup that suits the way you work.

### 9. System Maintenance

Fix common issues with search, icons, audio, or network connections without troubleshooting manually or running commands.

### 10. Context Menus (Windows)

Browse right-click menu entries by location or app. Disable, restore, or remove supported entries, and tailor menus to individual file types.

Add shortcuts for everyday actions such as copying a path, opening a terminal, or launching system tools.

> **Activity**

### 11. Operation History

Review past cleanups and system changes. See how much space you freed, which actions completed, and what still needs attention.

## Resource Usage and Memory Management

Track CPU, GPU, memory, network, and disk activity in real time. On supported hardware, check temperature, power use, battery status, and disk health.

Find and quit apps using too many resources, or free memory manually or automatically.

Keep an eye on your system from the menu bar, taskbar, system tray, or a floating desktop window. Choose which metrics to show and how they look.

## AI Explanations

Unsure what an item does or what changing it might affect? Ask for an explanation next to the item. It uses the item’s description and scan results to explain its purpose and what to consider before acting. Available for built-in Deep Cleanup rules, Privacy Cleanup, Startup Item Management, System Optimization, and System Maintenance.

## Safety and Rules

> [!IMPORTANT]
> **MangoDisk puts data safety first.**
> Cleanup rules and system optimizations ship only after their safe operating limits are defined and tested on real systems.

Scans are read-only by default. Before cleaning, deleting files, uninstalling apps, or changing system settings, MangoDisk shows what will change and asks for confirmation. Results are saved to Operation History.

System Optimization runs only built-in, tested settings. MangoDisk checks the setting after applying it and warns you beforehand about risks, administrator access, or a restart.

MangoDisk maintains its own cleanup rules. Third-party projects can suggest areas to investigate. Each new rule is checked against reliable sources and tested on real systems before inclusion.

The rules and their revision history are public. [Browse the MangoDisk cleanup rules](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules).

## Screenshots

<p align="center">
  <strong>Deep Cleanup</strong><br>
  <sub>Clear built-up caches, build artifacts, and AI models you no longer need</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-01-deep-cleanup.jpg?ver=2.1.1">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg?ver=2.1.1">
    <img src="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg?ver=2.1.1" width="1200" alt="MangoDisk Deep Cleanup interface">
  </picture>
</p>

<p align="center">
  <strong>Space Distribution</strong><br>
  <sub>See where your storage goes and find the largest files and folders</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-05-disk-space-analysis.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg?ver=2.1.1" width="1200" alt="MangoDisk Disk Space Analysis interface">
    </picture>
  </a>
</p>

<p align="center">
  <strong>Space Changes</strong><br>
  <sub>Compare two scans to see where disk usage changed</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" width="1200" alt="MangoDisk Space Changes view comparing two scans">
    </picture>
  </a>
</p>

<p align="center">
  <strong>System Monitoring</strong><br>
  <sub>CPU, memory, network, and disk activity at a glance</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/en/light-system-monitoring-memory.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-system-monitoring-memory.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-system-monitoring-memory.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-system-monitoring-memory.jpg?ver=2.1.1" width="720" alt="MangoDisk system monitoring and memory management">
    </picture>
  </a>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>Large File Cleanup</strong><br>
      <sub>Find the files taking up the most space</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-02-large-file-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk Large File Cleanup interface">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>Duplicate File Cleanup</strong><br>
      <sub>Remove exact duplicates and keep at least one copy per group</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-03-duplicate-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk Duplicate File Cleanup interface">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Startup Item Management</strong><br>
      <sub>Disable startup items you don’t need</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-06-startup-items.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg?ver=2.1.1" width="100%" alt="MangoDisk Startup Item Management interface">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>Application Uninstall and Cleanup</strong><br>
      <sub>Remove apps and the files they leave behind</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-04-app-uninstaller.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg?ver=2.1.1" width="100%" alt="MangoDisk Application Uninstaller interface">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>System Optimization</strong><br>
      <sub>Adjust system settings for performance, privacy, and everyday use</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-07-system-optimization.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg?ver=2.1.1" width="100%" alt="MangoDisk System Optimization interface">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>System Maintenance</strong><br>
      <sub>Fix common system issues</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-08-system-maintenance.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg?ver=2.1.1" width="100%" alt="MangoDisk System Maintenance interface">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Privacy Cleanup</strong><br>
      <sub>Clear activity traces left by browsers, apps, and the system</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-09-privacy-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk Privacy Cleanup interface">
        </picture>
      </a>
    </td>
  </tr>
</table>

## Before You Begin

> [!CAUTION]
>
> 1. Cleanup, permanent deletion, and uninstall operations may be irreversible. Check what you’ve selected and back up important data before continuing.
> 2. Check what an item does and what may change before running system maintenance or adjusting startup items or system settings.
> 3. Some system optimizations can affect security, privacy, battery life, or system updates.

## Desktop App

Download MangoDisk from the [official download page](https://mangodisk.app/download) or [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest), then follow the instructions for your operating system below.

### macOS

**Requirements:** macOS Monterey 12.5 or later.

**Install with Homebrew:**

```sh
brew install --cask harry0703/tap/mangodisk
```

**Manual installation:** Download the DMG from the [official download page](https://mangodisk.app/download), open it, and drag MangoDisk into the Applications folder.

### Windows

**Requirements:** 64-bit Windows 10 or later.

**Install with PowerShell:**

```powershell
irm https://get.mangodisk.app | iex
```

**Install with WinGet (official source):**

```powershell
winget install --id MangoDisk.MangoDisk --exact --source winget
```

**Manual installation:** Download the Windows installer from the [official download page](https://mangodisk.app/download) and follow the installation prompts.

### Linux

**Recommended:** Ubuntu 22.04 LTS or later, on x64 or ARM64.

Available as `.deb` packages and AppImages. Compatibility with other Linux distributions depends on their system libraries and desktop environment.

**Install with Homebrew:** Requires a recent Homebrew version and a desktop environment.

```sh
brew install --cask harry0703/tap/mangodisk
```

**Install from the terminal (Debian/Ubuntu):** This command detects your architecture and installs the latest matching `.deb` package.

```sh
curl -fsSL https://get.mangodisk.app/linux | bash
```

**Manual installation:** Choose the package for your architecture on the [official download page](https://mangodisk.app/download).

- **Debian/Ubuntu:** Install the `.deb` package for your architecture.
- **Other distributions:** Try the AppImage by making it executable, then running it.

## Command Line (CLI)

Run MangoDisk from a terminal or a script using the same cleanup engine as the desktop app.

### macOS

**Install with Homebrew:**

```sh
brew install harry0703/tap/mangodisk-cli
```

### Windows

**Install with PowerShell:**

```powershell
irm https://get.mangodisk.app/cli | iex
```

**Install with WinGet (official source):**

```powershell
winget install --id MangoDisk.CLI --exact --source winget
```

### Linux

Prebuilt standalone CLI downloads are not yet available for Linux. Follow the [Build from Source](#build-from-source) instructions below to build it yourself.

### Usage Examples

If `mangodisk` is not immediately available after installation, open a new terminal, then verify the installation:

```sh
mangodisk --version
```

Common commands:

```sh
# Scan and show cleanable content without changing anything
mangodisk clean

# Apply the same smart recommendations as the desktop application
mangodisk clean --apply

# Preview all selectable content without deleting anything
mangodisk clean --apply --selection all --dry-run

# Produce machine-readable JSON output
mangodisk clean --format json --no-progress
```

`mangodisk clean` scans in read-only mode by default. For cleanup in a non-interactive environment, pass `--yes` to confirm. Run the following command to see all options:

```sh
mangodisk clean --help
```

## Build from Source

### Prerequisites

- Node.js 24 LTS
- pnpm 11.13.1
- Stable Rust

For platform-specific dependencies, see the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/).

### Get the Source and Run the Desktop Application

```sh
git clone https://github.com/harry0703/MangoDisk.git
cd MangoDisk
pnpm install --frozen-lockfile
pnpm tauri:dev
```

### Run the Required Checks

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

### Build the Desktop Installer

```sh
pnpm tauri:build
```

### Build the CLI

```sh
pnpm cli:build
```

Local builds are for development and verification. Release signing, notarization, and update metadata are handled by the official release process.

## Support MangoDisk

MangoDisk is free, open source, and ad-free. The core features are free for personal and commercial use under GPL-3.0. To support development and use MangoDisk’s hosted AI, [become a supporter](https://mangodisk.app/supporter).

## Contributing

Issues, cleanup rules, fixes, and new features are welcome. Read [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`AGENTS.md`](AGENTS.md) before getting started.

Routine cleanup coverage should use build-validated, declarative TOML rules. See [`src-tauri/crates/mangodisk-core/rules/README.md`](src-tauri/crates/mangodisk-core/rules/README.md) for the rule schema, safety constraints, and validation instructions.

Before submitting changes, run at least:

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

Report security vulnerabilities privately through GitHub Security Advisories as described in [`SECURITY.md`](SECURITY.md). Do not open a public issue for a security vulnerability.

## Technology Stack

- [Tauri 2](https://tauri.app/): Desktop runtime and system integration
- [Rust](https://www.rust-lang.org/): Scanning, filesystem access, safety validation, and cleanup execution
- [Vue 3](https://vuejs.org/) and [TypeScript](https://www.typescriptlang.org/): Desktop user interface

## License

MangoDisk is open source under the [GNU General Public License v3.0](https://github.com/harry0703/MangoDisk/blob/main/LICENSE). Third-party components remain subject to their respective licenses.
