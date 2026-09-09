<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk 應用程式圖示"> MangoDisk
</h1>

<p align="center">適用於 macOS 與 Windows 的磁碟清理、空間分析、隱私保護與系統最佳化工具</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · 繁體中文 · <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  <a href="https://github.com/harry0703/MangoDisk/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/harry0703/MangoDisk?display_name=tag&sort=semver"></a>
  <img alt="支援 macOS" src="https://img.shields.io/badge/macOS-supported-111827?logo=apple&logoColor=white">
  <img alt="支援 Windows" src="https://img.shields.io/badge/Windows-supported-2563eb?logo=windows&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri&logoColor=white">
  <img alt="Rust Core" src="https://img.shields.io/badge/core-Rust-b7410e?logo=rust&logoColor=white">
</p>

<p align="center">
  <a href="https://mangodisk.app/tw">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/readme/tw-dark.jpg">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/readme/tw-light.jpg">
      <img src="https://assets.mangodisk.app/images/readme/tw-light.jpg" width="1200" alt="MangoDisk 磁碟清理、空間分析、隱私保護與系統最佳化工具">
    </picture>
  </a>
</p>

## MangoDisk 能做什麼

> **儲存空間**

### 1. 深度清理

一次找出散落在系統、應用程式、開發工具與本機專案中的可清理內容，省去逐一查找的時間，並依類別彙整可釋放空間：

- **系統與使用者快取**：釋放系統暫存檔案、診斷資料和可重建快取長期佔用的空間。
- **應用程式快取**：避免常用應用程式的快取、記錄檔、更新套件和暫存內容持續累積、佔用更多空間。
- **瀏覽器資料**：取回 Chrome、Edge、Firefox、Brave、Arc、Opera 等瀏覽器快取和暫存網頁資料佔用的空間。
- **開發工具與 Xcode**：快速釋放套件管理工具、IDE、編譯工具和 Xcode 開發資料佔用的大量空間。
- **容器快取**：釋放 Docker 等容器工具的閒置建置快取和可重新產生資料佔用的空間。
- **專案建置產物**：找回 Node.js、Rust、Gradle、Swift、Python、.NET、Godot、CMake 等專案中相依套件、快取和建置目錄佔用的空間。
- **AI 模型與快取**：快速找出佔用大量空間的本機模型、下載快取和暫存傳輸檔案。
- **應用程式最佳化**：在不影響正常使用的情況下縮小應用程式體積，為磁碟騰出更多空間。

智慧建議能協助你快速做出安全選擇，也可以逐項確認並事先掌握預估可釋放空間，讓每次清理都更安心、更容易掌控。

### 2. 大型檔案清理

快速找出最佔空間的大型檔案，輕鬆釋放舊安裝檔、影片、封存檔等內容佔用的空間，不必再逐層翻找資料夾。

### 3. 重複檔案清理

找回被重複副本佔用的空間，同時避免把同名但內容不同的檔案誤判為重複項目；智慧選取會為每組至少保留一份，清理更省心。

### 4. 磁碟空間分析

一眼看懂磁碟空間都用在哪裡，逐層找出佔用最大的資料夾與檔案，避免盲目清理。

> **隱私與安全**

### 5. 隱私清理

減少瀏覽紀錄、搜尋紀錄、Cookie、最近使用項目和剪貼簿內容長期留在電腦中，降低瀏覽習慣、活動紀錄和登入狀態意外曝光的風險，讓日常隱私更容易掌握與管理。

> **系統工具**

### 6. 解除安裝應用程式與殘留清理

解除安裝應用程式時一併清除相關快取、設定與殘留檔案，避免程式移除了，磁碟空間卻沒有真正釋放；謹慎處理可能包含個人檔案的內容，在騰出空間的同時降低誤刪風險。

### 7. 啟動項目管理

減少不必要的開機等待與背景佔用，讓電腦啟動更快、運作更輕快；需要時仍可隨時重新啟用。

### 8. 系統最佳化

減少拖慢系統或干擾日常使用的不必要設定，兼顧效能、隱私與使用習慣，讓電腦運作更流暢、用起來更順手。

### 9. 系統維護

快速解決搜尋異常、圖示錯亂、沒有聲音或網路連線異常等常見系統問題，不必逐項檢查或輸入複雜指令，讓電腦盡快恢復正常。

> **操作紀錄**

### 10. 操作紀錄

讓每次清理與系統調整都有紀錄可查，方便確認釋放了多少空間、哪些操作已經完成，以及是否仍有需要處理的問題。

## AI 解讀

> 自 1.1.0 版本起提供

不清楚某個項目有什麼用途，或操作後會有什麼影響？AI 解讀會結合項目說明與目前的掃描結果，用易懂的文字說明用途與注意事項，省去四處查資料的麻煩，協助你判斷是否需要處理。

支援深度清理的內建規則、隱私清理、啟動項目管理、系統最佳化和系統維護，可直接在項目旁查看解讀。

官方版本提供每日免費解讀額度，也支援連接自己的 AI 服務。解讀僅供參考，實際操作仍由你決定。

## 安全與規則

> [!IMPORTANT]
> **MangoDisk 始終將資料安全放在清理效果之前。**
> 所有清理規則與系統最佳化項目，只有在安全邊界明確且通過實際系統驗證後，才會納入正式版本。

MangoDisk 預設只進行唯讀掃描。執行清理、刪除、解除安裝或變更系統設定前，會先顯示內容並由使用者確認；操作結果會保留在操作紀錄中。

系統最佳化只會執行內建且經過驗證的設定，不接受任意登錄路徑、終端機指令或腳本。變更後會重新讀取系統狀態；高影響、需要系統管理員權限或需要重新啟動的項目都會提前提示。

清理規則由 MangoDisk 獨立維護。第三方專案只用來提供研究線索；候選規則必須核對可靠來源、確認安全範圍，並通過真實系統驗證後才會收錄。安全範圍不明確的內容不會加入規則庫。

完整規則庫與修改紀錄均可檢視、追溯：[查看 MangoDisk 清理規則庫](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules)。

## 介面預覽

<p align="center">
  <strong>深度清理</strong><br>
  <sub>集中找出系統、應用程式、開發工具與專案中的可清理內容，釋放更多空間</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-01-deep-cleanup.jpg">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-01-deep-cleanup.jpg">
    <img src="https://assets.mangodisk.app/images/screenshots/tw/light-01-deep-cleanup.jpg" width="1200" alt="MangoDisk 深度清理介面">
  </picture>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>大型檔案清理</strong><br>
      <sub>快速鎖定最佔空間的檔案，不必逐層翻找</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-02-large-file-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-02-large-file-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/tw/light-02-large-file-cleanup.jpg" width="100%" alt="MangoDisk 大型檔案清理介面">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>重複檔案清理</strong><br>
      <sub>安全清理重複副本，並確保每組至少保留一份</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-03-duplicate-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-03-duplicate-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/tw/light-03-duplicate-cleanup.jpg" width="100%" alt="MangoDisk 重複檔案清理介面">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>磁碟空間分析</strong><br>
      <sub>一眼看懂空間去向，快速找出佔用最多的內容</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-05-disk-space-analysis.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-05-disk-space-analysis.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/tw/light-05-disk-space-analysis.jpg" width="100%" alt="MangoDisk 磁碟空間分析介面">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>啟動項目管理</strong><br>
      <sub>減少不必要的開機啟動程式，加快登入並降低背景占用</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-06-startup-items.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-06-startup-items.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/tw/light-06-startup-items.jpg" width="100%" alt="MangoDisk 啟動項目管理介面">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>解除安裝應用程式與殘留清理</strong><br>
      <sub>解除安裝應用程式並清除相關殘留，釋放更多磁碟空間</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-04-app-uninstaller.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-04-app-uninstaller.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/tw/light-04-app-uninstaller.jpg" width="100%" alt="MangoDisk 解除安裝應用程式介面">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>系統最佳化</strong><br>
      <sub>一鍵改善效能、隱私與使用體驗，讓系統運作更流暢</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-07-system-optimization.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-07-system-optimization.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/tw/light-07-system-optimization.jpg" width="100%" alt="MangoDisk 系統最佳化介面">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>系統維護</strong><br>
      <sub>快速解決常見系統問題，讓電腦恢復正常</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-08-system-maintenance.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-08-system-maintenance.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/tw/light-08-system-maintenance.jpg" width="100%" alt="MangoDisk 系統維護介面">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>隱私清理</strong><br>
      <sub>減少活動痕跡殘留，讓日常隱私更有保障</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/tw/dark-09-privacy-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/tw/light-09-privacy-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/tw/light-09-privacy-cleanup.jpg" width="100%" alt="MangoDisk 隱私清理介面">
      </picture>
    </td>
  </tr>
</table>

## 安裝與使用

目前版本支援以下系統：

- **macOS**：macOS Monterey 12.5 或更新版本。
- **Windows**：64 位元 Windows 10 或更新版本。

macOS 使用者可以透過 Homebrew 快速安裝：

```sh
brew install --cask harry0703/tap/mangodisk
```

Windows 使用者可以在 PowerShell 中快速安裝：

```powershell
irm "https://get.mangodisk.app" | iex
```

也可以前往 [MangoDisk 官網](https://mangodisk.app/tw) 或 [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest) 下載最新版：

- **macOS**：開啟 DMG，將 MangoDisk 拖入「應用程式」資料夾。
- **Windows**：執行 Windows 安裝程式並按提示完成安裝。

> [!CAUTION]
>
> 1. 清理、永久刪除與解除安裝可能無法復原。執行前請確認內容，並妥善備份重要資料。
> 2. 執行系統維護、變更啟動項目或系統設定前，請先確認相關項目的用途與影響。
> 3. 部分系統最佳化可能影響安全性、隱私、電池續航力或系統更新策略。

## CLI 快速上手

macOS 使用者可以透過 Homebrew 安裝獨立 CLI：

```sh
brew install harry0703/tap/mangodisk-cli
```

Windows 使用者可以在 PowerShell 中安裝最新版 CLI：

```powershell
irm "https://get.mangodisk.app/cli" | iex
```

安裝完成後，如果暫時找不到 `mangodisk`，請重新開啟終端機，再檢查版本：

```sh
mangodisk --version
```

CLI 與桌面應用程式使用同一套安全清理引擎，可以使用以下命令：

```sh
# 只掃描並展示可清理內容
mangodisk clean

# 套用與桌面應用程式相同的智慧建議
mangodisk clean --apply

# 預覽全部可選內容，不實際刪除
mangodisk clean --apply --selection all --dry-run

# 輸出便於腳本處理的 JSON
mangodisk clean --format json --no-progress
```

`mangodisk clean` 預設只會掃描，不會修改檔案。在非互動式環境執行實際清理時，還必須傳入 `--yes` 明確確認；完整選項請執行：

```sh
mangodisk clean --help
```

## 從原始碼建置

### 環境要求

- Node.js 24 LTS
- pnpm 11.13.1
- Rust 穩定版工具鏈
- macOS：Xcode Command Line Tools
- Windows：Visual Studio 2022 Build Tools，並安裝「使用 C++ 的桌面開發」
- Windows：Microsoft Edge WebView2 Runtime

各平台的相依套件需求請參考 [Tauri 2 前置需求](https://v2.tauri.app/start/prerequisites/)。

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

本機建置產物不包含 MangoDisk 正式發布流程提供的簽名、公證和更新元資料，僅用於開發與驗證。

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
