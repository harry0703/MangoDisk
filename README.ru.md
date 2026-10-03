<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="Значок приложения MangoDisk"> MangoDisk
</h1>

<p align="center">Очистка диска, анализ хранилища и защита конфиденциальности для <b>macOS</b>, <b>Windows</b> и <b>Linux</b></p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a> · Русский
</p>

<p align="center">
  <a href="https://github.com/harry0703/MangoDisk/releases/latest"><img alt="Последний выпуск" src="https://img.shields.io/github/v/release/harry0703/MangoDisk?display_name=tag&sort=semver"></a>
  <img alt="Поддержка macOS" src="https://img.shields.io/badge/macOS-supported-111827?logo=apple&logoColor=white">
  <img alt="Поддержка Windows" src="https://img.shields.io/badge/Windows-supported-2563eb?logo=windows&logoColor=white">
  <img alt="Поддержка Linux" src="https://img.shields.io/badge/Linux-supported-f59e0b?logo=linux&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri&logoColor=white">
  <img alt="Ядро на Rust" src="https://img.shields.io/badge/core-Rust-b7410e?logo=rust&logoColor=white">
</p>

<p align="center">
  <a href="https://mangodisk.app/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/readme/en-dark.jpg">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/readme/en-light.jpg">
      <img src="https://assets.mangodisk.app/images/readme/en-light.jpg" width="1200" alt="MangoDisk: очистка диска, анализ хранилища, защита конфиденциальности и оптимизация системы">
    </picture>
  </a>
</p>

## Возможности MangoDisk

> **Хранилище**

### 1. Глубокая очистка

За одно сканирование находит данные, которые можно удалить, в системе, приложениях, инструментах разработчика и локальных проектах. MangoDisk избавляет от ручной проверки каждого расположения и группирует результаты по объёму освобождаемого места:

- **Системные и пользовательские кэши**: освобождает место, которое со временем заняли системные временные файлы, диагностические данные и кэши, допускающие повторное создание.
- **Кэши приложений**: не позволяет кэшам, журналам, пакетам обновлений и временным данным приложений незаметно занимать всё больше места.
- **Данные браузеров**: освобождает место, занятое кэшированными и временными веб-данными Chrome, Edge, Firefox, Brave, Arc, Opera и других браузеров.
- **Инструменты разработчика и Xcode**: быстро освобождает значительный объём хранилища, занятый менеджерами пакетов, IDE, кэшами компиляторов и данными разработки Xcode.
- **Кэши контейнеров**: удаляет неиспользуемые кэши сборки и восстанавливаемые данные Docker и других контейнерных инструментов.
- **Артефакты сборки проектов**: освобождает место, занятое восстанавливаемыми зависимостями, кэшами и каталогами сборки проектов Node.js, Rust, Gradle, Swift, Python, .NET, Godot, CMake и других экосистем.
- **Модели и кэши ИИ**: быстро обнаруживает крупные локальные модели ИИ, кэши загрузок и временные файлы передачи.
- **Оптимизация приложений**: уменьшает размер поддерживаемых приложений без нарушения их нормальной работы.

Умные рекомендации помогают быстро выбирать безопасные действия. Каждый элемент также можно проверить отдельно и заранее увидеть оценку освобождаемого места, сохраняя очистку предсказуемой и контролируемой.

### 2. Очистка крупных файлов

Быстро находит самые большие файлы и освобождает место, занятое старыми установщиками, видео, архивами и другими объёмными данными, без ручного обхода папок.

### 3. Очистка дубликатов

Освобождает место, занятое точными копиями, не считая файлы дубликатами только из-за одинакового имени. Умный выбор сохраняет не менее одного файла в каждой группе, поэтому очистка остаётся простой и безопасной.

### 4. Анализ дискового пространства

Наглядно показывает, чем занято хранилище. Древовидная карта и список позволяют быстро перейти к самым большим папкам и файлам вместо очистки вслепую.

> **Конфиденциальность и безопасность**

### 5. Очистка конфиденциальных данных

Удаляет историю посещений и поиска, файлы cookie, недавние элементы и данные буфера обмена. Очистка следов, оставленных браузерами, приложениями и системой, снижает раскрытие активности и упрощает повседневную защиту конфиденциальности.

> **Системные инструменты**

### 6. Удаление приложений и связанных данных

Удаляет приложения вместе со связанными кэшами, настройками и остаточными файлами, чтобы пространство действительно освобождалось. С потенциально личными файлами MangoDisk обращается осторожно, снижая риск случайной потери данных.

### 7. Управление автозагрузкой

Сокращает ненужные задержки при запуске и фоновое потребление ресурсов, ускоряя загрузку компьютера. При необходимости отключённые элементы можно включить снова в любое время.

### 8. Оптимизация системы

Отключает ненужные параметры, которые замедляют систему или мешают работе. Позволяет сбалансировать производительность, конфиденциальность и личные предпочтения.

### 9. Обслуживание системы

Исправляет распространённые проблемы — пропавшие результаты поиска, неправильные значки, отсутствие звука или сбои сетевого подключения — без самостоятельного поиска решений и ввода сложных команд.

> **Активность**

### 10. История операций

Хранит понятную запись каждой очистки и системного изменения. Показывает, сколько места освобождено, какие действия завершились успешно и что ещё требует внимания.

## Использование ресурсов и управление памятью

> Доступно начиная с версии 1.1.1

Показывает загрузку процессора и памяти, скорость сети и активность диска. Можно увидеть приложения с наибольшим потреблением памяти и освободить память одним нажатием, когда ресурсов становится мало.

Эти сведения доступны в строке меню, панели задач или системном трее — открывать главное окно не требуется.

## Объяснения с помощью ИИ

> Доступно начиная с версии 1.1.0

Не уверены, для чего нужен элемент и к чему приведёт его изменение? ИИ использует описание элемента и текущие результаты сканирования, чтобы объяснить назначение и важные последствия. Это помогает меньше искать информацию вручную и принимать более обоснованные решения.

Объяснения доступны непосредственно для элементов глубокой очистки (встроенных правил), очистки конфиденциальных данных, управления автозагрузкой, оптимизации системы и обслуживания системы.

Официальные выпуски ежедневно предоставляют бесплатные объяснения; также можно подключить собственный сервис ИИ. ИИ даёт рекомендации, а решение о действии остаётся за вами.

## Безопасность и правила

> [!IMPORTANT]
> **Для MangoDisk безопасность данных важнее максимального освобождения места.**
> Правила очистки и системные оптимизации включаются в выпуск только после определения чётких границ безопасности и проверки на реальных системах.

По умолчанию MangoDisk выполняет сканирование в режиме только для чтения. Перед очисткой, безвозвратным удалением, удалением приложения или изменением системных параметров можно проверить и подтвердить точный план действий. Результаты сохраняются в истории операций.

Оптимизация системы использует только встроенные и проверенные параметры. Она никогда не принимает произвольные пути реестра, команды терминала или сценарии. После изменения MangoDisk повторно считывает каждый параметр, отмечает действия с высоким влиянием и указывает, когда требуются права администратора или перезагрузка.

MangoDisk поддерживает собственные правила очистки. Сторонние проекты могут быть источником идей для исследования, но новое правило принимается только после проверки надёжных источников, безопасных границ и поведения на реальной системе. Всё, для чего нельзя определить чёткую границу безопасности, исключается.

Полная библиотека правил и история её изменений открыты для проверки: [библиотека правил очистки MangoDisk](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules).

## Снимки экрана

<p align="center">
  <strong>Глубокая очистка</strong><br>
  <sub>Поиск очищаемых данных в системе, приложениях, инструментах разработчика и проектах</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-01-deep-cleanup.jpg">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg">
    <img src="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg" width="1200" alt="Интерфейс глубокой очистки MangoDisk">
  </picture>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>Очистка крупных файлов</strong><br>
      <sub>Поиск файлов, занимающих больше всего места, без ручного обхода папок</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-02-large-file-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg" width="100%" alt="Интерфейс очистки крупных файлов MangoDisk">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>Очистка дубликатов</strong><br>
      <sub>Безопасное удаление точных дубликатов с сохранением не менее одной копии</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-03-duplicate-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg" width="100%" alt="Интерфейс очистки дубликатов MangoDisk">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Анализ дискового пространства</strong><br>
      <sub>Поиск самых больших файлов и папок с наглядным представлением занятого места</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-05-disk-space-analysis.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg" width="100%" alt="Интерфейс анализа дискового пространства MangoDisk">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>Управление автозагрузкой</strong><br>
      <sub>Отключение ненужных программ для ускорения входа и снижения фоновой активности</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-06-startup-items.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg" width="100%" alt="Интерфейс управления автозагрузкой MangoDisk">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Удаление приложений и связанных данных</strong><br>
      <sub>Удаление приложений и их остаточных файлов для освобождения места</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-04-app-uninstaller.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg" width="100%" alt="Интерфейс удаления приложений MangoDisk">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>Оптимизация системы</strong><br>
      <sub>Настройка производительности, конфиденциальности и удобства одним нажатием</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-07-system-optimization.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg" width="100%" alt="Интерфейс оптимизации системы MangoDisk">
      </picture>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Обслуживание системы</strong><br>
      <sub>Быстрое исправление распространённых системных проблем</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-08-system-maintenance.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg" width="100%" alt="Интерфейс обслуживания системы MangoDisk">
      </picture>
    </td>
    <td width="50%" align="center">
      <strong>Очистка конфиденциальных данных</strong><br>
      <sub>Сокращение следов активности и защита повседневной конфиденциальности</sub><br><br>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-09-privacy-cleanup.jpg">
        <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg">
        <img src="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg" width="100%" alt="Интерфейс очистки конфиденциальных данных MangoDisk">
      </picture>
    </td>
  </tr>
</table>

## Перед началом работы

> [!CAUTION]
>
> 1. Очистка, безвозвратное удаление файлов и удаление приложений могут быть необратимыми. Проверяйте выбранные данные и храните надёжные резервные копии важных файлов.
> 2. Перед обслуживанием системы, изменением элемента автозагрузки или системного параметра убедитесь, что понимаете назначение и последствия действия.
> 3. Некоторые системные оптимизации могут влиять на безопасность, конфиденциальность, время автономной работы или поведение обновлений.

## Настольное приложение

Загрузите MangoDisk с [официальной страницы](https://mangodisk.app/download) или из раздела [GitHub Releases](https://github.com/harry0703/MangoDisk/releases/latest), затем следуйте инструкции для своей операционной системы.

### macOS

**Требования:** macOS Monterey 12.5 или новее.

**Установка через Homebrew:**

```sh
brew install --cask harry0703/tap/mangodisk
```

**Установка вручную:** загрузите DMG с [официальной страницы](https://mangodisk.app/download), откройте его и перетащите MangoDisk в папку Applications.

### Windows

**Требования:** 64-разрядная Windows 10 или новее.

**Установка через PowerShell:**

```powershell
irm https://get.mangodisk.app | iex
```

**Установка через WinGet (официальный источник):**

```powershell
winget install --id MangoDisk.MangoDisk --exact --source winget
```

**Установка вручную:** загрузите установщик Windows с [официальной страницы](https://mangodisk.app/download) и следуйте указаниям программы установки.

### Linux

**Рекомендуется:** Ubuntu 22.04 LTS или новее на x64 или ARM64.

Доступны пакеты `.deb` и образы AppImage. Совместимость с другими дистрибутивами Linux зависит от их системных библиотек и окружения рабочего стола.

**Установка из терминала (Debian/Ubuntu):** команда определит архитектуру компьютера и установит подходящий `.deb` последней версии.

```sh
curl -fsSL https://get.mangodisk.app/linux | bash
```

**Установка вручную:** выберите пакет для своей архитектуры на [официальной странице](https://mangodisk.app/download).

- **Debian/Ubuntu:** установите пакет `.deb` для своей архитектуры.
- **Другие дистрибутивы:** сделайте AppImage исполняемым и попробуйте запустить его.

## Командная строка (CLI)

Используйте MangoDisk в терминале или сценариях с тем же ориентированным на безопасность механизмом очистки, что и в настольном приложении.

### macOS

**Установка через Homebrew:**

```sh
brew install harry0703/tap/mangodisk-cli
```

### Windows

**Установка через PowerShell:**

```powershell
irm https://get.mangodisk.app/cli | iex
```

**Установка через WinGet (официальный источник):**

```powershell
winget install --id MangoDisk.CLI --exact --source winget
```

### Linux

Готовые автономные сборки CLI для Linux пока недоступны. Соберите приложение самостоятельно по инструкции [«Сборка из исходного кода»](#сборка-из-исходного-кода).

### Примеры использования

Если команда `mangodisk` недоступна сразу после установки, откройте новый терминал и проверьте установку:

```sh
mangodisk --version
```

Основные команды:

```sh
# Сканировать и показать очищаемые данные без изменений
mangodisk clean

# Применить те же умные рекомендации, что и в настольном приложении
mangodisk clean --apply

# Предварительно показать все доступные данные без удаления
mangodisk clean --apply --selection all --dry-run

# Вывести результат в машиночитаемом формате JSON
mangodisk clean --format json --no-progress
```

По умолчанию `mangodisk clean` только сканирует и никогда не изменяет файлы. Для очистки в неинтерактивной среде необходимо также передать `--yes`, явно подтверждая действие. Чтобы увидеть все параметры, выполните:

```sh
mangodisk clean --help
```

## Сборка из исходного кода

### Требования

- Node.js 24 LTS
- pnpm 11.13.1
- Стабильная версия Rust

Зависимости для конкретных платформ перечислены в [требованиях Tauri 2](https://v2.tauri.app/start/prerequisites/).

### Получение исходного кода и запуск настольного приложения

```sh
git clone https://github.com/harry0703/MangoDisk.git
cd MangoDisk
pnpm install --frozen-lockfile
pnpm tauri:dev
```

### Запуск обязательных проверок

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

### Сборка установщика настольного приложения

```sh
pnpm tauri:build
```

### Сборка CLI

```sh
pnpm cli:build
```

Локальные сборки не содержат подписей, нотариального заверения и метаданных обновления из официальных выпусков MangoDisk. Используйте их только для локальной разработки и проверки.

## Участие в разработке

Сообщения о проблемах, правила очистки, исправления и новые функции приветствуются. Перед началом работы прочитайте [`CONTRIBUTING.md`](CONTRIBUTING.md) и [`AGENTS.md`](AGENTS.md).

Обычные сценарии очистки следует реализовывать декларативными правилами TOML, проверяемыми при сборке. Схема правил, ограничения безопасности и инструкции по проверке описаны в [`src-tauri/crates/mangodisk-core/rules/README.md`](src-tauri/crates/mangodisk-core/rules/README.md).

Перед отправкой изменений выполните как минимум:

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

Сообщайте об уязвимостях конфиденциально через GitHub Security Advisories, как описано в [`SECURITY.md`](SECURITY.md). Не создавайте публичную issue для уязвимости безопасности.

## Технологический стек

- [Tauri 2](https://tauri.app/): среда выполнения настольного приложения и интеграция с системой
- [Rust](https://www.rust-lang.org/): сканирование, доступ к файловой системе, проверка безопасности и выполнение очистки
- [Vue 3](https://vuejs.org/) и [TypeScript](https://www.typescriptlang.org/): пользовательский интерфейс настольного приложения

## Лицензия

MangoDisk распространяется с открытым исходным кодом по лицензии [GNU General Public License v3.0](https://github.com/harry0703/MangoDisk/blob/main/LICENSE). Сторонние компоненты подчиняются условиям собственных лицензий.
