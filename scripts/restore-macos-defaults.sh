#!/usr/bin/env bash
#
# restore-macos-defaults.sh
#
# Undo every macOS preference change that MangoDisk's System Optimization may
# have written, restoring all affected keys to their factory defaults.
#
# How it works:
#   - System Optimization writes preference keys via `defaults write` (see
#     src-tauri/crates/mangodisk-platform/src/macos/system_settings.rs for the
#     domain/key mapping and
#     src-tauri/crates/mangodisk-core/src/system_settings/catalog.rs for the
#     values). The list below mirrors those definitions.
#   - On macOS, an absent preference key means "use the system default".
#     Restoring a key therefore means deleting it, not writing a guessed
#     default value back.
#   - Every key in the list is deleted; keys that were never written are
#     skipped silently.
#
# Safety:
#   1. Before touching anything, all affected preference domains are exported
#      to ~/Desktop/mango-restore-backup-<timestamp>/. A domain can be rolled
#      back afterwards with: defaults import <domain> <backup>.plist
#   2. Only the keys listed below are deleted; nothing else is modified.
#   3. com.apple.sound.beep.feedback (interface sound effects) is written back
#      to `true` instead of deleted: on some systems a missing key is treated
#      as "sound effects off", while the factory default is on.
#
# Notes:
#   - Safari, TextEdit and similar apps pick up the restored values the next
#     time they launch.
#   - The script restarts Finder and Dock at the end; the screen may flicker
#     briefly, which is expected.
#   - Keys customized manually outside MangoDisk are also reset; they are
#     covered by the backup.
#   - When a macOS setting is added to or removed from System Optimization,
#     update the KEYS list below in the same change.

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script must run on macOS." >&2
  exit 1
fi

BACKUP_DIR="$HOME/Desktop/mango-restore-backup-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$BACKUP_DIR"

# All preference domains System Optimization writes to (backed up as a whole).
DOMAINS=(
  NSGlobalDomain
  com.apple.finder
  com.apple.desktopservices
  com.apple.dock
  com.apple.screencapture
  com.apple.print.PrintingPrefs
  com.apple.Safari
  com.apple.TextEdit
  com.apple.ImageCapture
  com.apple.AdLib
  com.apple.NetworkBrowser
  com.apple.ActivityMonitor
  com.apple.commerce
  com.apple.TimeMachine
  com.apple.screensaver
)

echo "==> Backing up affected preference domains to $BACKUP_DIR"
for d in "${DOMAINS[@]}"; do
  # NSGlobalDomain cannot be exported; fall back to a readable text dump.
  # Domains that were never written cannot be exported either; skip them.
  defaults export "$d" "$BACKUP_DIR/$d.plist" 2>/dev/null \
    || defaults read "$d" >"$BACKUP_DIR/$d.txt" 2>/dev/null \
    || true
done

# Every <domain>|<key> pair System Optimization may have written.
KEYS=(
  # NSGlobalDomain
  "NSGlobalDomain|AppleShowAllExtensions"
  "NSGlobalDomain|AppleShowAllFiles"
  "NSGlobalDomain|NSNavPanelExpandedStateForSaveMode"
  "NSGlobalDomain|PMPrintingExpandedStateForPrint"
  "NSGlobalDomain|NSAutomaticWindowAnimationsEnabled"
  "NSGlobalDomain|NSQuitAlwaysKeepsWindows"
  "NSGlobalDomain|AppleActionOnDoubleClick"
  "NSGlobalDomain|AppleKeyboardUIMode"
  "NSGlobalDomain|KeyRepeat"
  "NSGlobalDomain|InitialKeyRepeat"
  "NSGlobalDomain|ApplePressAndHoldEnabled"
  "NSGlobalDomain|com.apple.keyboard.fnState"
  "NSGlobalDomain|NSDocumentSaveNewDocumentsToCloud"
  "NSGlobalDomain|NSAutomaticSpellingCorrectionEnabled"
  "NSGlobalDomain|NSAutomaticQuoteSubstitutionEnabled"
  "NSGlobalDomain|NSAutomaticDashSubstitutionEnabled"
  "NSGlobalDomain|NSAutomaticCapitalizationEnabled"
  "NSGlobalDomain|NSAutomaticPeriodSubstitutionEnabled"
  "NSGlobalDomain|NSAutomaticTextCompletionEnabled"
  "NSGlobalDomain|NSAutomaticInlinePredictionEnabled"
  # Finder
  "com.apple.finder|ShowPathbar"
  "com.apple.finder|ShowStatusBar"
  "com.apple.finder|_FXShowPosixPathInTitle"
  "com.apple.finder|DisableAllAnimations"
  "com.apple.finder|_FXSortFoldersFirst"
  "com.apple.finder|FXDefaultSearchScope"
  "com.apple.finder|FXEnableExtensionChangeWarning"
  "com.apple.finder|ShowHardDrivesOnDesktop"
  "com.apple.finder|ShowExternalHardDrivesOnDesktop"
  "com.apple.finder|ShowRemovableMediaOnDesktop"
  "com.apple.finder|FXPreferredViewStyle"
  "com.apple.finder|_FXSortFoldersFirstOnDesktop"
  "com.apple.finder|QuitMenuItem"
  "com.apple.finder|FXRemoveOldTrashItems"
  "com.apple.finder|WarnOnEmptyTrash"
  # DesktopServices (.DS_Store writes)
  "com.apple.desktopservices|DSDontWriteNetworkStores"
  "com.apple.desktopservices|DSDontWriteUSBStores"
  # Dock / Mission Control
  "com.apple.dock|autohide"
  "com.apple.dock|minimize-to-application"
  "com.apple.dock|mineffect"
  "com.apple.dock|mru-spaces"
  "com.apple.dock|show-recents"
  "com.apple.dock|launchanim"
  "com.apple.dock|static-only"
  "com.apple.dock|showhidden"
  "com.apple.dock|autohide-delay"
  "com.apple.dock|magnification"
  "com.apple.dock|expose-group-apps"
  "com.apple.dock|enable-spring-load-actions-on-all-items"
  "com.apple.dock|scroll-to-open"
  "com.apple.dock|autohide-time-modifier"
  # Screenshots
  "com.apple.screencapture|disable-shadow"
  "com.apple.screencapture|type"
  "com.apple.screencapture|show-thumbnail"
  # Printing
  "com.apple.print.PrintingPrefs|Quit When Finished"
  # Safari
  "com.apple.Safari|ShowFullURLInSmartSearchField"
  "com.apple.Safari|AutoOpenSafeDownloads"
  "com.apple.Safari|ShowOverlayStatusBar"
  "com.apple.Safari|IncludeDevelopMenu"
  "com.apple.Safari|SuppressSearchSuggestions"
  "com.apple.Safari|PreloadTopHit"
  # TextEdit
  "com.apple.TextEdit|RichText"
  # Photos / privacy / sharing / other
  "com.apple.ImageCapture|disableHotPlug"
  "com.apple.AdLib|allowApplePersonalizedAdvertising"
  "com.apple.NetworkBrowser|DisableAirDrop"
  "com.apple.ActivityMonitor|ShowCategory"
  "com.apple.commerce|AutoUpdate"
  "com.apple.TimeMachine|DoNotOfferNewDisksForBackup"
  "com.apple.screensaver|askForPassword"
  "com.apple.screensaver|askForPasswordDelay"
)

echo "==> Restoring ${#KEYS[@]} preference keys plus interface sound effects"
deleted=0
absent=0
for entry in "${KEYS[@]}"; do
  d="${entry%%|*}"
  k="${entry#*|}"
  if defaults read "$d" "$k" >/dev/null 2>&1; then
    defaults delete "$d" "$k" >/dev/null 2>&1 || true
    echo "  restored: $d | $k"
    deleted=$((deleted + 1))
  else
    absent=$((absent + 1))
  fi
done

# Interface sound effects: write the explicit factory default ("on") instead
# of deleting, because a missing key is treated as off on some systems.
defaults write -g com.apple.sound.beep.feedback -bool true >/dev/null
echo "  restored: NSGlobalDomain | com.apple.sound.beep.feedback (written to true)"

echo "==> Restarting Finder and Dock to apply the changes"
killall Finder Dock SystemUIServer 2>/dev/null || true

echo ""
echo "Done: $deleted key(s) restored, $absent already at the default."
echo "Backup location: $BACKUP_DIR"
echo "Roll back one domain with: defaults import <domain> \"$BACKUP_DIR/<domain>.plist\""
