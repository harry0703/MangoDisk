<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk 应用图标"> MangoDisk
</h1>

<p align="center">面向 macOS 和 Windows 的磁盘清理、空间分析与系统优化工具</p>

<p align="center">
  <a href="README.md">English</a> · 简体中文 · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  <a href="https://github.com/harry0703/MangoDisk/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/harry0703/MangoDisk?display_name=tag&sort=semver"></a>
  <img alt="支持 macOS" src="https://img.shields.io/badge/macOS-supported-111827?logo=apple&logoColor=white">
  <img alt="支持 Windows" src="https://img.shields.io/badge/Windows-supported-2563eb?logo=windows&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri&logoColor=white">
  <img alt="Rust Core" src="https://img.shields.io/badge/core-Rust-b7410e?logo=rust&logoColor=white">
</p>

<p align="center">
  <a href="https://mangodisk.app/zh">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/readme/zh-dark.jpg">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/readme/zh-light.jpg">
      <img src="https://assets.mangodisk.app/images/readme/zh-light.jpg" width="1200" alt="MangoDisk 磁盘清理、空间分析与系统优化工具">
    </picture>
  </a>
</p>

## MangoDisk 能做什么

> **存储空间**

### 1. 深度清理

一次找出散落在系统、应用、开发工具和本地项目中的可清理内容，省去手动逐处查找，并按类别汇总可释放空间：

- **系统与用户缓存**：清理系统临时文件、诊断数据，以及保存在用户目录中的可重建缓存。
- **应用缓存**：清理常用应用运行时产生的缓存、日志、更新包和临时内容。
- **浏览器数据**：清理 Chrome、Edge、Firefox、Brave、Arc、Opera 等浏览器产生的缓存和临时网页数据。
- **开发工具与 Xcode**：清理包管理器下载缓存、IDE 索引、编译缓存，以及 Xcode 生成的设备支持、归档和开发数据。
- **容器缓存**：清理 Docker 等容器工具产生的闲置构建缓存和可重新生成的临时数据。
- **项目构建产物**：识别 Node.js、Rust、Gradle、Swift、Python、.NET、Godot、CMake 等项目中可重新生成的依赖、缓存和构建目录。
- **AI 模型与缓存**：识别本地 AI 模型、下载缓存和临时传输文件，帮助发现占用空间较大的模型数据。
- **应用优化**：清理支持的应用中当前设备用不到的处理器代码，在不影响正常使用的前提下减少应用占用空间。

扫描过程只读取文件信息，不会自动删除任何内容。你可以采用智能推荐，也可以逐项确认，查看预计可释放空间后再执行清理。

### 2. 大文件清理

快速锁定最占空间的文件，不必逐层翻找目录；按类型和大小筛选后，再确认内容和位置，放心清理。

### 3. 重复文件清理

按文件内容准确识别重复副本，避免仅凭文件名误判；智能选择会为每组保留至少一份，让释放空间更省心。

### 4. 磁盘空间分析

直观看清磁盘空间都用在了哪里，逐层定位占用最大的目录和文件，减少盲目清理。

> **系统工具**

### 5. 应用卸载与残留清理

卸载应用时一并找出关联缓存、设置和残留文件，释放更多空间；同时区分可重新生成的内容与可能包含个人文件的数据，避免误删。应用正在运行或受系统保护时，MangoDisk 会提前提示。

### 6. 启动项管理

关闭不必要的自启动程序，有助于缩短开机或登录等待时间并减少后台占用；需要时可以随时重新启用。

### 7. 系统优化

集中优化影响性能、隐私和日常体验的系统设置，减少不必要的后台负担与干扰，让电脑更流畅、更顺手。

- **一键优化**：选择智能推荐、性能优先或隐私优先，快速匹配不同使用需求。
- **自由调整**：每项设置都可以单独开启或关闭，应用前可查看全部更改。
- **清晰提示**：高影响、需要管理员权限或需要重新登录、重启的项目会提前说明。
- **全面验证**：全部适用优化项已在 Windows 10、Windows 11、macOS 12.5、macOS 15.7 和 macOS 26 的真实系统环境中逐项测试。

> **操作记录**

**恢复 macOS 默认设置**

系统优化会直接修改 macOS 偏好设置。如果想一次性撤销全部修改、把所有受影响的设置恢复为系统出厂默认（无需在应用内逐项切换），可以运行仓库自带脚本。脚本会先把受影响的偏好域备份到 `~/Desktop`，任何域都可以回滚：

```sh
./scripts/restore-macos-defaults.sh
```

### 8. 操作历史

清楚回顾每次清理和系统调整做了什么、结果如何，方便核对变更和排查失败项目。

## 安全与规则

> [!IMPORTANT]
> **MangoDisk 始终将数据安全置于清理效果之上。**
> 所有清理规则和系统优化项只有在明确安全边界并通过真实系统验证后，才会纳入正式版本。

MangoDisk 默认只读扫描。执行清理、删除、卸载或系统设置变更前，会先展示内容并由用户确认；操作结果会保留在历史记录中。

系统优化仅执行内置且经过验证的设置项，不接受任意注册表路径、终端命令或脚本。更改后会重新读取系统状态；高风险、需要管理员权限或需要重启的项目会提前提示。

清理规则由 MangoDisk 独立维护。第三方项目只用于提供线索，候选规则必须核对可靠来源、明确安全边界，并通过真实系统验证后才会收录。无法确认安全的内容不会加入规则库。

完整规则库及修改记录均可审计、追溯：[查看 MangoDisk 清理规则库](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules)。

## 界面预览

<p align="center">
  <strong>深度清理</strong><br>
  <sub>集中找出系统、应用、开发工具和项目中的可清理内容，释放更多空间</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-01-deep-cleanup.jpg">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-01-deep-cleanup.jpg">
    <img src="https://assets.mangodisk.app/images/screenshots/zh/light-01-deep-cleanup.jpg" width="1200" alt="MangoDisk 深度清理界面">
  </picture>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>大文件清理</strong><br>
      <sub>快速锁定最占空间的文件，避免逐层翻找</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-02-large-file-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-02-large-file-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/zh/light-02-large-file-cleanup.jpg" width="100%" alt="MangoDisk 大文件清理界面">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>重复文件清理</strong><br>
      <sub>安全清理重复副本，并确保每组至少保留一份</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-03-duplicate-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-03-duplicate-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/zh/light-03-duplicate-cleanup.jpg" width="100%" alt="MangoDisk 重复文件清理界面">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>磁盘空间分析</strong><br>
      <sub>直观看清空间去向，快速定位占用最多的内容</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-05-disk-space-analysis.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-05-disk-space-analysis.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/zh/light-05-disk-space-analysis.jpg" width="100%" alt="MangoDisk 磁盘空间分析界面">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>启动项管理</strong><br>
      <sub>减少不必要的自启动程序，加快登录并降低后台占用</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-06-startup-items.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-06-startup-items.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/zh/light-06-startup-items.jpg" width="100%" alt="MangoDisk 启动项管理界面">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>应用卸载与残留清理</strong><br>
      <sub>卸载应用并清理关联残留，释放更多磁盘空间</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-04-app-uninstaller.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-04-app-uninstaller.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/zh/light-04-app-uninstaller.jpg" width="100%" alt="MangoDisk 应用卸载界面">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>系统优化</strong><br>
      <sub>一键优化性能、隐私与使用体验，让系统运行更流畅</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-07-system-optimization.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-07-system-optimization.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/zh/light-07-system-optimization.jpg" width="100%" alt="MangoDisk 系统优化界面">
      </picture>
    </td>
  </tr>
</table>

## 安装与使用

当前版本支持以下系统：

- **macOS**：macOS 12.5 Monterey 或更高版本。
- **Windows**：64 位 Windows 10 或更高版本。

macOS 用户可以通过 Homebrew 快速安装：

```sh
brew install --cask harry0703/tap/mangodisk
```

Windows 用户可以在 PowerShell 中快速安装：

```powershell
irm "https://get.mangodisk.app" | iex
```

也可以前往 [MangoDisk 官网](https://mangodisk.app/zh) 或 [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest) 下载最新版：

- **macOS**：打开 DMG，将 MangoDisk 拖入“应用程序”文件夹。
- **Windows**：运行 Windows 安装程序并按提示完成安装。

> [!CAUTION]
>
> 1. 清理、彻底删除和卸载操作可能无法恢复。请在执行前确认内容，并为重要数据保留可靠备份
> 2. 修改启动项或系统设置前，也请确认相关程序和优化项的用途
> 3. 部分系统优化可能影响安全性、隐私、续航或系统更新策略

## CLI 快速示例

macOS 用户可以通过 Homebrew 安装独立 CLI：

```sh
brew install harry0703/tap/mangodisk-cli
```

Windows 用户可以在 PowerShell 中安装最新版 CLI：

```powershell
irm "https://get.mangodisk.app/cli" | iex
```

安装完成后，如果暂时无法识别 `mangodisk`，请重新打开终端，然后检查版本：

```sh
mangodisk --version
```

CLI 与桌面应用使用同一套安全清理引擎，可以使用以下命令：

```sh
# 只扫描并展示可清理内容
mangodisk clean

# 应用与桌面端一致的智能推荐
mangodisk clean --apply

# 预览全部可选内容，不实际删除
mangodisk clean --apply --selection all --dry-run

# 输出便于脚本处理的 JSON
mangodisk clean --format json --no-progress
```

`mangodisk clean` 默认只扫描，不会修改文件。非交互环境执行实际清理时，还必须传入 `--yes` 明确确认；完整选项请运行：

```sh
mangodisk clean --help
```

## 从源码构建

### 环境要求

- Node.js 24 LTS
- pnpm 11.13.1
- Stable Rust
- macOS：Xcode Command Line Tools
- Windows：Visual Studio 2022 Build Tools，并安装“使用 C++ 的桌面开发”
- Windows：Microsoft Edge WebView2 Runtime

平台依赖也可以参考 [Tauri 2 前置依赖说明](https://v2.tauri.app/start/prerequisites/)。

### 获取源码并启动桌面应用

```sh
git clone https://github.com/harry0703/MangoDisk.git
cd MangoDisk
pnpm install --frozen-lockfile
pnpm tauri:dev
```

### 运行完整检查

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

### 构建桌面安装包

```sh
pnpm tauri:build
```

### 构建 CLI

```sh
pnpm cli:build
```

本地构建产物不包含 MangoDisk 正式发布流程提供的签名、公证和更新元数据，仅用于开发与验证。

## 参与贡献

欢迎提交问题、清理规则、修复和新功能。开始前请阅读
[`CONTRIBUTING.md`](CONTRIBUTING.md) 和 [`AGENTS.md`](AGENTS.md)。

常规清理覆盖优先使用经过构建期校验的声明式 TOML 规则。规则结构、安全约束和验证方式请参阅
[`src-tauri/crates/mangodisk-core/rules/README.md`](src-tauri/crates/mangodisk-core/rules/README.md)。

提交修改前，请至少运行：

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

发现安全问题时，请按照 [`SECURITY.md`](SECURITY.md) 通过 GitHub Security Advisories 私下报告，不要创建公开 Issue。

## 技术栈

- [Tauri 2](https://tauri.app/)：桌面运行时与系统集成
- [Rust](https://www.rust-lang.org/)：扫描、文件系统、安全校验和清理执行
- [Vue 3](https://vuejs.org/) 与 [TypeScript](https://www.typescriptlang.org/)：桌面交互界面

## 许可证

MangoDisk 基于 [GNU General Public License v3.0](https://github.com/harry0703/MangoDisk/blob/main/LICENSE) 开源。第三方组件继续遵循各自的许可证。
