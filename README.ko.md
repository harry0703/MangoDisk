<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk application icon"> MangoDisk
</h1>

<p align="center">macOS와 Windows를 위한 디스크 정리, 저장 공간 분석, 개인정보 보호, 시스템 최적화</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · 한국어
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
      <img src="https://assets.mangodisk.app/images/readme/en-light.jpg" width="1200" alt="MangoDisk 디스크 정리, 저장 공간 분석, 개인정보 보호, 시스템 최적화">
    </picture>
  </a>
</p>

## MangoDisk로 할 수 있는 일

> **저장 공간**

### 1. 심층 정리

시스템, 앱, 개발자 도구, 로컬 프로젝트 곳곳에 흩어진 정리 가능한 콘텐츠를 한 번의 스캔으로 찾습니다. 위치마다 직접 확인할 필요 없이 MangoDisk가 회수 가능한 공간별로 결과를 묶어 보여줍니다:

- **시스템 및 사용자 캐시**: 시간이 지나며 쌓인 시스템 임시 파일, 진단 데이터, 다시 만들 수 있는 캐시가 차지한 공간을 회수합니다.
- **앱 캐시**: 앱 캐시, 로그, 업데이트 패키지, 임시 콘텐츠가 조용히 저장 공간을 잠식하지 않게 합니다.
- **브라우저 데이터**: Chrome, Edge, Firefox, Brave, Arc, Opera 등 브라우저의 캐시와 임시 웹 데이터가 쓰는 공간을 회수합니다.
- **개발자 도구 및 Xcode**: 패키지 관리자, IDE, 컴파일러 캐시, Xcode 개발 데이터가 차지한 큰 용량을 빠르게 되찾습니다.
- **컨테이너 캐시**: Docker 등 컨테이너 도구의 사용하지 않는 빌드 캐시와 다시 만들 수 있는 데이터가 쓰는 공간을 확보합니다.
- **프로젝트 빌드 산출물**: Node.js, Rust, Gradle, Swift, Python, .NET, Godot, CMake 등 프로젝트의 다시 만들 수 있는 의존성, 캐시, 빌드 디렉터리가 쓰는 공간을 되찾습니다.
- **AI 모델 및 캐시**: 대용량 로컬 AI 모델, 다운로드 캐시, 임시 전송 파일을 빠르게 찾아냅니다.
- **앱 최적화**: 지원되는 앱을 정상 사용에 영향 없이 줄여 디스크 여유 공간을 늘립니다.

스마트 추천이 안전한 선택을 빠르게 돕습니다. 항목을 하나씩 검토하고 회수 예상 공간을 미리 확인할 수 있어 모든 정리를 예측 가능하게, 직접 통제하며 진행할 수 있습니다.

### 2. 대용량 파일 정리

폴더를 일일이 뒤지지 않고 가장 큰 파일을 빠르게 찾아 오래된 설치 파일, 동영상, 압축 파일 등 부피 큰 콘텐츠가 쓰는 공간을 회수합니다.

### 3. 중복 파일 정리

이름이 같다는 이유만으로 중복으로 보지 않고, 실제 중복 복사본이 차지한 공간을 회수합니다. 스마트 선택은 각 그룹에 파일을 하나 이상 남기므로 정리가 간편하고 안전합니다.

### 4. 디스크 공간 분석

저장 공간이 어디에 쓰이는지 한눈에 봅니다. 트리맵과 목록으로 파고들어 가장 큰 폴더와 파일을 찾고, 무작정 정리하는 대신 정확히 짚어냅니다.

> **개인정보 보호 및 보안**

### 5. 개인정보 정리

방문 기록, 검색어, 쿠키, 최근 항목, 클립보드 데이터가 컴퓨터에 남지 않게 합니다. 브라우저, 앱, 시스템이 남긴 흔적을 지워 활동 노출을 줄이고 일상적인 개인정보 관리를 쉽게 합니다.

> **시스템 도구**

### 6. 앱 제거 및 정리

앱을 제거하면서 관련 캐시, 설정, 잔여 파일까지 지워 실제로 공간을 되돌려 받습니다. 개인 파일일 가능성이 있는 항목은 실수로 삭제되지 않도록 신중하게 다룹니다.

### 7. 시작 항목 관리

불필요한 시작 지연과 백그라운드 리소스 사용을 줄여 컴퓨터가 더 빨리 켜지고 가볍게 느껴지게 합니다. 다시 필요하면 언제든 항목을 켤 수 있습니다.

### 8. 시스템 최적화

시스템을 느리게 하거나 방해하는 불필요한 설정을 줄입니다. 성능, 개인정보 보호, 개인 취향의 균형을 맞춰 컴퓨터를 더 빠르고 쓰기 편하게 만듭니다.

### 9. 시스템 유지 관리

검색 결과 누락, 잘못된 아이콘, 소리 안 남, 네트워크 연결 실패 같은 흔한 문제를 해결책을 찾아 헤매거나 복잡한 명령을 입력하지 않고 고칩니다. 컴퓨터를 더 빨리 정상으로 되돌립니다.

> **활동**

### 10. 작업 기록

모든 정리와 시스템 변경을 명확히 기록합니다. 얼마나 많은 공간을 되찾았는지, 무엇이 성공적으로 끝났는지, 아직 확인이 필요한 것이 있는지 볼 수 있습니다.

## AI 설명

> 버전 1.1.0부터 제공

항목이 무엇을 하는지, 바꾸면 어떤 일이 생길지 확신이 없나요? AI 설명은 항목 설명과 현재 스캔 결과를 바탕으로 그 용도와 실행 전 고려할 점을 설명합니다. 찾아보는 시간은 줄이고 더 나은 선택을 하세요.

심층 정리(내장 규칙), 개인정보 정리, 시작 항목 관리, 시스템 최적화, 시스템 유지 관리의 항목에서 바로 설명을 볼 수 있습니다.

공식 릴리스에는 매일 무료 설명이 포함되며, 자신의 AI 서비스를 연결할 수도 있습니다. AI는 안내를 제공할 뿐, 어떤 작업을 실행할지는 사용자가 결정합니다.

## 안전과 규칙

> [!IMPORTANT]
> **MangoDisk는 더 많은 공간을 회수하는 것보다 데이터 안전을 우선합니다.**
> 정리 규칙과 시스템 최적화는 안전 경계가 명확히 정의되고 실제 시스템에서 검증을 통과한 뒤에만 배포됩니다.

MangoDisk는 기본적으로 읽기 전용으로 스캔합니다. 정리, 삭제, 제거, 시스템 설정 변경을 시작하기 전에 정확히 무슨 일이 일어날지 검토하고 확인할 수 있습니다. 결과는 작업 기록에 저장됩니다.

시스템 최적화는 내장된 검증 완료 설정만 사용합니다. 임의의 레지스트리 경로, 터미널 명령, 스크립트는 받지 않습니다. MangoDisk는 설정을 변경한 뒤 다시 읽어 확인하며, 영향이 큰 항목과 관리자 권한이나 재시동이 필요한 변경을 명확히 알립니다.

MangoDisk는 자체 정리 규칙을 관리합니다. 서드파티 프로젝트가 조사 단서를 줄 수는 있지만, 후보 규칙은 신뢰할 수 있는 출처, 안전 경계, 실제 시스템 동작이 검증된 뒤에만 채택됩니다. 안전 경계가 명확하지 않은 것은 제외됩니다.

전체 규칙 라이브러리와 변경 이력은 공개되어 있습니다: [MangoDisk 정리 규칙 라이브러리 보기](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules).

## 스크린샷

<p align="center">
  <strong>심층 정리</strong><br>
  <sub>시스템, 앱, 개발자 도구, 프로젝트 전반의 정리 가능한 콘텐츠를 찾아 더 많은 공간을 회수합니다</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-01-deep-cleanup.jpg">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg">
    <img src="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg" width="1200" alt="MangoDisk 심층 정리 화면">
  </picture>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>대용량 파일 정리</strong><br>
      <sub>폴더를 뒤지지 않고 가장 많은 공간을 차지하는 파일을 찾습니다</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-02-large-file-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg" width="100%" alt="MangoDisk 대용량 파일 정리 화면">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>중복 파일 정리</strong><br>
      <sub>복사본을 하나 이상 남기면서 완전히 동일한 중복 파일을 안전하게 제거합니다</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-03-duplicate-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg" width="100%" alt="MangoDisk 중복 파일 정리 화면">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>디스크 공간 분석</strong><br>
      <sub>저장 공간이 어디에 쓰이는지 보고 가장 큰 파일과 폴더를 빠르게 찾습니다</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-05-disk-space-analysis.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg" width="100%" alt="MangoDisk 디스크 공간 분석 화면">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>시작 항목 관리</strong><br>
      <sub>불필요한 시작 프로그램을 줄여 더 빠른 로그인과 더 적은 백그라운드 활동을 만듭니다</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-06-startup-items.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg" width="100%" alt="MangoDisk 시작 항목 관리 화면">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>앱 제거 및 정리</strong><br>
      <sub>앱을 제거하고 관련 잔여 파일까지 지워 더 많은 공간을 회수합니다</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-04-app-uninstaller.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg" width="100%" alt="MangoDisk 앱 제거 화면">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>시스템 최적화</strong><br>
      <sub>성능, 개인정보 보호, 일상적인 사용성을 한 번의 클릭으로 최적화합니다</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-07-system-optimization.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg" width="100%" alt="MangoDisk 시스템 최적화 화면">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>시스템 유지 관리</strong><br>
      <sub>흔한 시스템 문제를 빠르게 고쳐 컴퓨터를 정상으로 되돌립니다</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-08-system-maintenance.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg" width="100%" alt="MangoDisk 시스템 유지 관리 화면">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>개인정보 정리</strong><br>
      <sub>활동 흔적을 덜 남기고 일상적인 사용을 더 비공개로 유지합니다</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-09-privacy-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg" width="100%" alt="MangoDisk 개인정보 정리 화면">
      </picture>
    </td>
  </tr>
</table>

## 설치 및 실행

MangoDisk는 현재 다음 환경을 지원합니다:

- **macOS**: macOS Monterey 12.5 이상.
- **Windows**: 64비트 Windows 10 이상.

macOS에서는 Homebrew로 MangoDisk를 설치합니다:

```sh
brew install --cask harry0703/tap/mangodisk
```

Windows에서는 PowerShell로 MangoDisk를 설치합니다:

```powershell
irm "https://get.mangodisk.app" | iex
```

또는 [MangoDisk 웹사이트](https://mangodisk.app/)나 [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest)에서 최신 버전을 다운로드하세요:

- **macOS**: DMG를 열고 MangoDisk를 응용 프로그램 폴더로 드래그합니다.
- **Windows**: Windows 설치 프로그램을 실행하고 안내를 따릅니다.

> [!CAUTION]
>
> 1. 정리, 영구 삭제, 제거 작업은 되돌리지 못할 수 있습니다. 선택한 내용을 검토하고 중요한 데이터는 안전하게 백업해 두세요.
> 2. 시스템 유지 관리를 실행하거나 시작 항목·시스템 설정을 바꾸기 전에 그 용도와 영향을 확실히 이해하세요.
> 3. 일부 시스템 최적화는 보안, 개인정보 보호, 배터리 수명, 업데이트 동작에 영향을 줄 수 있습니다.

## CLI 빠른 시작

macOS에서는 Homebrew로 독립 CLI를 설치합니다:

```sh
brew install harry0703/tap/mangodisk-cli
```

Windows에서는 PowerShell로 최신 CLI를 설치합니다:

```powershell
irm "https://get.mangodisk.app/cli" | iex
```

설치 직후 `mangodisk` 명령을 바로 쓸 수 없으면 새 터미널을 연 뒤 설치를 확인하세요:

```sh
mangodisk --version
```

CLI는 데스크탑 앱과 같은 안전 우선 정리 엔진을 사용합니다. 다음과 같은 명령을 사용할 수 있습니다:

```sh
# 아무것도 변경하지 않고 정리 가능한 콘텐츠를 스캔해 표시
mangodisk clean

# 데스크탑 앱과 같은 스마트 추천 적용
mangodisk clean --apply

# 아무것도 삭제하지 않고 선택 가능한 모든 콘텐츠 미리 보기
mangodisk clean --apply --selection all --dry-run

# 기계가 읽을 수 있는 JSON 출력 생성
mangodisk clean --format json --no-progress
```

`mangodisk clean`은 기본적으로 스캔만 하며 파일을 수정하지 않습니다. 비대화형 환경에서 정리를 실행하려면 `--yes`를 함께 넘겨 명시적으로 확인해야 합니다. 사용 가능한 모든 옵션은 다음 명령으로 확인하세요:

```sh
mangodisk clean --help
```

## 소스에서 빌드

### 사전 요구 사항

- Node.js 24 LTS
- pnpm 11.13.1
- Rust 안정 버전
- macOS: Xcode Command Line Tools
- Windows: **C++를 사용한 데스크톱 개발** 워크로드가 포함된 Visual Studio 2022 Build Tools
- Windows: Microsoft Edge WebView2 Runtime

플랫폼별 자세한 요구 사항은 [Tauri 2 사전 요구 사항](https://v2.tauri.app/start/prerequisites/)을 참고하세요.

### 소스 받기 및 데스크탑 앱 실행

```sh
git clone https://github.com/harry0703/MangoDisk.git
cd MangoDisk
pnpm install --frozen-lockfile
pnpm tauri:dev
```

### 필수 검사 실행

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

### 데스크탑 설치 프로그램 빌드

```sh
pnpm tauri:build
```

### CLI 빌드

```sh
pnpm cli:build
```

로컬 빌드에는 공식 MangoDisk 릴리스가 제공하는 서명, 공증, 업데이트 메타데이터가 포함되지 않습니다. 로컬 개발과 검증 용도로만 사용하세요.

## 기여하기

이슈, 정리 규칙, 수정, 새 기능 모두 환영합니다. 시작하기 전에 [`CONTRIBUTING.md`](CONTRIBUTING.md)와 [`AGENTS.md`](AGENTS.md)를 읽어 주세요.

일반적인 정리 범위는 빌드 시 검증되는 선언형 TOML 규칙을 사용해야 합니다. 규칙 스키마, 안전 제약, 검증 방법은 [`src-tauri/crates/mangodisk-core/rules/README.md`](src-tauri/crates/mangodisk-core/rules/README.md)를 참고하세요.

변경 사항을 제출하기 전에 최소한 다음을 실행하세요:

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

보안 취약점은 [`SECURITY.md`](SECURITY.md)에 설명된 대로 GitHub Security Advisories를 통해 비공개로 신고해 주세요. 보안 취약점을 공개 이슈로 올리지 마세요.

## 기술 스택

- [Tauri 2](https://tauri.app/): 데스크탑 런타임 및 시스템 통합
- [Rust](https://www.rust-lang.org/): 스캔, 파일 시스템 접근, 안전 검증, 정리 실행
- [Vue 3](https://vuejs.org/)와 [TypeScript](https://www.typescriptlang.org/): 데스크탑 사용자 인터페이스

## 라이선스

MangoDisk는 [GNU General Public License v3.0](https://github.com/harry0703/MangoDisk/blob/main/LICENSE)에 따라 공개된 오픈소스입니다. 서드파티 구성 요소에는 각각의 라이선스가 적용됩니다.
