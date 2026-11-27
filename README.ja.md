<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk アプリアイコン"> MangoDisk
</h1>

<p align="center"><b>macOS</b>・<b>Windows</b>・<b>Linux</b> 向けのディスククリーンアップ・ストレージ分析ツール</p>

<p align="center">
<a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · 日本語 · <a href="README.ko.md">한국어</a> · <a href="README.ru.md">Русский</a>
</p>

<p align="center">
<a href="https://github.com/harry0703/MangoDisk/releases/latest"><img alt="最新リリース" src="https://img.shields.io/github/v/release/harry0703/MangoDisk?display_name=tag&sort=semver"></a>
  <img alt="macOS 対応" src="https://img.shields.io/badge/macOS-supported-111827?logo=apple&logoColor=white">
  <img alt="Windows 対応" src="https://img.shields.io/badge/Windows-supported-2563eb?logo=windows&logoColor=white">
  <img alt="Linux 対応" src="https://img.shields.io/badge/Linux-supported-f59e0b?logo=linux&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri&logoColor=white">
  <img alt="Rust Core" src="https://img.shields.io/badge/core-Rust-b7410e?logo=rust&logoColor=white">
</p>

<p align="center">
  <a href="https://mangodisk.app/ja">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-00-overview.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-00-overview.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/ja/light-00-overview.jpg?ver=2.1.1" width="1200" alt="MangoDisk の概要画面">
    </picture>
  </a>
</p>

## MangoDisk でできること

> **ストレージ**

### 1. ディープクリーン

普段使うアプリや開発ツール、AI ツールが残すキャッシュ、ビルド成果物、ローカルの AI モデルをまとめて探せます。隠しフォルダーを一つずつ確認する手間が省けます。

- **システムとユーザーのキャッシュ**：システムの一時ファイル、診断データ、再生成できるキャッシュを削除します。
- **アプリのキャッシュ**：アプリが残したキャッシュ、ログ、更新パッケージ、一時ファイルを整理します。
- **ブラウザーのデータ**：Chrome、Edge、Firefox、Brave、Arc、Opera などのキャッシュや一時データを削除します。
- **開発ツールと Xcode**：パッケージマネージャー、IDE、コンパイラーのキャッシュや Xcode の開発データを整理します。
- **コンテナーのキャッシュ**：Docker などで使われていないビルドキャッシュや再生成できるデータを削除します。
- **プロジェクトのビルド成果物**：Node.js、Rust、Gradle、Swift、Python、.NET、Godot、CMake などのプロジェクトで再生成できる依存関係、キャッシュ、ビルドディレクトリを整理します。
- **AI ツール**：ChatGPT、Codex、Claude などが残したキャッシュ、古いログ、ダウンロード時の一時ファイルを削除します。
- **AI モデル**：Ollama、LM Studio、Hugging Face、ComfyUI などのローカルモデルが使う容量を確認し、不要なモデルを手動で選んで削除できます。再び使う場合はダウンロードし直してください。
- **アプリの最適化**：対応アプリを引き続き使える状態のまま、アプリのサイズを小さくします。

おすすめの項目をまとめて選ぶことも、一つずつ確認することもできます。どのくらい空き容量が増えるかは実行前に確認できます。

### 2. 大容量ファイル

古いインストーラー、動画、アーカイブなど、容量を多く使うファイルをすばやく見つけられます。

### 3. 重複ファイル

ファイル名に関係なく、内容が完全に一致するファイルを探します。自動選択では、グループごとに少なくとも 1 つを残します。

### 4. ディスク容量分析

#### 容量の分布

ストレージの使い道を**ツリーマップ**や**サンバーストチャート**で確認できます。表示する階層の深さを変えたり、ファイル一覧からたどったりして、容量を多く使うフォルダーやファイルを見つけられます。

#### 容量の変化

空き容量が急に減ったときは、スキャン履歴を有効にし、同じディスクやフォルダーの 2 回分の結果を比べてみてください。容量が増減したファイルやフォルダーを確認できます。クリーンアップ後に再スキャンすれば、どこで空き容量が増えたかもわかります。

> **プライバシーとセキュリティ**

### 5. プライバシークリーンアップ

閲覧・検索履歴、Cookie、最近使った項目、クリップボードの内容を削除し、ブラウザーやアプリ、システムに残る利用の痕跡を減らします。

> **システムツール**

### 6. アプリのアンインストールとクリーンアップ

アプリと一緒に、関連するキャッシュや設定、残ったファイルも整理できます。個人ファイルを含む可能性がある場所は、誤って削除しないよう慎重に扱います。

### 7. スタートアップ項目の管理

不要な自動起動を無効にして、起動時の待ち時間やバックグラウンドの負荷を減らします。必要になったら再度有効にできます。

### 8. システム最適化

動作の軽さ、プライバシー、普段の使い方に合わせて、システム設定を見直せます。

### 9. システムメンテナンス

検索が動かない、アイコンが正しく表示されない、音が出ない、ネットワークにつながらないといった問題に対処できます。コマンドの入力や手作業での切り分けは不要です。

### 10. 右クリックメニュー（Windows）

メニューの場所やアプリ別に項目を確認し、対応する項目を無効化、復元、削除できます。ファイルの種類ごとの調整にも対応しています。

パスのコピーやターミナルを開く操作、システムツールへのショートカットも追加できます。

> **操作履歴**

### 11. 操作履歴

クリーンアップや設定変更の履歴から、確保できた空き容量、完了した操作、対処が必要な問題を確認できます。

## リソースの使用状況とメモリ管理

CPU、GPU、メモリ、ネットワーク、ディスクの使用状況をリアルタイムで確認できます。対応機器では温度、消費電力、バッテリーの状態、ディスクの健全性も確認できます。

負荷の高いアプリを見つけて終了したり、手動または自動でメモリを解放したりできます。

メインウィンドウを開かなくても、メニューバー、タスクバー、システムトレイ、デスクトップのフローティングウィンドウから使用状況を確認できます。表示項目や外観はカスタマイズできます。

## AI 解説

項目の用途や操作による影響が気になるときは、項目の横から AI 解説を開けます。項目の説明とスキャン結果をもとに、用途や注意点をわかりやすく説明します。ディープクリーンの組み込みルール、プライバシークリーンアップ、スタートアップ項目の管理、システム最適化、システムメンテナンスで利用できます。

## 安全性とルール

> [!IMPORTANT]
> **MangoDisk はデータの安全性を優先します。**
> 安全に実行できる条件を明確にし、実際のシステムで検証したクリーンアップルールとシステム最適化項目を正式版に採用します。

スキャンは標準で読み取り専用です。クリーンアップ、削除、アンインストール、システム設定の変更は、内容を確認してから実行します。結果は操作履歴に保存されます。

システム最適化で変更できるのは、検証済みの組み込み設定に限られます。変更後は設定を読み直して結果を確認し、リスクの高い項目や管理者権限・再起動が必要な項目は実行前にお知らせします。

クリーンアップルールは MangoDisk が独自に管理しています。他のプロジェクトは調査の手がかりとし、新しいルールは信頼できる情報源と安全に実行できる条件を確認したうえで、実際のシステムで検証します。

ルールと変更履歴はすべて公開しています。[MangoDisk のクリーンアップルールを見る](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules)

## スクリーンショット

<p align="center">
<strong>ディープクリーン</strong><br>
<sub>たまったキャッシュやビルド成果物、不要な AI モデルを整理</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-01-deep-cleanup.jpg?ver=2.1.1">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-01-deep-cleanup.jpg?ver=2.1.1">
    <img src="https://assets.mangodisk.app/images/screenshots/ja/light-01-deep-cleanup.jpg?ver=2.1.1" width="1200" alt="MangoDisk ディープクリーン画面">
  </picture>
</p>

<p align="center">
  <strong>容量の分布</strong><br>
  <sub>容量の内訳を見ながら、大きなファイルやフォルダーを探す</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/ja/light-05-disk-space-analysis.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-05-disk-space-analysis.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-05-disk-space-analysis.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/ja/light-05-disk-space-analysis.jpg?ver=2.1.1" width="1200" alt="MangoDisk ディスク容量分析画面">
    </picture>
  </a>
</p>

<p align="center">
  <strong>容量の変化</strong><br>
  <sub>2 回のスキャン結果を比べて、容量の増減を確認</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/ja/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/ja/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" width="1200" alt="MangoDisk の容量の変化画面に表示されたスキャン結果の比較">
    </picture>
  </a>
</p>

<p align="center">
  <strong>システムモニター</strong><br>
  <sub>CPU、メモリ、ネットワーク、ディスクの使用状況をひと目で確認</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/ja/light-system-monitoring-memory.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-system-monitoring-memory.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-system-monitoring-memory.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/ja/light-system-monitoring-memory.jpg?ver=2.1.1" width="720" alt="MangoDisk のシステムモニターとメモリ管理画面">
    </picture>
  </a>
</p>

<table>
  <tr>
    <td width="50%" align="center">
<strong>大容量ファイル</strong><br>
<sub>容量を多く使うファイルをすばやく発見</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/ja/light-02-large-file-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-02-large-file-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-02-large-file-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/ja/light-02-large-file-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 大容量ファイル画面">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
<strong>重複ファイル</strong><br>
<sub>内容が同じファイルを整理し、各グループに少なくとも 1 つを残す</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/ja/light-03-duplicate-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-03-duplicate-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-03-duplicate-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/ja/light-03-duplicate-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 重複ファイル画面">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
<strong>スタートアップ項目の管理</strong><br>
<sub>不要な自動起動を無効化</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/ja/light-06-startup-items.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-06-startup-items.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-06-startup-items.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/ja/light-06-startup-items.jpg?ver=2.1.1" width="100%" alt="MangoDisk スタートアップ項目管理画面">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
<strong>アプリのアンインストールとクリーンアップ</strong><br>
<sub>アプリと一緒に、関連する残存ファイルも整理</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/ja/light-04-app-uninstaller.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-04-app-uninstaller.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-04-app-uninstaller.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/ja/light-04-app-uninstaller.jpg?ver=2.1.1" width="100%" alt="MangoDisk アプリアンインストーラー画面">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
<strong>システム最適化</strong><br>
<sub>パフォーマンスやプライバシー、普段の使い方に合わせて設定を調整</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/ja/light-07-system-optimization.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-07-system-optimization.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-07-system-optimization.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/ja/light-07-system-optimization.jpg?ver=2.1.1" width="100%" alt="MangoDisk システム最適化画面">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
<strong>システムメンテナンス</strong><br>
<sub>よくあるシステムの不具合を解消</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/ja/light-08-system-maintenance.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-08-system-maintenance.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-08-system-maintenance.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/ja/light-08-system-maintenance.jpg?ver=2.1.1" width="100%" alt="MangoDisk システムメンテナンス画面">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
<strong>プライバシークリーンアップ</strong><br>
<sub>ブラウザーやアプリ、システムに残った利用の痕跡を削除</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/ja/light-09-privacy-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-09-privacy-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-09-privacy-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/ja/light-09-privacy-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk プライバシークリーンアップ画面">
        </picture>
      </a>
    </td>
  </tr>
</table>

## ご利用前の注意事項

> [!CAUTION]
>
> 1. クリーンアップ、完全削除、アンインストールは元に戻せない場合があります。実行前に選択内容を確認し、大切なデータをバックアップしてください。
> 2. システムメンテナンスを実行したり、スタートアップ項目やシステム設定を変更したりする前に、項目の用途と影響を確認してください。
> 3. システム最適化の項目によっては、セキュリティ、プライバシー、バッテリー駆動時間、システムの更新に影響することがあります。

## デスクトップ版

[公式ダウンロードページ](https://mangodisk.app/ja/download) または [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest) から、お使いの OS に合ったファイルをダウンロードし、以下の手順でインストールしてください。

### macOS

**動作環境：** macOS Monterey 12.5 以降。

**Homebrew でインストール：**

```sh
brew install --cask harry0703/tap/mangodisk
```

**手動でインストール：** [公式ダウンロードページ](https://mangodisk.app/ja/download) から DMG をダウンロードして開き、MangoDisk を「アプリケーション」フォルダーにドラッグします。

### Windows

**動作環境：** 64 ビット版 Windows 10 以降。

**PowerShell でインストール：**

```powershell
irm https://get.mangodisk.app | iex
```

**WinGet でインストール（公式ソース）：**

```powershell
winget install --id MangoDisk.MangoDisk --exact --source winget
```

**手動でインストール：** [公式ダウンロードページ](https://mangodisk.app/ja/download) から Windows 用インストーラーをダウンロードし、画面の案内に従ってインストールします。

### Linux

**推奨環境：** Ubuntu 22.04 LTS 以降。x64 と ARM64 に対応しています。

`.deb` パッケージと AppImage を提供しています。ほかの Linux ディストリビューションでの動作は、システムライブラリやデスクトップ環境によって異なります。

**Homebrew でインストール：** 新しいバージョンの Homebrew とデスクトップ環境が必要です。

```sh
brew install --cask harry0703/tap/mangodisk
```

**ターミナルからインストール（Debian/Ubuntu）：** アーキテクチャを自動判別し、対応する最新版の `.deb` パッケージをインストールします。

```sh
curl -fsSL https://get.mangodisk.app/linux | bash
```

**手動でインストール：** [公式ダウンロードページ](https://mangodisk.app/ja/download) で、お使いのアーキテクチャに合ったファイルを選んでください。

- **Debian/Ubuntu**：アーキテクチャに合った `.deb` パッケージをインストールします。
- **ほかのディストリビューション**：AppImage をお試しください。実行権限を付けてから起動してください。

## コマンドライン版（CLI）

ターミナルやスクリプトから利用できます。クリーンアップの処理はデスクトップ版と共通です。

### macOS

**Homebrew でインストール：**

```sh
brew install harry0703/tap/mangodisk-cli
```

### Windows

**PowerShell でインストール：**

```powershell
irm https://get.mangodisk.app/cli | iex
```

**WinGet でインストール（公式ソース）：**

```powershell
winget install --id MangoDisk.CLI --exact --source winget
```

### Linux

Linux 向けスタンドアロン版 CLI のビルド済みパッケージは、現在配布していません。下記の [ソースからビルド](#ソースからビルド) の手順でビルドできます。

### 使用例

インストール後に `mangodisk` コマンドが見つからない場合は、新しいターミナルを開いてからバージョンを確認してください。

```sh
mangodisk --version
```

よく使うコマンド：

```sh
# 変更を加えず、削除可能な内容をスキャンして表示
mangodisk clean

# デスクトップ版と同じスマート選択を適用
mangodisk clean --apply

# ファイルを削除せず、選択可能な内容をすべてプレビュー
mangodisk clean --apply --selection all --dry-run

# 機械処理しやすい JSON 形式で出力
mangodisk clean --format json --no-progress
```

`mangodisk clean` は標準で読み取り専用のスキャンを行います。非対話環境でクリーンアップを実行するには、確認のために `--yes` を指定してください。オプション一覧は次のコマンドで確認できます。

```sh
mangodisk clean --help
```

## ソースからビルド

### 前提条件

- Node.js 24 LTS
- pnpm 11.13.1
- 安定版 Rust

プラットフォームごとの依存パッケージについては、[Tauri 2 の前提条件](https://v2.tauri.app/start/prerequisites/) を参照してください。

### ソースを取得してデスクトップアプリを実行

```sh
git clone https://github.com/harry0703/MangoDisk.git
cd MangoDisk
pnpm install --frozen-lockfile
pnpm tauri:dev
```

### 必要なチェックを実行

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

### デスクトップインストーラーをビルド

```sh
pnpm tauri:build
```

### CLI をビルド

```sh
pnpm cli:build
```

ローカルビルドは開発・検証用です。署名、公証、アップデート用メタデータは公式リリース時に提供されます。

## MangoDisk を支援する

MangoDisk は無料で使える、広告のないオープンソースソフトウェアです。基本機能は GPL-3.0 に基づき、個人利用・商用利用ともに無料です。開発を応援したい方や公式クラウド AI を使いたい方は、[サポーターのご案内](https://mangodisk.app/ja/supporter)をご覧ください。

## 貢献

不具合の報告や、クリーンアップルールの追加、不具合の修正、新機能の実装を歓迎します。作業を始める前に [`CONTRIBUTING.md`](CONTRIBUTING.md) と [`AGENTS.md`](AGENTS.md) をお読みください。

通常のクリーンアップ対象は、ビルド時に検証される宣言的な TOML ルールとして追加してください。ルールスキーマ、安全上の制約、検証手順については [`src-tauri/crates/mangodisk-core/rules/README.md`](src-tauri/crates/mangodisk-core/rules/README.md) を参照してください。

変更を提出する前に、少なくとも次のコマンドを実行してください。

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

セキュリティ上の問題は、[`SECURITY.md`](SECURITY.md) の案内に従って GitHub Security Advisories から非公開で報告してください。公開 Issue には投稿しないでください。

## 技術スタック

- [Tauri 2](https://tauri.app/): デスクトップランタイムおよびシステム統合
- [Rust](https://www.rust-lang.org/): スキャン、ファイルシステムアクセス、安全性の検証、クリーンアップ実行
- [Vue 3](https://vuejs.org/) および [TypeScript](https://www.typescriptlang.org/): デスクトップユーザーインターフェース

## ライセンス

MangoDisk は [GNU General Public License v3.0](https://github.com/harry0703/MangoDisk/blob/main/LICENSE) に基づくオープンソースソフトウェアです。サードパーティ製コンポーネントには、それぞれのライセンスが適用されます。
