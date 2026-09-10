<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk アプリアイコン"> MangoDisk
</h1>

<p align="center">macOS・Windows 対応のディスククリーンアップ、容量分析、プライバシー保護、システム最適化ツール</p>

<p align="center">
<a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · 日本語 · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
<a href="https://github.com/harry0703/MangoDisk/releases/latest"><img alt="最新リリース" src="https://img.shields.io/github/v/release/harry0703/MangoDisk?display_name=tag&sort=semver"></a>
  <img alt="macOS 対応" src="https://img.shields.io/badge/macOS-supported-111827?logo=apple&logoColor=white">
  <img alt="Windows 対応" src="https://img.shields.io/badge/Windows-supported-2563eb?logo=windows&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri&logoColor=white">
  <img alt="Rust Core" src="https://img.shields.io/badge/core-Rust-b7410e?logo=rust&logoColor=white">
</p>

<p align="center">
  <a href="https://mangodisk.app/ja">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/readme/ja-dark.jpg">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/readme/ja-light.jpg">
      <img src="https://assets.mangodisk.app/images/readme/ja-light.jpg" width="1200" alt="MangoDisk のディスククリーンアップ、容量分析、プライバシー保護、システム最適化">
    </picture>
  </a>
</p>

## MangoDisk でできること

> **ストレージ**

### 1. ディープクリーン

システム、アプリ、開発ツール、ローカルプロジェクトに散らばるクリーンアップ対象を一度に見つけます。場所ごとに探す手間を省き、解放できる容量をカテゴリ別にまとめて確認できます。

- **システムとユーザーのキャッシュ**：システムの一時ファイルや診断データ、再作成可能なキャッシュを整理し、空き容量を増やします。
- **アプリキャッシュ**：アプリのキャッシュ、ログ、更新パッケージ、一時データが蓄積してストレージを圧迫するのを防ぎます。
- **ブラウザデータ**：Chrome、Edge、Firefox、Brave、Arc、Opera などのキャッシュや一時的なウェブデータが使用している容量を取り戻します。
- **開発ツールと Xcode**：パッケージマネージャー、IDE、コンパイルキャッシュ、Xcode の開発データが占める大きな容量をすばやく解放します。
- **コンテナキャッシュ**：Docker などの未使用のビルドキャッシュや再作成可能なデータから空き容量を取り戻します。
- **プロジェクトのビルド成果物**：Node.js、Rust、Gradle、Swift、Python、.NET、Godot、CMake などのプロジェクトにある再作成可能な依存関係、キャッシュ、ビルドディレクトリから空き容量を取り戻します。
- **AI モデルとキャッシュ**：容量の大きいローカル AI モデル、ダウンロードキャッシュ、一時転送ファイルをすばやく見つけます。
- **アプリ容量の最適化**：普段の利用に影響を与えずに対応アプリを小さくし、ディスクの空き容量を増やします。

スマート選択なら、安全性に配慮した候補をすばやく選べます。項目ごとの確認や事前の空き容量見積もりもできるため、納得したうえで安心して整理できます。

### 2. 大容量ファイル

容量を多く使う大きなファイルをすばやく見つけ、古いインストーラー、動画、アーカイブなどから空き容量を取り戻せます。フォルダーを一つずつ探す必要はありません。

### 3. 重複ファイル

重複したコピーが占める容量を取り戻しながら、同じ名前でも内容が異なるファイルの誤判定を防ぎます。スマート選択では各グループに少なくとも 1 ファイルを残すため、安心して整理できます。

### 4. ディスク容量分析

ディスク容量が何に使われているかをひと目で把握できます。ツリーマップとリストをたどって容量の大きいフォルダーやファイルを見つけ、むやみな削除を避けられます。

> **プライバシーとセキュリティ**

### 5. プライバシークリーンアップ

閲覧履歴、検索履歴、Cookie、最近使った項目、クリップボードの内容がパソコンに残り続けるのを防ぎます。利用傾向やアクセス履歴、ログイン状態を他人に見られるリスクを抑え、日々のプライバシーを手軽に管理できます。

> **システムツール**

### 6. アプリのアンインストールとクリーンアップ

アプリ本体と関連するキャッシュ、設定、残存ファイルをまとめて削除し、アンインストール後も残る不要なデータから空き容量を取り戻します。個人ファイルの可能性があるデータは慎重に扱い、誤削除のリスクを抑えます。

### 7. スタートアップ項目の管理

不要な起動待ちとバックグラウンド負荷を減らし、パソコンの起動を速くして、動作を軽快にします。必要になったときはいつでも再度有効にできます。

### 8. システム最適化

動作を重くしたり日常の利用を妨げたりする不要な設定を見直します。パフォーマンス、プライバシー、使い方のバランスを整え、より軽快で快適に使える環境にします。

### 9. システムメンテナンス

検索でファイルが見つからない、アイコンが正しく表示されない、音が出ない、ネットワークにつながらないといったよくある問題をすばやく解消します。手作業で原因を探したり複雑なコマンドを使ったりする手間を省き、いつもの状態に戻せます。

> **操作履歴**

### 10. 操作履歴

クリーンアップやシステム設定の変更をあとから確認できます。増えた空き容量、完了した操作、対応が必要な問題をひと目で把握できます。

## AI 解説

> バージョン 1.1.0 から利用できます

項目の用途や、操作すると何が変わるのかがわからないときに役立つ機能です。項目の説明と現在のスキャン結果をもとに、AI が用途や注意点をわかりやすく解説します。あちこち調べる手間を省き、操作が必要かどうかの判断に役立ちます。

ディープクリーンの内蔵ルール、プライバシークリーンアップ、スタートアップ項目の管理、システム最適化、システムメンテナンスに対応し、各項目から解説を確認できます。

公式版では毎日一定回数まで無料で利用でき、お使いの AI サービスにも接続できます。解説はあくまで参考情報であり、実際に操作するかどうかは自分で決められます。

## 安全性とルール

> [!IMPORTANT]
> **MangoDisk は、空き容量の確保よりもデータの安全性を優先します。**
> クリーンアップルールとシステム最適化項目は、安全な範囲を明確にし、実際のシステムで検証したものだけを製品版に採用しています。

MangoDisk はデフォルトで読み取り専用のスキャンを行います。クリーンアップ、削除、アンインストール、システム設定の変更前に内容を表示し、ユーザーの確認を求めます。操作結果は履歴に保存されます。

システム最適化では、内蔵の検証済み設定だけを使用します。任意のレジストリパス、ターミナルコマンド、スクリプトを実行することはありません。変更後は設定を再度読み取り、影響の大きい項目や管理者権限、再起動が必要な項目を事前にお知らせします。

クリーンアップルールは MangoDisk が独自に管理しています。サードパーティ製プロジェクトは調査の手がかりとしてのみ参照し、信頼できる情報源、安全な範囲、実際のシステムでの動作を確認してから採用します。安全性を明確に確認できない内容はルールに含めません。

ルールライブラリと変更履歴はすべて公開されています：[MangoDisk のクリーンアップルールを見る](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules)。

## スクリーンショット

<p align="center">
<strong>ディープクリーン</strong><br>
<sub>システム、アプリ、開発ツール、プロジェクトのクリーンアップ対象をまとめて見つけ、空き容量を増やします</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-01-deep-cleanup.jpg">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-01-deep-cleanup.jpg">
    <img src="https://assets.mangodisk.app/images/screenshots/ja/light-01-deep-cleanup.jpg" width="1200" alt="MangoDisk ディープクリーン画面">
  </picture>
</p>

<table>
  <tr>
    <td width="50%" align="center">
<strong>大容量ファイル</strong><br>
<sub>フォルダーを一つずつたどらずに、容量を多く使うファイルを見つけます</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-02-large-file-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-02-large-file-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/ja/light-02-large-file-cleanup.jpg" width="100%" alt="MangoDisk 大容量ファイル画面">
      </picture>
    </td>
    <td width="50%" align="center">
<strong>重複ファイル</strong><br>
<sub>完全に同じファイルを安全に整理し、各グループに少なくとも 1 ファイルを残します</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-03-duplicate-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-03-duplicate-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/ja/light-03-duplicate-cleanup.jpg" width="100%" alt="MangoDisk 重複ファイル画面">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
<strong>ディスク容量分析</strong><br>
<sub>容量の使い道をひと目で把握し、サイズの大きいデータをすばやく見つけます</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-05-disk-space-analysis.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-05-disk-space-analysis.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/ja/light-05-disk-space-analysis.jpg" width="100%" alt="MangoDisk ディスク容量分析画面">
      </picture>
    </td>
    <td width="50%" align="center">
<strong>スタートアップ項目の管理</strong><br>
<sub>不要な自動起動を減らし、サインインを速くしてバックグラウンドの負荷を抑えます</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-06-startup-items.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-06-startup-items.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/ja/light-06-startup-items.jpg" width="100%" alt="MangoDisk スタートアップ項目管理画面">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
<strong>アプリのアンインストールとクリーンアップ</strong><br>
<sub>アプリと関連する残存ファイルをまとめて削除し、空き容量を増やします</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-04-app-uninstaller.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-04-app-uninstaller.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/ja/light-04-app-uninstaller.jpg" width="100%" alt="MangoDisk アプリアンインストーラー画面">
      </picture>
    </td>
    <td width="50%" align="center">
<strong>システム最適化</strong><br>
<sub>パフォーマンス、プライバシー、使いやすさをワンクリックで整えます</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-07-system-optimization.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-07-system-optimization.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/ja/light-07-system-optimization.jpg" width="100%" alt="MangoDisk システム最適化画面">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
<strong>システムメンテナンス</strong><br>
<sub>よくあるシステムの問題をすばやく解消し、いつもの状態に戻します</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-08-system-maintenance.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-08-system-maintenance.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/ja/light-08-system-maintenance.jpg" width="100%" alt="MangoDisk システムメンテナンス画面">
      </picture>
    </td>
    <td width="50%" align="center">
<strong>プライバシークリーンアップ</strong><br>
<sub>不要な利用履歴を減らし、日々のプライバシーを守ります</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/ja/dark-09-privacy-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/ja/light-09-privacy-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/ja/light-09-privacy-cleanup.jpg" width="100%" alt="MangoDisk プライバシークリーンアップ画面">
      </picture>
    </td>
  </tr>
</table>

## インストールと実行

現在のバージョンは、次の OS に対応しています。

- **macOS**：macOS Monterey 12.5 以降
- **Windows**：64 ビット版 Windows 10 以降

Homebrew を使って macOS に MangoDisk をインストールできます。

```sh
brew install --cask harry0703/tap/mangodisk
```

Windows では、PowerShell から MangoDisk をインストールできます。

```powershell
irm "https://get.mangodisk.app" | iex
```

または、[MangoDisk 公式サイト](https://mangodisk.app/ja) か [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest) から最新版をダウンロードできます。

- **macOS**：DMG を開き、MangoDisk を「アプリケーション」フォルダーへドラッグします。
- **Windows**：Windows インストーラーを実行し、画面の案内に従います。

> [!CAUTION]
>
> 1. クリーンアップ、完全削除、アンインストールは元に戻せない場合があります。実行前に内容を確認し、重要なデータは必ずバックアップしてください。
> 2. システムメンテナンスの実行や、スタートアップ項目・システム設定の変更前に、対象項目の目的と影響を確認してください。
> 3. システム最適化の一部は、セキュリティ、プライバシー、バッテリー駆動時間、システムアップデートの動作に影響する場合があります。

## CLI クイックスタート

Homebrew を使って macOS にスタンドアロン版 CLI をインストールできます。

```sh
brew install harry0703/tap/mangodisk-cli
```

Windows では、PowerShell から最新版の CLI をインストールできます。

```powershell
irm "https://get.mangodisk.app/cli" | iex
```

インストール後に `mangodisk` コマンドが見つからない場合は、新しいターミナルを開いてからバージョンを確認してください。

```sh
mangodisk --version
```

CLI はデスクトップアプリと同じ、安全性を重視したクリーンアップエンジンを使用します。

```sh
# 変更を加えず、削除可能な内容をスキャンして表示
mangodisk clean

# デスクトップアプリと同じスマート選択を適用
mangodisk clean --apply

# ファイルを削除せず、選択可能な内容をすべてプレビュー
mangodisk clean --apply --selection all --dry-run

# 機械処理しやすい JSON 形式で出力
mangodisk clean --format json --no-progress
```

`mangodisk clean` は既定でスキャンのみを行い、ファイルを変更しません。非対話環境で実際にクリーンアップする場合は、明示的な確認として `--yes` も指定する必要があります。利用できるすべてのオプションは次のコマンドで確認できます。

```sh
mangodisk clean --help
```

## ソースからビルド

### 前提条件

- Node.js 24 LTS
- pnpm 11.13.1
- 安定版 Rust
- macOS：Xcode Command Line Tools
- Windows：Visual Studio 2022 Build Tools（**C++ によるデスクトップ開発**を含む）
- Windows：Microsoft Edge WebView2 Runtime

詳細なプラットフォーム要件については、[Tauri 2 の前提条件](https://v2.tauri.app/start/prerequisites/) を参照してください。

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

ローカルビルドには、MangoDisk の公式リリースで提供される署名、公証、アップデート用メタデータは含まれません。開発と検証にのみ使用してください。

## 貢献

不具合報告、クリーンアップルール、修正、新機能の提案を歓迎します。作業を始める前に [`CONTRIBUTING.md`](CONTRIBUTING.md) と [`AGENTS.md`](AGENTS.md) をお読みください。

通常のクリーンアップ対象は、ビルド時に検証される宣言的な TOML ルールとして追加してください。ルールスキーマ、セーフティ制約、検証手順については [`src-tauri/crates/mangodisk-core/rules/README.md`](src-tauri/crates/mangodisk-core/rules/README.md) を参照してください。

変更を提出する前に、少なくとも次を実行してください:

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
