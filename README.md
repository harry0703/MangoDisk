<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk application icon"> MangoDisk
</h1>

<p align="center">Disk cleanup, storage analysis, privacy protection, and system optimization for macOS and Windows</p>

<p align="center">
  English · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  <a href="https://github.com/harry0703/MangoDisk/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/harry0703/MangoDisk?display_name=tag&sort=semver"></a>
  <img alt="macOS supported" src="https://img.shields.io/badge/macOS-supported-111827?logo=apple&logoColor=white">
  <img alt="Windows supported" src="https://img.shields.io/badge/Windows-supported-2563eb?logo=windows&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri&logoColor=white">
  <img alt="Rust Core" src="https://img.shields.io/badge/core-Rust-b7410e?logo=rust&logoColor=white">
</p>

<p align="center">
  <a href="https://mangodisk.app/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/readme/en-dark.jpg">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/readme/en-light.jpg">
      <img src="https://assets.mangodisk.app/images/readme/en-light.jpg" width="1200" alt="MangoDisk disk cleanup, storage analysis, privacy protection, and system optimization">
    </picture>
  </a>
</p>

## What MangoDisk Can Do

> **Storage**

### 1. Deep Cleanup

Find cleanable content scattered across the system, applications, developer tools, and local projects in one scan. MangoDisk saves you from checking each location manually and groups the results by reclaimable space:

- **System and user caches**: Reclaim space taken up over time by system temporary files, diagnostic data, and rebuildable caches.
- **Application caches**: Keep application caches, logs, update packages, and temporary content from quietly consuming more and more storage.
- **Browser data**: Reclaim space used by cached and temporary web data from Chrome, Edge, Firefox, Brave, Arc, Opera, and other browsers.
- **Developer tools and Xcode**: Quickly recover substantial storage used by package managers, IDEs, compiler caches, and Xcode development data.
- **Container caches**: Free up space used by inactive build caches and rebuildable data from Docker and other container tools.
- **Project build artifacts**: Recover space used by rebuildable dependencies, caches, and build directories across Node.js, Rust, Gradle, Swift, Python, .NET, Godot, CMake, and other projects.
- **AI models and caches**: Quickly spot large local AI models, download caches, and temporary transfer files.
- **Application optimization**: Shrink supported applications without affecting normal use, leaving more room on your disk.

Smart recommendations help you make safe choices quickly. You can also review items individually and see the estimated reclaimable space upfront, keeping every cleanup predictable and under your control.

### 2. Large File Cleanup

Quickly find the largest files and reclaim space used by old installers, videos, archives, and other bulky content without digging through folders one by one.

### 3. Duplicate File Cleanup

Reclaim space taken up by duplicate copies without treating files as duplicates just because they share a name. Smart selection keeps at least one file in every group, so cleanup stays effortless and safe.

### 4. Disk Space Analysis

See where your storage is going at a glance. Drill down through a treemap and list to locate the largest folders and files instead of cleaning blindly.

> **Privacy & Security**

### 5. Privacy Cleanup

Keep browsing history, searches, cookies, recent items, and clipboard data from lingering on your computer. Clear traces left by browsers, applications, and the system to reduce exposure of your activity and make everyday privacy easier to manage.

> **System Tools**

### 6. Application Uninstall and Cleanup

Uninstall applications and clear related caches, settings, and leftovers so removing an application actually gives you the space back. Potential personal files are handled cautiously to reduce the risk of accidental deletion.

### 7. Startup Item Management

Reduce unnecessary startup delays and background resource use, so your computer starts faster and feels lighter. Turn items back on at any time when you need them again.

### 8. System Optimization

Reduce unnecessary settings that slow down your system or get in the way. Balance performance, privacy, and personal preferences so your computer feels faster and easier to use.

### 9. System Maintenance

Fix common problems like missing search results, incorrect icons, no sound, or network connection failures—without hunting down fixes or typing complex commands. Get your computer back to normal sooner.

> **Activity**

### 10. Operation History

Keep a clear record of every cleanup and system change. See how much space you recovered, what completed successfully, and whether anything still needs your attention.

## AI Explanations

> Available since version 1.1.0

Unsure what an item does or what might happen if you change it? AI explanations use the item's description and current scan results to explain its purpose and what to consider before taking action. Spend less time looking things up and make more informed choices.

Get explanations directly from items in Deep Cleanup (built-in rules), Privacy Cleanup, Startup Item Management, System Optimization, and System Maintenance.

Official releases include free explanations each day, with the option to connect your own AI service. AI offers guidance; you decide which actions to take.

## Safety and Rules

> [!IMPORTANT]
> **MangoDisk puts data safety ahead of reclaiming more space.**
> Cleanup rules and system optimizations only ship after their safety boundaries are clearly defined and they pass validation on real systems.

MangoDisk scans in read-only mode by default. Before cleanup, deletion, uninstall, or system setting changes begin, you can review and confirm exactly what will happen. Results are saved to Operation History.

System Optimization only uses built-in, validated settings. It never accepts arbitrary registry paths, terminal commands, or scripts. MangoDisk reads each setting again after changing it and calls out high-impact items and changes that require administrator access or a restart.

MangoDisk maintains its own cleanup rules. Third-party projects may provide research leads, but a candidate rule is only accepted after reliable sources, safe boundaries, and real-system behavior have been verified. Anything without a clear safety boundary is excluded.

The complete rule library and revision history are open for inspection: [view the MangoDisk cleanup rule library](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules).

## Screenshots

<p align="center">
  <strong>Deep Cleanup</strong><br>
  <sub>Find cleanable content across the system, applications, developer tools, and projects to reclaim more space</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-01-deep-cleanup.jpg">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg">
    <img src="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg" width="1200" alt="MangoDisk Deep Cleanup interface">
  </picture>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>Large File Cleanup</strong><br>
      <sub>Find the files taking up the most space without digging through folders</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-02-large-file-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg" width="100%" alt="MangoDisk Large File Cleanup interface">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>Duplicate File Cleanup</strong><br>
      <sub>Safely remove exact duplicates while keeping at least one copy</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-03-duplicate-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg" width="100%" alt="MangoDisk Duplicate File Cleanup interface">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Disk Space Analysis</strong><br>
      <sub>See where your storage is going and quickly find the largest files and folders</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-05-disk-space-analysis.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg" width="100%" alt="MangoDisk Disk Space Analysis interface">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>Startup Item Management</strong><br>
      <sub>Reduce unnecessary startup programs for faster sign-in and less background activity</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-06-startup-items.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg" width="100%" alt="MangoDisk Startup Item Management interface">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Application Uninstall and Cleanup</strong><br>
      <sub>Uninstall applications and remove related leftovers to reclaim more space</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-04-app-uninstaller.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg" width="100%" alt="MangoDisk Application Uninstaller interface">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>System Optimization</strong><br>
      <sub>Optimize performance, privacy, and everyday usability in one click</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-07-system-optimization.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg" width="100%" alt="MangoDisk System Optimization interface">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>System Maintenance</strong><br>
      <sub>Fix common system issues quickly and get your computer back to normal</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-08-system-maintenance.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg" width="100%" alt="MangoDisk System Maintenance interface">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>Privacy Cleanup</strong><br>
      <sub>Leave fewer activity traces behind and keep everyday use more private</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-09-privacy-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg" width="100%" alt="MangoDisk Privacy Cleanup interface">
      </picture>
    </td>
  </tr>
</table>

## Install and Run

MangoDisk currently supports:

- **macOS**: macOS Monterey 12.5 or later.
- **Windows**: 64-bit Windows 10 or later.

Install MangoDisk on macOS with Homebrew:

```sh
brew install --cask harry0703/tap/mangodisk
```

Install MangoDisk on Windows from PowerShell:

```powershell
irm "https://get.mangodisk.app" | iex
```

Alternatively, download the latest version from the [MangoDisk website](https://mangodisk.app/) or [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest):

- **macOS**: Open the DMG and drag MangoDisk into the Applications folder.
- **Windows**: Run the Windows installer and follow the prompts.

> [!CAUTION]
>
> 1. Cleanup, permanent deletion, and uninstall operations may not be reversible. Review the selected content and keep reliable backups of important data.
> 2. Before running system maintenance or changing a startup item or system setting, make sure you understand its purpose and impact.
> 3. Some system optimizations can affect security, privacy, battery life, or update behavior.

## CLI Quick Start

Install the standalone CLI on macOS with Homebrew:

```sh
brew install harry0703/tap/mangodisk-cli
```

On Windows, install the latest CLI from PowerShell:

```powershell
irm "https://get.mangodisk.app/cli" | iex
```

If `mangodisk` is not immediately available after installation, open a new terminal, then verify the installation:

```sh
mangodisk --version
```

The CLI uses the same safety-first cleanup engine as the desktop application. Use commands such as:

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

`mangodisk clean` only scans and never modifies files by default. To perform cleanup in a non-interactive environment, you must also pass `--yes` to confirm explicitly. Run the following command for all available options:

```sh
mangodisk clean --help
```

## Build from Source

### Prerequisites

- Node.js 24 LTS
- pnpm 11.13.1
- Stable Rust
- macOS: Xcode Command Line Tools
- Windows: Visual Studio 2022 Build Tools with **Desktop development with C++**
- Windows: Microsoft Edge WebView2 Runtime

See the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for detailed platform requirements.

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

Local builds do not include the signing, notarization, or update metadata provided by official MangoDisk releases. Use them for local development and validation only.

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
