<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk 应用图标"> MangoDisk
</h1>

<p align="center">面向 macOS 和 Windows 的磁盘清理、空间分析、隐私保护与系统优化工具</p>

<p align="center">
  <a href="README.md">English</a> · 简体中文 · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
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
      <img src="https://assets.mangodisk.app/images/readme/zh-light.jpg" width="1200" alt="MangoDisk 磁盘清理、空间分析、隐私保护与系统优化工具">
    </picture>
  </a>
</p>

## MangoDisk 能做什么

> **存储空间**

### 1. 深度清理

一次找出散落在系统、应用、开发工具和本地项目中的可清理内容，省去手动逐处查找，并按类别汇总可释放空间：

- **系统与用户缓存**：释放系统临时文件、诊断数据和可重建缓存长期占用的空间。
- **应用缓存**：减少常用应用的缓存、日志、更新包和临时内容不断累积造成的空间占用。
- **浏览器数据**：回收 Chrome、Edge、Firefox、Brave、Arc、Opera 等浏览器缓存和临时网页数据占用的空间。
- **开发工具与 Xcode**：快速回收包管理器、IDE、编译工具和 Xcode 开发数据占用的大量空间。
- **容器缓存**：释放 Docker 等容器工具的闲置构建缓存和可重新生成数据占用的空间。
- **项目构建产物**：找回 Node.js、Rust、Gradle、Swift、Python、.NET、Godot、CMake 等项目中依赖、缓存和构建目录占用的空间。
- **AI 模型与缓存**：快速发现占用空间较大的本地模型、下载缓存和临时传输文件。
- **应用优化**：在不影响正常使用的前提下缩小应用体积，为磁盘腾出更多空间。

智能推荐帮助你快速做出安全选择，也可以逐项确认并提前了解预计可释放空间，让每次清理都更安心、更可控。

### 2. 大文件清理

快速找出最占空间的大文件，轻松释放被旧安装包、视频、归档等内容占用的空间，不必再逐层翻找目录。

### 3. 重复文件清理

找回被重复副本占用的空间，同时避免把同名但内容不同的文件误判为重复项；智能选择会为每组保留至少一份，让清理更省心。

### 4. 磁盘空间分析

直观看清磁盘空间都用在了哪里，逐层定位占用最大的目录和文件，减少盲目清理。

> **隐私与安全**

### 5. 隐私清理

减少浏览记录、搜索记录、Cookie、最近使用项目和剪贴板内容长期留存在电脑中，降低个人习惯、访问记录和账户状态暴露的风险，让日常隐私保护更简单、更可控。

> **系统工具**

### 6. 应用卸载与残留清理

卸载应用的同时清理关联缓存、设置和残留文件，避免应用删了、磁盘空间却没有真正释放；谨慎处理可能包含个人文件的内容，在释放空间的同时降低误删风险。

### 7. 启动项管理

减少不必要的开机等待和后台占用，让电脑启动更快、运行更轻盈；需要时仍可随时恢复。

### 8. 系统优化

减少拖慢系统或干扰日常使用的不必要设置，兼顾性能、隐私和使用习惯，让电脑运行更流畅、更顺手。

### 9. 系统维护

快速解决搜索异常、图标错乱、没有声音或网络连接异常等常见系统问题，省去手动排查和复杂命令，让电脑尽快恢复正常。

> **操作记录**

### 10. 操作历史

让每次清理和系统调整都有据可查，方便确认释放了多少空间、哪些操作已经完成，以及是否存在需要处理的问题。

## AI 解读

> 自 1.1.0 版本起提供

不清楚某个条目有什么用，或操作后会有什么影响？AI 解读结合条目说明和当前扫描结果，用易懂的语言解释用途和注意事项，省去四处查资料的麻烦，帮助你判断是否需要处理。

支持深度清理的内置规则、隐私清理、启动项管理、系统优化和系统维护，可直接在条目旁查看解读。

官方版本提供每日免费解读额度，也支持连接自己的 AI 服务。解读仅供参考，具体操作仍由你决定。

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
  <tr>
    <td width="50%" align="center">
      <strong>系统维护</strong><br>
      <sub>快速解决常见系统问题，让电脑恢复正常</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-08-system-maintenance.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-08-system-maintenance.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/zh/light-08-system-maintenance.jpg" width="100%" alt="MangoDisk 系统维护界面">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>隐私清理</strong><br>
      <sub>减少活动痕迹留存，更好地保护日常隐私</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-09-privacy-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-09-privacy-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/zh/light-09-privacy-cleanup.jpg" width="100%" alt="MangoDisk 隐私清理界面">
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
> 2. 执行系统维护、修改启动项或系统设置前，也请确认相关项目的用途和影响
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
