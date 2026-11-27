<h1 align="center">
  <img src="public/mangodisk.svg" width="40" alt="Значок приложения MangoDisk"> MangoDisk
</h1>

<p align="center">Очистка диска и анализ дискового пространства для <b>macOS</b>, <b>Windows</b> и <b>Linux</b></p>

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
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-00-overview.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-00-overview.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-00-overview.jpg?ver=2.1.1" width="1200" alt="Обзорная панель MangoDisk">
    </picture>
  </a>
</p>

## Возможности MangoDisk

> **Хранилище**

### 1. Глубокая очистка

Приложения, инструменты разработки и программы для работы с ИИ оставляют кэши, файлы сборки и локальные модели. MangoDisk собирает их в одном списке, избавляя от поисков по скрытым папкам.

- **Системные и пользовательские кэши**. Очистка временных файлов, диагностических данных и кэшей, которые можно создать заново.
- **Кэши приложений**. Очистка кэшей, журналов, пакетов обновлений и временных файлов.
- **Данные браузеров**. Очистка кэшей и временных данных Chrome, Edge, Firefox, Brave, Arc, Opera и других браузеров.
- **Инструменты разработки и Xcode**. Очистка кэшей менеджеров пакетов, IDE и компиляторов, а также данных разработки Xcode.
- **Кэши контейнеров**. Очистка неиспользуемых кэшей сборки и других данных, которые Docker и похожие инструменты могут создать заново.
- **Файлы сборки проектов**. Очистка зависимостей, кэшей и папок сборки, которые можно заново установить или сгенерировать в проектах Node.js, Rust, Gradle, Swift, Python, .NET, Godot и CMake.
- **Инструменты ИИ**. Очистка кэшей, старых журналов и временных файлов загрузки ChatGPT, Codex, Claude и других приложений.
- **Модели ИИ**. Можно проверить, сколько места занимают локальные модели Ollama, LM Studio, Hugging Face и ComfyUI, и вручную выбрать ненужные. Если удалённая модель снова понадобится, её нужно будет скачать заново.
- **Оптимизация приложений**. Уменьшение размера поддерживаемых приложений с сохранением их работоспособности.

Можно выбрать рекомендованные позиции или проверить каждую вручную. Примерный объём освобождаемого места виден до начала очистки.

### 2. Очистка крупных файлов

Найдите самые объёмные файлы — например, старые установщики, видео и архивы — без ручного обхода папок.

### 3. Очистка дубликатов

MangoDisk сравнивает файлы по содержимому и находит точные копии независимо от имени файла. При автоматическом выборе в каждой группе остаётся хотя бы одна копия.

### 4. Анализ дискового пространства

#### Распределение места

Посмотрите, чем занят диск, на **древовидной карте** или **кольцевой диаграмме**. Настройте глубину отображения и просматривайте папки через список файлов, чтобы найти самые объёмные.

#### Изменения объёма

Свободного места вдруг стало меньше? Включите историю сканирования и сравните результаты двух проверок одного диска или папки. Так видно, какие файлы и папки стали занимать больше или меньше места и насколько. После очистки повторите сканирование, чтобы увидеть, где освободилось место.

> **Конфиденциальность и безопасность**

### 5. Очистка конфиденциальных данных

Удаляйте историю браузера и поиска, файлы cookie, списки недавних элементов и содержимое буфера обмена, чтобы убрать следы работы с браузерами, приложениями и системой.

> **Системные инструменты**

### 6. Удаление приложений и связанных данных

Удаляйте приложения вместе с кэшами, настройками и оставшимися файлами. MangoDisk с особой осторожностью обрабатывает папки, в которых могут храниться личные файлы.

### 7. Управление автозагрузкой

Отключайте ненужные программы в автозагрузке, чтобы ускорить запуск системы и снизить фоновую нагрузку. При необходимости их можно снова включить.

### 8. Оптимизация системы

Настройте систему с учётом производительности, конфиденциальности и ваших привычек.

### 9. Обслуживание системы

Устраняйте распространённые проблемы с поиском, значками, звуком или подключением к сети без ручной диагностики и команд в терминале.

### 10. Контекстное меню (Windows)

Просматривайте пункты по расположению в меню или по приложению. Поддерживаемые пункты можно отключать, восстанавливать и удалять, в том числе отдельно для разных типов файлов.

Добавляйте часто используемые действия, например копирование пути и открытие терминала, а также пункты для запуска системных инструментов.

> **Активность**

### 11. История операций

Проверяйте результаты очистки и изменений в системе. В истории видно, сколько места освободилось, какие действия выполнены и что ещё требует внимания.

## Использование ресурсов и управление памятью

Следите в реальном времени за нагрузкой на процессор и видеокарту, использованием памяти, работой сети и дисков. На совместимых устройствах также доступны сведения о температуре, энергопотреблении, аккумуляторе и состоянии дисков.

Находите и закрывайте приложения с высоким потреблением ресурсов, освобождайте память вручную или автоматически.

Показатели можно вывести в строку меню, на панель задач, в системный лоток или плавающее окно. Состав и вид показателей настраиваются.

## Объяснения с помощью ИИ

Если непонятно, для чего нужен элемент и что изменится после операции, откройте объяснение рядом с ним. ИИ использует описание элемента и результаты сканирования и подсказывает, что учесть перед действием. Объяснения доступны для встроенных правил глубокой очистки, очистки конфиденциальных данных, управления автозагрузкой, оптимизации и обслуживания системы.

## Безопасность и правила

> [!IMPORTANT]
> **Безопасность данных — приоритет MangoDisk.**
> В выпуски входят правила очистки и параметры оптимизации, для которых определены условия безопасного применения и проведена проверка на реальных системах.

При сканировании MangoDisk по умолчанию только читает данные. Перед очисткой, удалением файлов или приложений либо изменением настроек можно проверить и подтвердить список действий. Результаты сохраняются в истории операций.

Для оптимизации используются только встроенные и проверенные параметры. После изменения MangoDisk повторно считывает значение. Перед выполнением программа предупреждает о рисках, необходимости прав администратора или перезагрузки.

MangoDisk ведёт собственный набор правил очистки. Сторонние проекты могут подсказать, что стоит изучить. Для каждого нового правила проверяются источники и условия безопасного применения, а затем проводятся испытания на реальных системах.

Правила и история их изменений доступны всем. [Посмотреть правила очистки MangoDisk](https://github.com/harry0703/MangoDisk/tree/main/src-tauri/crates/mangodisk-core/rules).

## Снимки экрана

<p align="center">
  <strong>Глубокая очистка</strong><br>
  <sub>Очистка накопившихся кэшей, файлов сборки и ненужных моделей ИИ</sub>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-01-deep-cleanup.jpg?ver=2.1.1">
    <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg?ver=2.1.1">
    <img src="https://assets.mangodisk.app/images/screenshots/en/light-01-deep-cleanup.jpg?ver=2.1.1" width="1200" alt="Интерфейс глубокой очистки MangoDisk">
  </picture>
</p>

<p align="center">
  <strong>Распределение места</strong><br>
  <sub>Наглядная разбивка занятого места и поиск самых больших файлов и папок</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-05-disk-space-analysis.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis.jpg?ver=2.1.1" width="1200" alt="Интерфейс анализа дискового пространства MangoDisk">
    </picture>
  </a>
</p>

<p align="center">
  <strong>Изменения объёма</strong><br>
  <sub>Сравнение двух сканирований показывает, какие файлы и папки изменились в размере</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-05-disk-space-analysis-space-changes.jpg?ver=2.1.1" width="1200" alt="Сравнение результатов сканирований на экране изменений объёма MangoDisk">
    </picture>
  </a>
</p>

<p align="center">
  <strong>Мониторинг системы</strong><br>
  <sub>Нагрузка на процессор, использование памяти, активность сети и дисков</sub>
</p>

<p align="center">
  <a href="https://assets.mangodisk.app/images/screenshots/en/light-system-monitoring-memory.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-system-monitoring-memory.jpg?ver=2.1.1">
      <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-system-monitoring-memory.jpg?ver=2.1.1">
      <img src="https://assets.mangodisk.app/images/screenshots/en/light-system-monitoring-memory.jpg?ver=2.1.1" width="720" alt="Мониторинг системы и управление памятью в MangoDisk">
    </picture>
  </a>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>Очистка крупных файлов</strong><br>
      <sub>Быстрый поиск файлов, занимающих больше всего места</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-02-large-file-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-02-large-file-cleanup.jpg?ver=2.1.1" width="100%" alt="Интерфейс очистки крупных файлов MangoDisk">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>Очистка дубликатов</strong><br>
      <sub>Удаление точных дубликатов с сохранением хотя бы одной копии в каждой группе</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-03-duplicate-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-03-duplicate-cleanup.jpg?ver=2.1.1" width="100%" alt="Интерфейс очистки дубликатов MangoDisk">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Управление автозагрузкой</strong><br>
      <sub>Отключение ненужных программ в автозагрузке</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-06-startup-items.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-06-startup-items.jpg?ver=2.1.1" width="100%" alt="Интерфейс управления автозагрузкой MangoDisk">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>Удаление приложений и связанных данных</strong><br>
      <sub>Удаление приложений вместе с оставшимися файлами</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-04-app-uninstaller.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-04-app-uninstaller.jpg?ver=2.1.1" width="100%" alt="Интерфейс удаления приложений MangoDisk">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Оптимизация системы</strong><br>
      <sub>Настройки производительности, конфиденциальности и удобства работы</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-07-system-optimization.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-07-system-optimization.jpg?ver=2.1.1" width="100%" alt="Интерфейс оптимизации системы MangoDisk">
        </picture>
      </a>
    </td>
    <td width="50%" align="center">
      <strong>Обслуживание системы</strong><br>
      <sub>Исправление распространённых системных сбоев</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-08-system-maintenance.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-08-system-maintenance.jpg?ver=2.1.1" width="100%" alt="Интерфейс обслуживания системы MangoDisk">
        </picture>
      </a>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Очистка конфиденциальных данных</strong><br>
      <sub>Удаление следов работы с браузерами, приложениями и системой</sub><br><br>
      <a href="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg?ver=2.1.1" target="_blank" rel="noopener noreferrer">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="https://assets.mangodisk.app/images/screenshots/en/dark-09-privacy-cleanup.jpg?ver=2.1.1">
          <source media="(prefers-color-scheme: light)" srcset="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg?ver=2.1.1">
          <img src="https://assets.mangodisk.app/images/screenshots/en/light-09-privacy-cleanup.jpg?ver=2.1.1" width="100%" alt="Интерфейс очистки конфиденциальных данных MangoDisk">
        </picture>
      </a>
    </td>
  </tr>
</table>

## Перед началом работы

> [!CAUTION]
>
> 1. Данные после очистки, безвозвратного удаления файлов или удаления приложений могут не подлежать восстановлению. Проверьте выбранные элементы и сделайте резервную копию важных данных.
> 2. Перед обслуживанием системы или изменением автозагрузки и системных настроек проверьте, для чего нужен элемент и на что повлияет действие.
> 3. Некоторые настройки оптимизации могут повлиять на безопасность, конфиденциальность, время работы от батареи или обновления системы.

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

**Установка через Homebrew:** нужны актуальная версия Homebrew и графическое окружение рабочего стола.

```sh
brew install --cask harry0703/tap/mangodisk
```

**Установка из терминала (Debian/Ubuntu):** команда определит архитектуру компьютера и установит подходящий `.deb` последней версии.

```sh
curl -fsSL https://get.mangodisk.app/linux | bash
```

**Установка вручную:** выберите пакет для своей архитектуры на [официальной странице](https://mangodisk.app/download).

- **Debian/Ubuntu:** установите пакет `.deb` для своей архитектуры.
- **Другие дистрибутивы:** сделайте AppImage исполняемым и попробуйте запустить его.

## Командная строка (CLI)

CLI подходит для работы в терминале и автоматизации. В нём используется тот же механизм очистки, что и в настольном приложении.

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

По умолчанию `mangodisk clean` выполняет сканирование в режиме чтения. Для очистки без интерактивного запроса добавьте `--yes`: этот флаг подтверждает действие. Справку по всем параметрам можно открыть командой:

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

Локальные сборки предназначены для разработки и проверки. Подпись, нотаризация и метаданные для обновлений добавляются при официальном выпуске.

## Поддержать MangoDisk

MangoDisk — бесплатное приложение с открытым исходным кодом и без рекламы. Основные функции доступны бесплатно для личного и коммерческого использования по GPL-3.0. Чтобы поддержать разработку и пользоваться облачным ИИ MangoDisk, [узнайте, как поддержать проект](https://mangodisk.app/supporter).

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
