<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="MangoDisk 앱 아이콘"> MangoDisk
</h1>

<p align="center"><b>macOS</b>, <b>Windows</b>, <b>Linux</b>용 디스크 정리 및 저장 공간 분석 도구</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · 한국어 · <a href="README.ru.md">Русский</a>
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
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-00-overview.jpg?ver=2.1.1" width="1200" alt="MangoDisk 개요 화면">
    </picture>
  </a>
</p>

## MangoDisk로 할 수 있는 일

> **저장 공간**

### 1. 심층 정리

앱과 개발 도구, AI 도구가 남긴 캐시와 빌드 결과물, 로컬 모델을 한곳에서 확인할 수 있습니다. 숨겨진 폴더를 하나씩 찾아다닐 필요가 없습니다.

- **시스템 및 사용자 캐시**: 시스템 임시 파일, 진단 데이터, 다시 생성할 수 있는 캐시를 정리합니다.
- **앱 캐시**: 앱에 쌓인 캐시, 로그, 업데이트 패키지, 임시 파일을 정리합니다.
- **브라우저 데이터**: Chrome, Edge, Firefox, Brave, Arc, Opera 등에서 쌓인 캐시와 임시 웹 데이터를 지웁니다.
- **개발 도구 및 Xcode**: 패키지 관리자, IDE, 컴파일러의 캐시와 Xcode 개발 데이터를 정리합니다.
- **컨테이너 캐시**: Docker 등에서 사용하지 않는 빌드 캐시와 다시 생성할 수 있는 데이터를 정리합니다.
- **프로젝트 빌드 결과물**: Node.js, Rust, Gradle, Swift, Python, .NET, Godot, CMake 등의 프로젝트에서 다시 생성할 수 있는 의존성 파일, 캐시, 빌드 폴더를 정리합니다.
- **AI 도구**: ChatGPT, Codex, Claude 등이 남긴 캐시, 오래된 로그, 다운로드 임시 파일을 지웁니다.
- **AI 모델**: Ollama, LM Studio, Hugging Face, ComfyUI 등의 로컬 모델이 차지하는 공간을 확인하고, 필요 없는 모델을 직접 골라 삭제합니다. 나중에 다시 쓰려면 모델을 다시 내려받아야 합니다.
- **앱 최적화**: 지원하는 앱의 용량을 줄이면서 정상적으로 사용할 수 있도록 합니다.

추천 항목을 선택하거나 하나씩 검토할 수 있습니다. 정리하기 전에 얼마나 많은 공간을 확보할 수 있는지도 확인할 수 있습니다.

### 2. 대용량 파일 정리

오래된 설치 파일이나 동영상, 압축 파일처럼 공간을 많이 차지하는 파일을 빠르게 찾습니다.

### 3. 중복 파일 정리

파일 이름과 관계없이 내용이 완전히 같은 파일을 찾습니다. 자동 선택은 중복 그룹마다 복사본을 하나 이상 남깁니다.

### 4. 디스크 공간 분석

#### 공간 분포

**트리맵**과 **선버스트 차트**로 저장 공간이 어디에 쓰이는지 확인합니다. 표시할 폴더 깊이를 조절하고 파일 목록을 살펴보면 용량이 큰 폴더와 파일을 쉽게 찾을 수 있습니다.

#### 공간 변화

여유 공간이 갑자기 줄었나요? 스캔 기록을 켜고 같은 디스크나 폴더의 두 스캔 결과를 비교하면 어느 파일이나 폴더의 용량이 얼마나 달라졌는지 알 수 있습니다. 정리 후 다시 스캔하면 어느 곳에서 공간을 확보했는지도 확인할 수 있습니다.

> **개인정보 보호 및 보안**

### 5. 개인정보 정리

방문 및 검색 기록, 쿠키, 최근 사용 항목, 클립보드 내용을 지워 브라우저와 앱, 시스템에 남은 사용 흔적을 줄입니다.

> **시스템 도구**

### 6. 앱 제거 및 정리

앱을 제거하면서 관련 캐시, 설정, 남은 파일도 함께 정리합니다. 개인 파일이 있을 수 있는 위치는 실수로 삭제하는 일을 줄이도록 신중하게 처리합니다.

### 7. 시작 항목 관리

불필요한 자동 시작 항목을 꺼 부팅 대기 시간과 백그라운드 부하를 줄입니다. 필요할 때 다시 켤 수 있습니다.

### 8. 시스템 최적화

성능과 개인정보 보호, 평소 사용 방식에 맞게 시스템 설정을 조정합니다.

### 9. 시스템 유지 관리

검색이 되지 않거나 아이콘이 깨지는 문제, 소리가 나지 않거나 네트워크에 연결되지 않는 문제 등을 해결합니다. 복잡한 명령을 입력하거나 직접 원인을 찾아다닐 필요가 없습니다.

### 10. 우클릭 메뉴(Windows)

메뉴 위치나 앱별로 항목을 살펴보고, 지원하는 항목을 끄거나 복원하거나 삭제할 수 있습니다. 파일 유형별로도 조정할 수 있습니다.

경로 복사, 터미널 열기 같은 자주 쓰는 동작이나 시스템 도구 바로가기를 추가할 수 있습니다.

> **활동**

### 11. 작업 기록

정리와 시스템 변경 내역을 확인합니다. 얼마나 많은 공간을 확보했는지, 어떤 작업이 끝났는지, 처리할 문제가 남아 있는지 알 수 있습니다.

## 리소스 사용 현황과 메모리 관리

CPU, GPU, 메모리, 네트워크, 디스크 사용량을 실시간으로 확인합니다. 지원하는 기기에서는 온도, 전력 사용량, 배터리 상태, 디스크 상태도 볼 수 있습니다.

리소스를 많이 쓰는 앱을 찾아 종료하거나 메모리를 수동 또는 자동으로 확보할 수 있습니다.

기본 창을 열지 않아도 메뉴 막대, 작업 표시줄, 시스템 트레이, 바탕 화면의 플로팅 창에서 상태를 확인할 수 있습니다. 표시할 정보와 모양을 원하는 대로 바꿀 수 있습니다.

## AI 설명

항목의 용도나 변경했을 때의 영향이 궁금하다면 항목 옆에서 AI 설명을 열어 보세요. 항목 정보와 스캔 결과를 바탕으로 용도와 주의할 점을 알려 줍니다. 심층 정리의 기본 제공 규칙, 개인정보 정리, 시작 항목 관리, 시스템 최적화, 시스템 유지 관리에서 쓸 수 있습니다.

## 안전과 규칙

> [!IMPORTANT]
> **MangoDisk는 데이터 안전을 우선합니다.**
> 안전하게 실행할 수 있는 조건을 확인하고 실제 시스템에서 검증한 정리 규칙과 시스템 최적화 항목을 정식 버전에 포함합니다.

스캔은 기본적으로 읽기 전용입니다. 정리, 파일 삭제, 앱 제거, 시스템 설정 변경은 내용을 검토하고 확인한 후에 실행됩니다. 결과는 작업 기록에 저장됩니다.

시스템 최적화는 검증을 거쳐 기본 제공되는 설정만 변경합니다. 변경 후에는 설정을 다시 읽어 결과를 확인합니다. 위험도가 높거나 관리자 권한 또는 재시작이 필요한 항목은 실행 전에 알려 줍니다.

정리 규칙은 MangoDisk에서 직접 관리합니다. 다른 프로젝트는 조사에 참고하며, 새 규칙은 신뢰할 수 있는 출처와 안전하게 실행할 수 있는 조건을 확인한 뒤 실제 시스템에서 검증합니다.

정리 규칙과 변경 이력은 모두 공개되어 있습니다. [MangoDisk 정리 규칙 보기](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules)

## 스크린샷

<p align="center">
  <strong>심층 정리</strong><br>
  <sub>쌓인 캐시와 빌드 결과물, 필요 없는 AI 모델 정리</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-01-deep-cleanup.jpg?ver=2.1.1">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg?ver=2.1.1">
    <img src="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg?ver=2.1.1" width="1200" alt="MangoDisk 심층 정리 화면">
  </picture>
</p>

<p align="center">
  <strong>공간 분포</strong><br>
  <sub>저장 공간이 어디에 쓰이는지 보고 큰 파일과 폴더 찾기</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-05-disk-space-analysis.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg?ver=2.1.1" width="1200" alt="MangoDisk 디스크 공간 분석 화면">
    </picture>
  </a>
</p>

<p align="center">
  <strong>공간 변화</strong><br>
  <sub>스캔 결과를 비교해 공간 사용량 변화 확인</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" width="1200" alt="MangoDisk 공간 변화 화면의 스캔 비교 결과">
    </picture>
  </a>
</p>

<p align="center">
  <strong>시스템 모니터링</strong><br>
  <sub>CPU, 메모리, 네트워크, 디스크 상태를 한눈에 확인</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/en/light-system-monitoring-memory.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-system-monitoring-memory.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-system-monitoring-memory.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-system-monitoring-memory.jpg?ver=2.1.1" width="720" alt="MangoDisk 시스템 모니터링 및 메모리 관리 화면">
    </picture>
  </a>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>대용량 파일 정리</strong><br>
      <sub>공간을 많이 차지하는 파일을 빠르게 찾기</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-02-large-file-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 대용량 파일 정리 화면">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>중복 파일 정리</strong><br>
      <sub>내용이 같은 파일을 정리하고 그룹마다 복사본을 하나 이상 남기기</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-03-duplicate-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 중복 파일 정리 화면">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>시작 항목 관리</strong><br>
      <sub>불필요한 자동 시작 항목 끄기</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-06-startup-items.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg?ver=2.1.1" width="100%" alt="MangoDisk 시작 항목 관리 화면">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>앱 제거 및 정리</strong><br>
      <sub>앱과 함께 남은 파일 정리</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-04-app-uninstaller.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg?ver=2.1.1" width="100%" alt="MangoDisk 앱 제거 화면">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>시스템 최적화</strong><br>
      <sub>성능과 개인정보 보호, 평소 사용 방식에 맞게 시스템 설정 조정</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-07-system-optimization.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg?ver=2.1.1" width="100%" alt="MangoDisk 시스템 최적화 화면">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>시스템 유지 관리</strong><br>
      <sub>자주 발생하는 시스템 문제 해결</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-08-system-maintenance.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg?ver=2.1.1" width="100%" alt="MangoDisk 시스템 유지 관리 화면">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>개인정보 정리</strong><br>
      <sub>브라우저, 앱, 시스템에 남은 사용 흔적 지우기</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-09-privacy-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg?ver=2.1.1" width="100%" alt="MangoDisk 개인정보 정리 화면">
        </picture>
      </a>
    </td>
  </tr>
</table>

## 사용 전 주의 사항

> [!CAUTION]
>
> 1. 정리, 영구 삭제, 앱 제거는 되돌리지 못할 수 있습니다. 실행 전에 선택한 항목을 확인하고 중요한 데이터는 백업하세요.
> 2. 시스템 유지 관리를 실행하거나 시작 항목 또는 시스템 설정을 변경하기 전에 용도와 영향을 확인하세요.
> 3. 일부 시스템 최적화는 보안, 개인정보 보호, 배터리 사용 시간 또는 시스템 업데이트에 영향을 줄 수 있습니다.

## 데스크톱 버전

[공식 다운로드 페이지](https://mangodisk.app/download)나 [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest)에서 설치 파일을 다운로드한 뒤, 사용 중인 운영체제에 맞는 방법으로 설치하세요.

### macOS

**시스템 요구 사항:** macOS Monterey 12.5 이상.

**Homebrew로 설치:**

```sh
brew install --cask harry0703/tap/mangodisk
```

**직접 설치:** [공식 다운로드 페이지](https://mangodisk.app/download)에서 DMG를 다운로드해 열고, MangoDisk를 ‘응용 프로그램’ 폴더로 드래그하세요.

### Windows

**시스템 요구 사항:** 64비트 Windows 10 이상.

**PowerShell로 설치:**

```powershell
irm https://get.mangodisk.app | iex
```

**WinGet으로 설치(공식 소스):**

```powershell
winget install --id MangoDisk.MangoDisk --exact --source winget
```

**직접 설치:** [공식 다운로드 페이지](https://mangodisk.app/download)에서 Windows 설치 프로그램을 다운로드하고 화면의 안내에 따라 설치하세요.

### Linux

**권장 환경:** Ubuntu 22.04 LTS 이상. x64와 ARM64를 지원합니다.

`.deb` 패키지와 AppImage를 제공합니다. 다른 Linux 배포판에서의 호환성은 시스템 라이브러리와 데스크톱 환경에 따라 달라집니다.

**Homebrew로 설치:** 최신 Homebrew와 데스크톱 환경이 필요합니다.

```sh
brew install --cask harry0703/tap/mangodisk
```

**터미널에서 설치(Debian/Ubuntu):** 아키텍처를 자동으로 감지해 해당 아키텍처의 최신 `.deb` 패키지를 설치합니다.

```sh
curl -fsSL https://get.mangodisk.app/linux | bash
```

**직접 설치:** [공식 다운로드 페이지](https://mangodisk.app/download)에서 아키텍처에 맞는 파일을 선택하세요.

- **Debian/Ubuntu:** 아키텍처에 맞는 `.deb` 패키지를 설치하세요.
- **다른 배포판:** AppImage를 사용해 볼 수 있습니다. 실행 권한을 부여한 뒤 실행하세요.

## 명령줄 버전(CLI)

터미널이나 스크립트에서 사용할 수 있습니다. 데스크톱 버전과 같은 정리 엔진을 사용합니다.

### macOS

**Homebrew로 설치:**

```sh
brew install harry0703/tap/mangodisk-cli
```

### Windows

**PowerShell로 설치:**

```powershell
irm https://get.mangodisk.app/cli | iex
```

**WinGet으로 설치(공식 소스):**

```powershell
winget install --id MangoDisk.CLI --exact --source winget
```

### Linux

Linux용 독립 실행형 CLI는 아직 미리 빌드된 실행 파일을 제공하지 않습니다. 아래의 [소스에서 빌드](#소스에서-빌드) 절차에 따라 직접 빌드할 수 있습니다.

### 사용 예시

설치 후 `mangodisk` 명령을 찾을 수 없다면 터미널을 다시 열고 버전을 확인하세요.

```sh
mangodisk --version
```

자주 사용하는 명령:

```sh
# 파일을 변경하지 않고 정리 가능한 항목만 스캔
mangodisk clean

# 데스크톱 버전과 같은 스마트 추천 적용
mangodisk clean --apply

# 실제로 삭제하지 않고 선택 가능한 모든 항목 미리 보기
mangodisk clean --apply --selection all --dry-run

# 스크립트에서 처리하기 쉬운 JSON 형식으로 출력
mangodisk clean --format json --no-progress
```

`mangodisk clean`은 기본적으로 읽기 전용으로 스캔합니다. 대화형 입력 없이 정리하려면 `--yes`를 지정해 실행을 확인하세요. 전체 옵션은 다음 명령으로 확인할 수 있습니다.

```sh
mangodisk clean --help
```

## 소스에서 빌드

### 사전 요구 사항

- Node.js 24 LTS
- pnpm 11.13.1
- Rust 안정 버전

플랫폼별 필수 라이브러리는 [Tauri 2 사전 요구 사항](https://v2.tauri.app/start/prerequisites/)을 참고하세요.

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

로컬 빌드는 개발과 검증에 사용하세요. 서명, 공증, 업데이트 메타데이터는 공식 배포 과정에서 제공됩니다.

## MangoDisk 후원하기

MangoDisk는 광고 없는 무료 오픈 소스 앱입니다. GPL-3.0에 따라 개인 및 상업 용도로 핵심 기능을 무료로 이용할 수 있습니다. 개발을 후원하고 공식 클라우드 AI를 이용하고 싶다면 [후원 안내](https://mangodisk.app/supporter)를 확인하세요.

## 기여하기

이슈, 정리 규칙, 수정, 새 기능 모두 환영합니다. 시작하기 전에 [`CONTRIBUTING.md`](CONTRIBUTING.md)와 [`AGENTS.md`](AGENTS.md)를 읽어 주세요.

일반적인 정리 항목은 빌드 시 검증되는 선언형 TOML 규칙으로 추가하세요. 규칙 스키마, 안전 제약, 검증 방법은 [`src-tauri/crates/mangodisk-core/rules/README.md`](src-tauri/crates/mangodisk-core/rules/README.md)를 참고하세요.

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
