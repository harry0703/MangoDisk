import { describe, expect, it } from 'vitest';

import { CLEANUP_RULE_IDS } from '@/lib/models/cleanup';
import { ICON_NAMES } from '@/lib/models/ui';
import zhCN from '@/locales/zh-CN.json';

import { cleanupGroupIcon, cleanupRuleIcon, hasCleanupRuleIcon } from './cleanup-rule-icon';

describe('cleanup rule icons', () => {
  it('uses exact public brand icons for known products and ecosystems', () => {
    expect(cleanupRuleIcon('browser.chrome-cache', 'userCache')).toBe(ICON_NAMES.brandChrome);
    expect(cleanupRuleIcon('browser.chromium-cache', 'browser')).toBe(ICON_NAMES.brandChrome);
    expect(cleanupRuleIcon('browser.360-speed-cache', 'browser')).toBe(ICON_NAMES.brand360Browser);
    expect(cleanupRuleIcon('browser.360-safe-cache', 'browser')).toBe(ICON_NAMES.brand360Browser);
    expect(cleanupRuleIcon('browser.2345-cache', 'browser')).toBe(ICON_NAMES.brand360Browser);
    expect(cleanupRuleIcon('dev.node-tooling-cache', 'development')).toBe(ICON_NAMES.brandNodejs);
    expect(cleanupRuleIcon('dev.uv-cache', 'development')).toBe(ICON_NAMES.brandUv);
    expect(cleanupRuleIcon('dev.dart-analysis-cache', 'development')).toBe(ICON_NAMES.brandDart);
    expect(cleanupRuleIcon('dev.cargo-extracted-sources', 'development')).toBe(ICON_NAMES.brandRust);
    expect(cleanupRuleIcon('dev.qclaw-compile-cache', 'development')).toBe(ICON_NAMES.brandNodejs);
    expect(cleanupRuleIcon('project.rust-build-artifacts', 'project')).toBe(ICON_NAMES.brandRust);
    expect(cleanupRuleIcon('app.wechat-cache', 'userCache')).toBe(ICON_NAMES.brandWechat);
    expect(cleanupRuleIcon('app.wechat-diagnostic-cache', 'application')).toBe(ICON_NAMES.brandWechat);
    expect(cleanupRuleIcon('dev.jetbrains-cache', 'userCache')).toBe(ICON_NAMES.brandJetbrains);
    expect(cleanupRuleIcon('dev.gradle-cache', 'development')).toBe(ICON_NAMES.brandGradle);
    expect(cleanupRuleIcon('project.gradle-build-artifacts', 'project')).toBe(ICON_NAMES.brandGradle);
    expect(cleanupRuleIcon('project.godot-build-artifacts', 'project')).toBe(ICON_NAMES.brandGodot);
    expect(cleanupRuleIcon('project.cmake-build-artifacts', 'project')).toBe(ICON_NAMES.brandCmake);
    expect(cleanupRuleIcon('dev.homebrew-cache', 'userCache')).toBe(ICON_NAMES.brandHomebrew);
    expect(cleanupRuleIcon('app.lark-renderer-cache', 'userCache')).toBe(ICON_NAMES.brandLark);
    expect(cleanupRuleIcon('app.qqlive-rendering-cache', 'application')).toBe(ICON_NAMES.brandTencentVideo);
    expect(cleanupRuleIcon('app.qqlive-update-cache', 'application')).toBe(ICON_NAMES.brandTencentVideo);
    expect(cleanupRuleIcon('special.qqlive-offline-videos', 'application')).toBe(ICON_NAMES.brandTencentVideo);
    expect(cleanupRuleIcon('special.qqlive-playback-cache', 'application')).toBe(ICON_NAMES.brandTencentVideo);
    expect(cleanupRuleIcon('app.youku-rendering-cache', 'application')).toBe(ICON_NAMES.brandYouku);
    expect(cleanupRuleIcon('special.youku-offline-videos', 'application')).toBe(ICON_NAMES.brandYouku);
    expect(cleanupRuleIcon('special.youku-playback-cache', 'application')).toBe(ICON_NAMES.brandYouku);
    expect(cleanupRuleIcon('app.iqiyi-rendering-cache', 'application')).toBe(ICON_NAMES.brandIqiyi);
    expect(cleanupRuleIcon('special.iqiyi-offline-videos', 'application')).toBe(ICON_NAMES.brandIqiyi);
    expect(cleanupRuleIcon('special.iqiyi-playback-cache', 'application')).toBe(ICON_NAMES.brandIqiyi);
    expect(cleanupRuleIcon('special.xcode-simulator-runtime', 'xcode')).toBe(ICON_NAMES.brandXcode);
    expect(cleanupRuleIcon('special.ai-model-hugging-face', 'ai')).toBe(ICON_NAMES.brandHuggingface);
    expect(cleanupRuleIcon('special.ai-model-ollama', 'ai')).toBe(ICON_NAMES.brandOllama);
    expect(cleanupRuleIcon('special.ai-model-modelscope', 'ai')).toBe(ICON_NAMES.brandModelScope);
    expect(cleanupRuleIcon('special.codex-archived-sessions', 'development')).toBe(ICON_NAMES.brandOpenai);
    expect(cleanupRuleIcon('app.codex-diagnostic-cache', 'application')).toBe(ICON_NAMES.brandOpenai);
    expect(cleanupRuleIcon('special.rust-toolchains', 'development')).toBe(ICON_NAMES.brandRust);
    expect(cleanupRuleIcon(CLEANUP_RULE_IDS.windowsRecycleBin, 'system')).toBe(ICON_NAMES.trash);
  });

  it('uses the supplied brand glyphs for matching application cleanup rules', () => {
    const brandedRules = [
      ['app.qq-cache', ICON_NAMES.brandQq],
      ['app.qq-update-cache', ICON_NAMES.brandQq],
      ['app.qq-rendering-cache', ICON_NAMES.brandQq],
      ['app.qwenwork-cache', ICON_NAMES.brandQwenWork],
      ['app.utools-rendering-cache', ICON_NAMES.brandUtools],
      ['special.dropbox-cache', ICON_NAMES.brandDropbox],
      ['app.dropbox-rendering-cache', ICON_NAMES.brandDropbox],
      ['app.wps-cache', ICON_NAMES.brandWps],
      ['app.wps-rendering-cache', ICON_NAMES.brandWps],
      ['app.wps-diagnostic-cache', ICON_NAMES.brandWps],
      ['app.notion-cache', ICON_NAMES.brandNotion],
      ['app.notion-service-worker-cache', ICON_NAMES.brandNotion],
      ['app.wechat-cache', ICON_NAMES.brandWechat],
      ['app.wechat-rendering-cache', ICON_NAMES.brandWechat],
      ['app.wechat-diagnostic-cache', ICON_NAMES.brandWechat],
      ['app.wecom-cache', ICON_NAMES.brandWechat],
      ['app.wecom-diagnostic-cache', ICON_NAMES.brandWechat],
      ['app.dingtalk-content-cache', ICON_NAMES.brandDingtalk],
      ['app.dingtalk-diagnostic-cache', ICON_NAMES.brandDingtalk],
      ['app.claude-cache', ICON_NAMES.brandClaude],
      ['app.electron-cache', ICON_NAMES.brandElectron],
      ['app.electron-productivity-cache', ICON_NAMES.brandElectron],
      ['app.google-updater-cache', ICON_NAMES.brandGoogle],
      ['app.tencent-meeting-cache', ICON_NAMES.brandTencentMeeting],
      ['browser.brave-cache', ICON_NAMES.brandBrave],
      ['browser.brave-origin-cache', ICON_NAMES.brandBrave],
      ['browser.uc-cache', ICON_NAMES.brandUcBrowser],
      ['browser.sogou-cache', ICON_NAMES.brandSogou],
      ['app.baidu-netdisk-rendering-cache', ICON_NAMES.brandBaiduNetdisk],
      ['app.workbuddy-rendering-cache', ICON_NAMES.brandWorkBuddy],
      ['app.manus-rendering-cache', ICON_NAMES.brandManus],
      ['app.manus-update-cache', ICON_NAMES.brandManus],
      ['app.lobsterai-update-cache', ICON_NAMES.brandLobsterAi],
      ['app.gitmind-rendering-cache', ICON_NAMES.brandGitMind],
      ['app.adobe-media-cache', ICON_NAMES.brandAdobe],
      ['app.thunder-cache', ICON_NAMES.brandXunlei],
      ['app.ynote-cache', ICON_NAMES.brandYoudaoNote],
      ['app.xmind-rendering-cache', ICON_NAMES.brandXmind],
      ['app.postman-cache', ICON_NAMES.brandPostman],
      ['app.vlc-cache', ICON_NAMES.brandVlc],
      ['app.signal-cache', ICON_NAMES.brandSignal],
      ['app.steam-rendering-cache', ICON_NAMES.brandSteam],
      ['app.minecraft-cache', ICON_NAMES.brandMinecraft],
      ['ai.claude-code-cache', ICON_NAMES.brandClaudeCode],
    ] as const;

    for (const [ruleId, expectedIcon] of brandedRules) {
      expect(cleanupRuleIcon(ruleId, 'application')).toBe(expectedIcon);
    }
  });

  it('uses semantic icons for known caches without a dedicated brand', () => {
    expect(cleanupRuleIcon('dev.browser-automation-cache', 'userCache')).toBe(ICON_NAMES.cleanupAutomationCache);
    expect(cleanupRuleIcon('system.geo-services-cache', 'userCache')).toBe(ICON_NAMES.cleanupLocationCache);
    expect(cleanupRuleIcon('system.help-cache', 'userCache')).toBe(ICON_NAMES.help);
    expect(cleanupRuleIcon('system.crash-dumps', 'system')).toBe(ICON_NAMES.cleanupCrashReports);
    expect(cleanupRuleIcon('system.quicklook-cache', 'system')).toBe(ICON_NAMES.cleanupPreviewCache);
    expect(cleanupRuleIcon('system.stale-partial-downloads', 'system')).toBe(ICON_NAMES.download);
    expect(cleanupRuleIcon('app.game-launcher-cache', 'application')).toBe(ICON_NAMES.game);
    expect(cleanupRuleIcon('app.wallpaper-agent-cache', 'application')).toBe(ICON_NAMES.wallpaper);
    expect(cleanupRuleIcon('app.obs-diagnostic-cache', 'application')).toBe(ICON_NAMES.brandObs);
    expect(cleanupRuleIcon('app.thunderbird-cache', 'application')).toBe(ICON_NAMES.brandThunderbird);
    expect(cleanupRuleIcon('browser.duckduckgo-cache', 'browser')).toBe(ICON_NAMES.brandDuckDuckGo);
    expect(cleanupRuleIcon('dev.bun-cache', 'development')).toBe(ICON_NAMES.brandBun);
    expect(cleanupRuleIcon('dev.composer-cache', 'development')).toBe(ICON_NAMES.brandComposer);
    expect(cleanupRuleIcon('dev.maven-cache', 'development')).toBe(ICON_NAMES.brandApacheMaven);
    expect(cleanupRuleIcon('dev.nuget-cache', 'development')).toBe(ICON_NAMES.brandNuget);
    expect(cleanupRuleIcon('dev.rubygems-cache', 'development')).toBe(ICON_NAMES.brandRubyGems);
  });

  it('uses the domain icon when no exact public brand icon is mapped', () => {
    expect(cleanupRuleIcon('dev.unknown-tool-cache', 'userCache')).toBe(ICON_NAMES.cleanupUserCache);
    expect(cleanupRuleIcon('project.unknown-build-artifacts', 'project')).toBe(ICON_NAMES.cleanupProjectArtifacts);
  });

  it('keeps category icons independent from individual rule brands', () => {
    expect(cleanupGroupIcon('browser')).toBe(ICON_NAMES.cleanupBrowserData);
    expect(cleanupGroupIcon('development')).toBe(ICON_NAMES.cleanupDeveloperTools);
    expect(cleanupGroupIcon('xcode')).toBe(ICON_NAMES.brandXcode);
  });

  it('assigns an intentional icon to every catalog entry', () => {
    expect(Object.keys(zhCN.cleanupRules.entries).filter(ruleId => !hasCleanupRuleIcon(ruleId))).toEqual([]);
  });
});
