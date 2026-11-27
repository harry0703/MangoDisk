<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk 应用图标"> MangoDisk - 芒果磁盘清理
</h1>

<p align="center">适用于 <b>macOS</b>、<b>Windows</b> 和 <b>Linux</b> 的磁盘清理与空间分析工具</p>

<p align="center">
  <a href="README.md">English</a> · 简体中文 · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a> · <a href="README.ru.md">Русский</a>
</p>

<p align="center">
  <a href="https://github.com/harry0703/MangoDisk/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/harry0703/MangoDisk?display_name=tag&sort=semver"></a>
  <img alt="支持 macOS" src="https://img.shields.io/badge/macOS-supported-111827?logo=apple&logoColor=white">
  <img alt="支持 Windows" src="https://img.shields.io/badge/Windows-supported-2563eb?logo=windows&logoColor=white">
  <img alt="支持 Linux" src="https://img.shields.io/badge/Linux-supported-f59e0b?logo=linux&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri&logoColor=white">
  <img alt="Rust Core" src="https://img.shields.io/badge/core-Rust-b7410e?logo=rust&logoColor=white">
</p>

<p align="center">
  <a href="https://mangodisk.app/zh">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-00-overview.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-00-overview.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/zh/light-00-overview.jpg?ver=2.1.1" width="1200" alt="MangoDisk 概览界面">
    </picture>
  </a>
</p>

## MangoDisk 能做什么

> **存储空间**

### 1. 深度清理

日常应用、开发工具和 AI 工具留下的缓存、构建产物与本地模型，常常散落在各处，不知不觉就占满磁盘。MangoDisk 把它们集中找出来，省去逐个翻找隐藏目录的麻烦：

- **系统与用户缓存**：清理系统临时文件、诊断数据和可重新生成的缓存，释放长期占用的空间。
- **应用缓存**：清理常用应用的缓存、日志、更新包和临时文件，避免它们越积越多。
- **浏览器数据**：清理 Chrome、Edge、Firefox、Brave、Arc、Opera 等浏览器的缓存和临时网页数据。
- **开发工具与 Xcode**：清理包管理器、IDE、编译工具产生的缓存，以及 Xcode 开发数据，快速释放空间。
- **容器缓存**：清理 Docker 等容器工具的闲置构建缓存和可重新生成的数据。
- **项目构建产物**：清理 Node.js、Rust、Gradle、Swift、Python、.NET、Godot、CMake 等项目中可重新生成的依赖、缓存和构建目录。
- **AI 工具**：清理 ChatGPT、Codex、Claude 等工具留下的缓存、旧日志和临时下载文件，无需逐个查找数据目录。
- **AI 模型**：查看 Ollama、LM Studio、Hugging Face、ComfyUI 等本地模型占用的空间，删除不再需要的模型。模型需手动选择；清理后如需再次使用，要重新下载。
- **应用优化**：在不影响正常使用的前提下，减小受支持应用的体积。

智能推荐可帮你快速选出安全的清理项。你也可以逐项确认，并在清理前查看预计可释放的空间。

### 2. 大文件清理

快速找出最占空间的大文件，集中查看旧安装包、视频、归档文件等，不用再逐层翻目录。

### 3. 重复文件清理

根据文件内容识别重复项，避免把同名但内容不同的文件误判为重复。智能选择会为每组至少保留一份，方便清理多余副本。

### 4. 磁盘空间分析

#### 空间分布

直观看清磁盘空间的去向。可在**矩形图**和**旭日图**之间切换，自定义显示层级，并结合文件列表逐层浏览，快速找到最占空间的目录和文件。

#### 空间变化

磁盘空间突然少了？开启扫描记录后，对比同一磁盘或文件夹的两次扫描，就能看到哪些文件和文件夹的占用发生了变化、变化了多少。清理后再扫一次，还能确认空间是从哪里腾出来的。

> **隐私与安全**

### 5. 隐私清理

清理浏览记录、搜索记录、Cookie、最近使用项目和剪贴板内容，减少浏览器、应用和系统留下的使用痕迹，降低个人隐私泄露的风险。

> **系统工具**

### 6. 应用卸载与残留清理

卸载应用时一并清理相关缓存、设置和残留文件，避免应用删了，空间却没真正释放出来。对可能包含个人文件的内容会谨慎处理，降低误删风险。

### 7. 启动项管理

关闭不必要的启动项，减少开机等待和后台资源占用；需要时可随时恢复。

### 8. 系统优化

调整影响性能或日常使用的不必要设置，兼顾性能、隐私和个人习惯，让系统更流畅、更顺手。

### 9. 系统维护

修复搜索异常、图标错乱、无声或网络连接异常等常见问题，无需手动排查或输入复杂命令。

### 10. 右键菜单（Windows）

按菜单位置或所属应用查看条目，可禁用、恢复或删除可管理的菜单项，也能按文件类型分别调整，让右键菜单更清爽。

还可添加复制路径、打开终端等常用操作，以及系统工具入口。

> **操作记录**

### 11. 操作历史

每次清理和系统调整都有记录，方便查看释放了多少空间、哪些操作已完成，以及是否有需要处理的问题。

## 运行状态与内存管理

实时监控 CPU、GPU、内存、网络和磁盘状态，了解资源占用情况及变化趋势。在支持的设备上，还可查看温度、功耗、电池状态和磁盘健康信息。

快速定位资源占用较高的应用，可直接退出，也可手动或自动释放内存。

无需打开主窗口，通过菜单栏、任务栏、系统托盘或桌面悬浮窗即可查看电脑状态，显示内容与外观均可自定义。

## AI 解读

看不懂某个条目，或不确定操作后的影响？AI 解读会结合条目说明和当前扫描结果，说明用途与注意事项，帮你判断是否需要处理。深度清理内置规则、隐私清理、启动项管理、系统优化和系统维护均支持在条目旁查看。

## 安全与规则

> [!IMPORTANT]
> **MangoDisk 始终将数据安全置于清理效果之上。**
> 所有清理规则和系统优化项，都必须明确安全边界并通过真实系统验证，才会纳入正式版本。

MangoDisk 默认以只读方式扫描。清理、删除、卸载或修改系统设置前，会先列出相关内容，确认后才执行；操作结果保存在历史记录中。

系统优化只执行内置且经过验证的设置项，不支持任意注册表路径、终端命令或脚本。更改后会重新读取系统状态；高风险、需要管理员权限或重启的项目会提前提示。

MangoDisk 独立维护清理规则。第三方项目仅作为线索；候选规则必须核对可靠来源、明确安全边界，并通过真实系统验证后才会收录。无法确认安全的内容不会加入规则库。

完整规则库及修改记录均公开可查：[查看 MangoDisk 清理规则库](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules)。

## 界面预览

<p align="center">
  <strong>深度清理</strong><br>
  <sub>清理积累的缓存、构建产物和不再需要的 AI 模型，为需要的内容腾出空间</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-01-deep-cleanup.jpg?ver=2.1.1">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-01-deep-cleanup.jpg?ver=2.1.1">
    <img src="https://assets.mangodisk.app/images/screenshots/zh/light-01-deep-cleanup.jpg?ver=2.1.1" width="1200" alt="MangoDisk 深度清理界面">
  </picture>
</p>

<p align="center">
  <strong>空间分布</strong><br>
  <sub>直观看清空间去向，快速定位占用最多的内容</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/zh/light-05-disk-space-analysis.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-05-disk-space-analysis.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-05-disk-space-analysis.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/zh/light-05-disk-space-analysis.jpg?ver=2.1.1" width="1200" alt="MangoDisk 磁盘空间分析界面">
    </picture>
  </a>
</p>

<p align="center">
  <strong>空间变化</strong><br>
  <sub>对比两次扫描，看清哪些文件和文件夹的占用增加或减少</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/zh/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/zh/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" width="1200" alt="MangoDisk 空间变化界面的扫描对比结果">
    </picture>
  </a>
</p>

<p align="center">
  <strong>系统监控</strong><br>
  <sub>CPU、内存、网络和磁盘状态一目了然</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/zh/light-system-monitoring-memory.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-system-monitoring-memory.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-system-monitoring-memory.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/zh/light-system-monitoring-memory.jpg?ver=2.1.1" width="720" alt="MangoDisk 系统监控与内存管理界面">
    </picture>
  </a>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>大文件清理</strong><br>
      <sub>快速锁定最占空间的文件，避免逐层翻找</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/zh/light-02-large-file-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-02-large-file-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-02-large-file-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/zh/light-02-large-file-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 大文件清理界面">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>重复文件清理</strong><br>
      <sub>安全清理重复副本，并确保每组至少保留一份</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/zh/light-03-duplicate-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-03-duplicate-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-03-duplicate-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/zh/light-03-duplicate-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 重复文件清理界面">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>启动项管理</strong><br>
      <sub>减少不必要的自启动程序，加快登录并降低后台占用</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/zh/light-06-startup-items.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-06-startup-items.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-06-startup-items.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/zh/light-06-startup-items.jpg?ver=2.1.1" width="100%" alt="MangoDisk 启动项管理界面">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>应用卸载与残留清理</strong><br>
      <sub>卸载应用并清理关联残留，释放更多磁盘空间</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/zh/light-04-app-uninstaller.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-04-app-uninstaller.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-04-app-uninstaller.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/zh/light-04-app-uninstaller.jpg?ver=2.1.1" width="100%" alt="MangoDisk 应用卸载界面">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>系统优化</strong><br>
      <sub>一键优化性能、隐私与使用体验，让系统运行更流畅</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/zh/light-07-system-optimization.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-07-system-optimization.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-07-system-optimization.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/zh/light-07-system-optimization.jpg?ver=2.1.1" width="100%" alt="MangoDisk 系统优化界面">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>系统维护</strong><br>
      <sub>快速解决常见系统问题，让电脑恢复正常</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/zh/light-08-system-maintenance.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-08-system-maintenance.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-08-system-maintenance.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/zh/light-08-system-maintenance.jpg?ver=2.1.1" width="100%" alt="MangoDisk 系统维护界面">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>隐私清理</strong><br>
      <sub>减少活动痕迹留存，更好地保护日常隐私</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/zh/light-09-privacy-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/zh/dark-09-privacy-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/zh/light-09-privacy-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/zh/light-09-privacy-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 隐私清理界面">
        </picture>
      </a>
    </td>
  </tr>
</table>

## 使用注意事项

> [!CAUTION]
>
> 1. 清理、彻底删除和卸载后可能无法恢复。操作前请确认所选内容，并备份重要数据。
> 2. 执行系统维护、修改启动项或系统设置前，请确认相关项目的用途和影响。
> 3. 部分系统优化可能影响系统安全、隐私、续航或更新策略。

## 桌面版

可从 [官网下载页](https://mangodisk.app/zh/download) 或 [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest) 下载安装包，按你的系统选择安装方式。

### macOS

**系统要求：** macOS Monterey 12.5 或更高版本。

**Homebrew 安装：**

```sh
brew install --cask harry0703/tap/mangodisk
```

**手动安装：** 从 [官网下载页](https://mangodisk.app/zh/download) 下载 DMG 文件，打开后将 MangoDisk 拖入“应用程序”文件夹。

### Windows

**系统要求：** 64 位 Windows 10 或更高版本。

**PowerShell 安装：**

```powershell
irm https://get.mangodisk.app | iex
```

**WinGet 安装（官方源）：**

```powershell
winget install --id MangoDisk.MangoDisk --exact --source winget
```

**手动安装：** 从 [官网下载页](https://mangodisk.app/zh/download) 下载 Windows 安装程序，按提示完成安装。

### Linux

**推荐系统：** Ubuntu 22.04 LTS 或更高版本，支持 x64 和 ARM64。

提供 `.deb` 和 AppImage 安装包；其他发行版能否正常运行，取决于系统库和桌面环境。

**Homebrew 安装：** 需要较新版本的 Homebrew 和桌面环境。

```sh
brew install --cask harry0703/tap/mangodisk
```

**终端安装（Debian/Ubuntu）：** 自动识别系统架构，安装对应的最新版 `.deb` 包。

```sh
curl -fsSL https://get.mangodisk.app/linux | bash
```

**手动安装：** 从 [官网下载页](https://mangodisk.app/zh/download) 选择对应架构的安装包。

- **Debian/Ubuntu**：安装对应架构的 `.deb` 包。
- **其他发行版**：可尝试 AppImage，添加执行权限后直接运行。

## 命令行版（CLI）

适合在终端或脚本中使用，与桌面版共用同一套安全清理引擎。

### macOS

**Homebrew 安装：**

```sh
brew install harry0703/tap/mangodisk-cli
```

### Windows

**PowerShell 安装：**

```powershell
irm https://get.mangodisk.app/cli | iex
```

**WinGet 安装（官方源）：**

```powershell
winget install --id MangoDisk.CLI --exact --source winget
```

### Linux

暂未提供独立 CLI 预编译包，可按照下方 [从源码构建](#从源码构建) 的步骤自行构建。

### 使用示例

安装后如果无法识别 `mangodisk` 命令，请重新打开终端，再检查版本：

```sh
mangodisk --version
```

常用命令：

```sh
# 扫描并列出可清理内容，不做修改
mangodisk clean

# 按桌面版的智能推荐执行清理
mangodisk clean --apply

# 预览全部可选内容，不删除文件
mangodisk clean --apply --selection all --dry-run

# 输出 JSON，便于脚本处理
mangodisk clean --format json --no-progress
```

`mangodisk clean` 默认只扫描，不修改文件。在非交互环境中执行清理时，还需传入 `--yes` 明确确认。查看完整选项请运行：

```sh
mangodisk clean --help
```

## 从源码构建

### 环境要求

- Node.js 24 LTS
- pnpm 11.13.1
- Stable Rust

平台依赖请参考 [Tauri 2 前置依赖说明](https://v2.tauri.app/start/prerequisites/)。

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

本地构建产物不含正式发布版的签名、公证和更新元数据，仅供本地开发与验证。

## 支持 MangoDisk

MangoDisk 免费、开源、无广告，核心功能可依据 GPL-3.0 免费用于个人和商业用途。如果你想支持开发，并使用官方云端 AI，可以 [成为支持者](https://mangodisk.app/zh/supporter)。

## 参与贡献

欢迎反馈问题，也欢迎贡献清理规则、代码修复和新功能。开始前请先阅读
[`CONTRIBUTING.md`](CONTRIBUTING.md) 和 [`AGENTS.md`](AGENTS.md)。

常规清理功能优先使用经过构建期校验的声明式 TOML 规则。规则结构、安全约束和验证方法请参阅
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
