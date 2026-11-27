<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk 應用程式圖示"> MangoDisk
</h1>

<p align="center">適用於 <b>macOS</b>、<b>Windows</b> 與 <b>Linux</b> 的磁碟清理與儲存空間分析工具</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · 繁體中文 · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a> · <a href="README.ru.md">Русский</a>
</p>

<p align="center">
  <a href="https://github.com/harry0703/MangoDisk/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/harry0703/MangoDisk?display_name=tag&sort=semver"></a>
  <img alt="支援 macOS" src="https://img.shields.io/badge/macOS-supported-111827?logo=apple&logoColor=white">
  <img alt="支援 Windows" src="https://img.shields.io/badge/Windows-supported-2563eb?logo=windows&logoColor=white">
  <img alt="支援 Linux" src="https://img.shields.io/badge/Linux-supported-f59e0b?logo=linux&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri&logoColor=white">
  <img alt="Rust Core" src="https://img.shields.io/badge/core-Rust-b7410e?logo=rust&logoColor=white">
</p>

<p align="center">
  <a href="https://mangodisk.app/tw">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-00-overview.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-00-overview.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/tw/light-00-overview.jpg?ver=2.1.1" width="1200" alt="MangoDisk 總覽介面">
    </picture>
  </a>
</p>

## MangoDisk 能做什麼

> **儲存空間**

### 1. 深度清理

日常使用的應用程式、開發工具和 AI 工具會留下快取、建置產物和本機模型。MangoDisk 會把散落各處的內容集中列出，省去逐一翻找隱藏資料夾的麻煩。

- **系統與使用者快取**：清除系統暫存檔、診斷資料和可重新產生的快取。
- **應用程式快取**：清理應用程式留下的快取、記錄檔、更新套件和暫存檔。
- **瀏覽器資料**：清除 Chrome、Edge、Firefox、Brave、Arc、Opera 等瀏覽器的快取和暫存網頁資料。
- **開發工具與 Xcode**：清除套件管理工具、IDE、編譯工具的快取，以及 Xcode 開發資料。
- **容器快取**：清除 Docker 等容器工具閒置的建置快取和可重新產生的資料。
- **專案建置產物**：清理 Node.js、Rust、Gradle、Swift、Python、.NET、Godot、CMake 等專案可重新產生的相依套件、快取和建置目錄。
- **AI 工具**：清除 ChatGPT、Codex、Claude 等工具留下的快取、舊記錄檔和下載暫存檔。
- **AI 模型**：查看 Ollama、LM Studio、Hugging Face、ComfyUI 等本機模型占用的空間，再手動選取不需要的模型。刪除後若要再使用，需重新下載。
- **應用程式最佳化**：縮小支援的應用程式體積，維持正常使用。

你可以採用智慧推薦，或逐項檢查，並在清理前查看預計可釋放的空間。

### 2. 大型檔案清理

集中查看最占空間的檔案，像是舊安裝檔、影片或封存檔，不必一層層翻找資料夾。

### 3. 重複檔案清理

依檔案內容比對，即使檔名不同，也能找出內容完全相同的檔案。智慧選取會為每組至少保留一份。

### 4. 磁碟空間分析

#### 空間分布

用**樹狀圖**或**旭日圖**看清空間用在哪裡。你可以調整顯示層級，再透過檔案清單逐層查看，找出最占空間的資料夾和檔案。

#### 空間變化

磁碟空間突然變少了？開啟掃描紀錄後，比較同一個磁碟或資料夾的兩次掃描，就能看到哪些檔案和資料夾的占用空間有變化。清理後再掃描一次，也能確認哪些地方騰出了空間。

> **隱私與安全**

### 5. 隱私清理

清除瀏覽紀錄、搜尋紀錄、Cookie、最近使用的項目和剪貼簿內容，減少瀏覽器、應用程式及系統留下的使用痕跡。

> **系統工具**

### 6. 解除安裝應用程式與殘留清理

解除安裝時，一併清除相關快取、設定和殘留檔案。若資料夾可能含有個人檔案，MangoDisk 會謹慎處理，降低誤刪風險。

### 7. 啟動項目管理

停用不需要的開機啟動項目，縮短開機等待並減少背景資源用量。需要時可以再啟用。

### 8. 系統最佳化

依照你的需求調整系統設定，改善效能、隱私和日常操作。

### 9. 系統維護

修正搜尋異常、圖示顯示錯誤、沒聲音或網路連線異常等常見問題，省去手動排查或輸入指令的麻煩。

### 10. 右鍵選單（Windows）

依選單位置或應用程式瀏覽項目，停用、還原或移除可管理的項目，也能依檔案類型分別調整。

還可加入複製路徑、開啟終端機等常用操作，以及系統工具的捷徑。

> **操作紀錄**

### 11. 操作紀錄

每次清理和系統變更都有紀錄，方便查看釋放了多少空間、完成了哪些操作，還有哪些問題需要處理。

## 運作狀態與記憶體管理

即時查看 CPU、GPU、記憶體、網路和磁碟的使用狀況。支援的裝置也能顯示溫度、功耗、電池狀態和磁碟健康資訊。

找出資源用量高的應用程式，視需要結束程式，或手動、自動釋放記憶體。

無須開啟主視窗，就能從選單列、工作列、系統匣或桌面浮動視窗查看系統狀態。顯示項目和外觀都可以自訂。

## AI 解讀

不確定某個項目有什麼用途，或處理後會發生什麼事？AI 解讀會根據項目說明和掃描結果，簡要說明用途與注意事項。深度清理的內建規則、隱私清理、啟動項目管理、系統最佳化和系統維護，都能在項目旁查看解讀。

## 安全與規則

> [!IMPORTANT]
> **MangoDisk 以資料安全為優先。**
> 清理規則和系統最佳化項目都必須先確認在哪些情況下可以安全執行，並在實際系統上驗證，才會納入正式版。

掃描預設為唯讀。清理、刪除、解除安裝或變更系統設定前，請先檢查並確認操作內容。結果會存入操作紀錄。

系統最佳化只執行內建且已驗證的設定變更，完成後會重新讀取設定。高風險、需要系統管理員權限或需要重新啟動的項目，都會事先提醒。

MangoDisk 自行維護清理規則。其他專案可作為調查線索，每條新規則都會查證可靠來源、確認安全範圍，並在實際系統上測試後才收錄。

完整規則和歷次變更皆可公開查閱。[查看 MangoDisk 清理規則庫](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules)

## 介面預覽

<p align="center">
  <strong>深度清理</strong><br>
  <sub>清理累積的快取、建置產物和不再需要的 AI 模型</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-01-deep-cleanup.jpg?ver=2.1.1">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-01-deep-cleanup.jpg?ver=2.1.1">
    <img src="https://assets.mangodisk.app/images/screenshots/tw/light-01-deep-cleanup.jpg?ver=2.1.1" width="1200" alt="MangoDisk 深度清理介面">
  </picture>
</p>

<p align="center">
  <strong>空間分布</strong><br>
  <sub>看清空間用在哪裡，找出最占空間的檔案和資料夾</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/tw/light-05-disk-space-analysis.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-05-disk-space-analysis.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-05-disk-space-analysis.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/tw/light-05-disk-space-analysis.jpg?ver=2.1.1" width="1200" alt="MangoDisk 磁碟空間分析介面">
    </picture>
  </a>
</p>

<p align="center">
  <strong>空間變化</strong><br>
  <sub>比較兩次掃描，查看哪些檔案和資料夾的占用空間有變化</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/tw/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/tw/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" width="1200" alt="MangoDisk 空間變化畫面的掃描比較結果">
    </picture>
  </a>
</p>

<p align="center">
  <strong>系統監控</strong><br>
  <sub>一眼掌握 CPU、記憶體、網路和磁碟的使用狀況</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/tw/light-system-monitoring-memory.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-system-monitoring-memory.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-system-monitoring-memory.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/tw/light-system-monitoring-memory.jpg?ver=2.1.1" width="720" alt="MangoDisk 系統監控與記憶體管理介面">
    </picture>
  </a>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>大型檔案清理</strong><br>
      <sub>快速找出最占空間的檔案</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/tw/light-02-large-file-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-02-large-file-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-02-large-file-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/tw/light-02-large-file-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 大型檔案清理介面">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>重複檔案清理</strong><br>
      <sub>清理內容完全相同的檔案，每組至少保留一份</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/tw/light-03-duplicate-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-03-duplicate-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-03-duplicate-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/tw/light-03-duplicate-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 重複檔案清理介面">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>啟動項目管理</strong><br>
      <sub>停用不需要的開機啟動項目</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/tw/light-06-startup-items.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-06-startup-items.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-06-startup-items.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/tw/light-06-startup-items.jpg?ver=2.1.1" width="100%" alt="MangoDisk 啟動項目管理介面">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>解除安裝應用程式與殘留清理</strong><br>
      <sub>解除安裝應用程式，順帶清理殘留檔案</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/tw/light-04-app-uninstaller.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-04-app-uninstaller.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-04-app-uninstaller.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/tw/light-04-app-uninstaller.jpg?ver=2.1.1" width="100%" alt="MangoDisk 解除安裝應用程式介面">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>系統最佳化</strong><br>
      <sub>調整與效能、隱私及日常操作相關的系統設定</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/tw/light-07-system-optimization.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-07-system-optimization.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-07-system-optimization.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/tw/light-07-system-optimization.jpg?ver=2.1.1" width="100%" alt="MangoDisk 系統最佳化介面">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>系統維護</strong><br>
      <sub>修正常見的系統問題</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/tw/light-08-system-maintenance.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-08-system-maintenance.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-08-system-maintenance.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/tw/light-08-system-maintenance.jpg?ver=2.1.1" width="100%" alt="MangoDisk 系統維護介面">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>隱私清理</strong><br>
      <sub>減少瀏覽器、應用程式和系統留下的使用痕跡</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/tw/light-09-privacy-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-09-privacy-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-09-privacy-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/tw/light-09-privacy-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 隱私清理介面">
        </picture>
      </a>
    </td>
  </tr>
</table>

## 使用注意事項

> [!CAUTION]
>
> 1. 清理、永久刪除或解除安裝後，資料可能無法復原。執行前請確認所選項目，並備份重要資料。
> 2. 執行系統維護、調整開機啟動項目或系統設定前，請先了解項目的用途和變更後的影響。
> 3. 部分系統最佳化項目可能影響安全性、隱私、電池續航力或系統更新。

## 桌面版

可從 [官網下載頁面](https://mangodisk.app/tw/download) 或 [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest) 下載安裝檔，並依照所用系統選擇下方的安裝方式。

### macOS

**系統需求：** macOS Monterey 12.5 或更新版本。

**使用 Homebrew 安裝：**

```sh
brew install --cask harry0703/tap/mangodisk
```

**手動安裝：** 從 [官網下載頁面](https://mangodisk.app/tw/download) 下載 DMG，開啟後將 MangoDisk 拖入「應用程式」資料夾。

### Windows

**系統需求：** 64 位元 Windows 10 或更新版本。

**使用 PowerShell 安裝：**

```powershell
irm https://get.mangodisk.app | iex
```

**使用 WinGet 安裝（官方來源）：**

```powershell
winget install --id MangoDisk.MangoDisk --exact --source winget
```

**手動安裝：** 從 [官網下載頁面](https://mangodisk.app/tw/download) 下載 Windows 安裝程式，依照畫面指示完成安裝。

### Linux

**建議系統：** Ubuntu 22.04 LTS 或更新版本，支援 x64 與 ARM64。

提供 `.deb` 套件與 AppImage；其他 Linux 發行版的相容性取決於系統函式庫與桌面環境。

**使用 Homebrew 安裝：** 需要新版 Homebrew 與桌面環境。

```sh
brew install --cask harry0703/tap/mangodisk
```

**透過終端機安裝（Debian/Ubuntu）：** 自動辨識架構，並安裝對應的最新版 `.deb` 套件。

```sh
curl -fsSL https://get.mangodisk.app/linux | bash
```

**手動安裝：** 從 [官網下載頁面](https://mangodisk.app/tw/download) 選擇對應架構的安裝檔。

- **Debian/Ubuntu**：安裝對應架構的 `.deb` 套件。
- **其他發行版**：可嘗試 AppImage，賦予執行權限後即可執行。

## 命令列版（CLI）

你可以在終端機或指令碼中使用 CLI。它和桌面版共用同一套清理引擎。

### macOS

**使用 Homebrew 安裝：**

```sh
brew install harry0703/tap/mangodisk-cli
```

### Windows

**使用 PowerShell 安裝：**

```powershell
irm https://get.mangodisk.app/cli | iex
```

**使用 WinGet 安裝（官方來源）：**

```powershell
winget install --id MangoDisk.CLI --exact --source winget
```

### Linux

目前尚未提供 Linux 獨立 CLI 的預先編譯版本，可依照下方 [從原始碼建置](#從原始碼建置) 的步驟自行編譯。

### 使用範例

安裝完成後，如果暫時找不到 `mangodisk`，請重新開啟終端機，再檢查版本：

```sh
mangodisk --version
```

常用指令：

```sh
# 僅掃描並顯示可清理的內容
mangodisk clean

# 套用與桌面版相同的智慧建議
mangodisk clean --apply

# 預覽所有可選項目，不實際刪除
mangodisk clean --apply --selection all --dry-run

# 輸出方便指令碼處理的 JSON
mangodisk clean --format json --no-progress
```

`mangodisk clean` 預設只掃描，檔案會維持原樣。在非互動環境中執行清理時，請加上 `--yes` 確認。完整選項可用下列指令查看：

```sh
mangodisk clean --help
```

## 從原始碼建置

### 環境要求

- Node.js 24 LTS
- pnpm 11.13.1
- Rust 穩定版工具鏈

各平台所需的相依套件請參閱 [Tauri 2 前置需求](https://v2.tauri.app/start/prerequisites/)。

### 取得原始碼並啟動桌面應用程式

```sh
git clone https://github.com/harry0703/MangoDisk.git
cd MangoDisk
pnpm install --frozen-lockfile
pnpm tauri:dev
```

### 執行完整檢查

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

### 建置桌面安裝程式

```sh
pnpm tauri:build
```

### 建置 CLI

```sh
pnpm cli:build
```

本機建置只供開發和驗證使用。簽署、公證和更新中繼資料由正式發布流程提供。

## 支持 MangoDisk

MangoDisk 免費、開放原始碼，而且沒有廣告。核心功能依 GPL-3.0 免費提供，可用於個人或商業用途。想支持開發並使用官方雲端 AI，歡迎 [成為支持者](https://mangodisk.app/tw/supporter)。

## 參與貢獻

歡迎提交問題、清理規則、修復和新功能。開始前請閱讀
[`CONTRIBUTING.md`](CONTRIBUTING.md) 和 [`AGENTS.md`](AGENTS.md)。

一般清理規則應使用經過建置期驗證的宣告式 TOML。規則結構、安全限制和驗證方式請參閱
[`src-tauri/crates/mangodisk-core/rules/README.md`](src-tauri/crates/mangodisk-core/rules/README.md)。

提交修改前，請至少執行：

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

發現安全問題時，請按照 [`SECURITY.md`](SECURITY.md) 透過 GitHub Security Advisories 私下報告，不要建立公開 Issue。

## 技術架構

- [Tauri 2](https://tauri.app/)：桌面執行時與系統整合
- [Rust](https://www.rust-lang.org/)：掃描、檔案系統、安全驗證和清理執行
- [Vue 3](https://vuejs.org/) 與 [TypeScript](https://www.typescriptlang.org/)：桌面使用者介面

## 授權條款

MangoDisk 採用 [GNU General Public License v3.0](https://github.com/harry0703/MangoDisk/blob/main/LICENSE) 開放原始碼。第三方元件仍適用各自的授權條款。
