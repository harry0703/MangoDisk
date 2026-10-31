import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { VolumeInfo } from '@/lib/models/storage/volume';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import { VolumeService } from '@/lib/services/storage/volume-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import { LanguageService } from '@/lib/services/settings/language-service';
import { ThemeService } from '@/lib/services/settings/theme-service';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import { COLOR_THEME_IDS, LANGUAGE_IDS, THEME_IDS } from '@/lib/models/settings/settings';
import type { AppSettings } from '@/lib/models/settings/settings';
import { DUPLICATE_KEEPER_RULE_IDS } from '@/lib/models/storage/duplicate-file';
import { BYTE_UNIT_BASES } from '@/lib/utils/presentation/format';
import * as AppSettingsUtils from '@/lib/utils/settings/app-settings';

import { useAppStore } from './app-store';

describe('app store settings upgrade', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
    vi.spyOn(LanguageService, 'apply').mockImplementation(() => undefined);
    vi.spyOn(ThemeService, 'apply').mockImplementation(() => undefined);
    vi.spyOn(ThemeService, 'applyColorTheme').mockImplementation(() => undefined);
    vi.spyOn(LoggerService, 'warn').mockImplementation(() => undefined);
  });

  it.each([BYTE_UNIT_BASES.binary, BYTE_UNIT_BASES.decimal])(
    'preserves and applies released legacy preferences on startup with base %s',
    async unitBase => {
      const legacy = {
        language: LANGUAGE_IDS.zhTW,
        theme: THEME_IDS.dark,
        largeFileMinimumBytes: 500 * 1024 * 1024,
        duplicateFileMinimumBytes: 10 * 1024 * 1024,
        duplicateKeeperRule: DUPLICATE_KEEPER_RULE_IDS.newestModified,
      };
      vi.spyOn(ByteSizeService, 'currentUnitBase').mockReturnValue(unitBase);
      vi.spyOn(PreferenceStorageService, 'loadSettings').mockResolvedValue(legacy);
      const clear = vi.spyOn(PreferenceStorageService, 'clearSettings').mockResolvedValue();
      const store = useAppStore();

      await store.loadSettings();

      expect(store.settings).toEqual({
        ...legacy,
        hideCleanupReadFailureAlerts: false,
        sidebarExpanded: null,
        colorTheme: COLOR_THEME_IDS.mango,
        largeFileMinimumBytes: 500 * unitBase ** 2,
        duplicateFileMinimumBytes: 10 * unitBase ** 2,
      });
      expect(clear).not.toHaveBeenCalled();
      expect(LoggerService.warn).not.toHaveBeenCalled();
      expect(LanguageService.apply).toHaveBeenCalledWith(legacy.language);
      expect(ThemeService.apply).toHaveBeenCalledWith(legacy.theme);
      expect(ThemeService.applyColorTheme).toHaveBeenCalledWith(COLOR_THEME_IDS.mango);
    }
  );

  it('persists color choices without changing the brightness preference', async () => {
    vi.spyOn(ByteSizeService, 'currentUnitBase').mockReturnValue(BYTE_UNIT_BASES.decimal);
    const save = vi.spyOn(PreferenceStorageService, 'saveSettings').mockResolvedValue();
    const store = useAppStore();
    const settings = {
      ...AppSettingsUtils.defaults(LANGUAGE_IDS.enUS, BYTE_UNIT_BASES.decimal),
      theme: THEME_IDS.dark,
      colorTheme: COLOR_THEME_IDS.warmGray,
    };

    store.saveSettings(settings);

    expect(store.settings).toEqual(settings);
    expect(save).toHaveBeenCalledWith(settings);
    expect(ThemeService.apply).toHaveBeenCalledWith(THEME_IDS.dark);
    expect(ThemeService.applyColorTheme).toHaveBeenCalledWith(COLOR_THEME_IDS.warmGray);
  });

  it.each([true, false])('restores sidebar choice %s after restarting the app store', async sidebarExpanded => {
    vi.spyOn(ByteSizeService, 'currentUnitBase').mockReturnValue(BYTE_UNIT_BASES.binary);
    let persisted: unknown = null;
    vi.spyOn(PreferenceStorageService, 'saveSidebarExpanded').mockImplementation(async (expanded, settings) => {
      persisted = JSON.parse(JSON.stringify(settings));
      expect(settings.sidebarExpanded).toBe(expanded);
    });
    vi.spyOn(PreferenceStorageService, 'loadSettings').mockImplementation(async () => persisted);
    const store = useAppStore();
    const settings = { ...store.settings, theme: THEME_IDS.dark, sidebarExpanded };

    store.settings = { ...store.settings, theme: THEME_IDS.dark };
    store.setSidebarExpanded(sidebarExpanded);
    setActivePinia(createPinia());
    const restarted = useAppStore();
    await restarted.loadSettings();

    expect(restarted.settings).toEqual(settings);
  });

  it('preserves a saved alert choice in the current format', async () => {
    const settings = {
      ...AppSettingsUtils.defaults(LANGUAGE_IDS.jaJP, BYTE_UNIT_BASES.decimal),
      theme: THEME_IDS.light,
      hideCleanupReadFailureAlerts: true,
      colorTheme: COLOR_THEME_IDS.warmGray,
    };
    vi.spyOn(ByteSizeService, 'currentUnitBase').mockReturnValue(BYTE_UNIT_BASES.decimal);
    vi.spyOn(PreferenceStorageService, 'loadSettings').mockResolvedValue(settings);
    const clear = vi.spyOn(PreferenceStorageService, 'clearSettings').mockResolvedValue();
    const store = useAppStore();

    await store.loadSettings();

    expect(store.settings).toEqual(settings);
    expect(clear).not.toHaveBeenCalled();
    expect(LanguageService.apply).toHaveBeenCalledWith(settings.language);
    expect(ThemeService.apply).toHaveBeenCalledWith(settings.theme);
    expect(ThemeService.applyColorTheme).toHaveBeenCalledWith(COLOR_THEME_IDS.warmGray);
  });

  it('preserves valid settings when a saved palette is unavailable', async () => {
    const saved = {
      ...AppSettingsUtils.defaults(LANGUAGE_IDS.zhTW, BYTE_UNIT_BASES.decimal),
      theme: THEME_IDS.dark,
      hideCleanupReadFailureAlerts: true,
      colorTheme: 'unavailable',
    };
    vi.spyOn(ByteSizeService, 'currentUnitBase').mockReturnValue(BYTE_UNIT_BASES.decimal);
    vi.spyOn(PreferenceStorageService, 'loadSettings').mockResolvedValue(saved);
    const clear = vi.spyOn(PreferenceStorageService, 'clearSettings').mockResolvedValue();
    const store = useAppStore();

    await store.loadSettings();

    expect(store.settings).toEqual({ ...saved, colorTheme: COLOR_THEME_IDS.mango });
    expect(clear).not.toHaveBeenCalled();
    expect(LanguageService.apply).toHaveBeenCalledWith(saved.language);
    expect(ThemeService.apply).toHaveBeenCalledWith(saved.theme);
    expect(ThemeService.applyColorTheme).toHaveBeenCalledWith(COLOR_THEME_IDS.mango);
  });

  it('clears a corrupt legacy document and applies safe defaults', async () => {
    vi.spyOn(ByteSizeService, 'currentUnitBase').mockReturnValue(BYTE_UNIT_BASES.decimal);
    vi.spyOn(LanguageService, 'detectSystemLanguage').mockReturnValue(LANGUAGE_IDS.zhCN);
    vi.spyOn(PreferenceStorageService, 'loadSettings').mockResolvedValue({ language: LANGUAGE_IDS.zhTW });
    const clear = vi.spyOn(PreferenceStorageService, 'clearSettings').mockResolvedValue();
    const store = useAppStore();

    await store.loadSettings();

    expect(clear).toHaveBeenCalledOnce();
    expect(store.settings).toEqual(AppSettingsUtils.defaults(LANGUAGE_IDS.zhCN, BYTE_UNIT_BASES.decimal));
    expect(LoggerService.warn).toHaveBeenCalledWith(LOG_DOMAINS.settings, LOG_EVENTS.savedSettingsInvalid, {
      error: expect.any(Error),
    });
    expect(LanguageService.apply).toHaveBeenCalledWith(LANGUAGE_IDS.zhCN);
    expect(ThemeService.apply).toHaveBeenCalledWith(THEME_IDS.system);
  });

  it('keeps unreadable persisted settings intact while applying safe defaults', async () => {
    const error = new Error('settings unavailable');
    vi.spyOn(ByteSizeService, 'currentUnitBase').mockReturnValue(BYTE_UNIT_BASES.decimal);
    vi.spyOn(LanguageService, 'detectSystemLanguage').mockReturnValue(LANGUAGE_IDS.zhCN);
    vi.spyOn(PreferenceStorageService, 'loadSettings').mockRejectedValue(error);
    const clear = vi.spyOn(PreferenceStorageService, 'clearSettings').mockResolvedValue();
    const store = useAppStore();

    await store.loadSettings();

    expect(clear).not.toHaveBeenCalled();
    expect(store.settings).toEqual(AppSettingsUtils.defaults(LANGUAGE_IDS.zhCN, BYTE_UNIT_BASES.decimal));
    expect(LoggerService.warn).toHaveBeenCalledWith(LOG_DOMAINS.settings, LOG_EVENTS.savedSettingsLoadFailed, {
      error,
    });
  });

  it('preserves unreadable settings when the user only changes the sidebar', async () => {
    vi.spyOn(ByteSizeService, 'currentUnitBase').mockReturnValue(BYTE_UNIT_BASES.binary);
    vi.spyOn(LanguageService, 'detectSystemLanguage').mockReturnValue(LANGUAGE_IDS.enUS);
    const saved = {
      ...AppSettingsUtils.defaults(LANGUAGE_IDS.jaJP),
      theme: THEME_IDS.dark,
      colorTheme: COLOR_THEME_IDS.warmGray,
    };
    let persisted: AppSettings = saved;
    vi.spyOn(PreferenceStorageService, 'loadSettings').mockRejectedValue(new Error('temporary read failure'));
    const saveSettings = vi.spyOn(PreferenceStorageService, 'saveSettings').mockImplementation(async settings => {
      persisted = settings;
    });
    vi.spyOn(PreferenceStorageService, 'saveSidebarExpanded').mockImplementation(async expanded => {
      persisted = { ...persisted, sidebarExpanded: expanded };
    });
    const store = useAppStore();
    await store.loadSettings();

    store.setSidebarExpanded(true);

    expect(persisted).toMatchObject({ language: saved.language, theme: saved.theme, colorTheme: saved.colorTheme });
    expect(saveSettings).not.toHaveBeenCalled();
    expect(store.settings.sidebarExpanded).toBe(true);
  });
});

const currentDisk: VolumeInfo = {
  name: 'System',
  mountPoint: '/',
  totalBytes: 1_000,
  availableBytes: 400,
  usedBytes: 600,
};

describe('app store disk refresh', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
  });

  it('publishes a fresh system disk snapshot across shared disk state', async () => {
    const refreshedDisk: VolumeInfo = {
      ...currentDisk,
      availableBytes: 650,
      usedBytes: 350,
    };
    vi.spyOn(VolumeService, 'getSystemVolume').mockResolvedValue(refreshedDisk);
    const store = useAppStore();
    store.disk = currentDisk;
    store.disks = [currentDisk, { ...currentDisk, name: 'External', mountPoint: '/Volumes/External' }];

    await expect(store.refreshSystemDisk()).resolves.toBe(true);

    expect(store.disk).toEqual(refreshedDisk);
    expect(VolumeService.getSystemVolume).toHaveBeenCalledWith(true);
    expect(store.disks).toEqual([refreshedDisk, { ...currentDisk, name: 'External', mountPoint: '/Volumes/External' }]);
  });

  it('requests a new native snapshot for every refresh', async () => {
    const firstSnapshot = { ...currentDisk, availableBytes: 450, usedBytes: 550 };
    const secondSnapshot = { ...currentDisk, availableBytes: 700, usedBytes: 300 };
    const getSystemVolume = vi
      .spyOn(VolumeService, 'getSystemVolume')
      .mockResolvedValueOnce(firstSnapshot)
      .mockResolvedValueOnce(secondSnapshot);
    const store = useAppStore();

    await store.refreshSystemDisk();
    expect(store.disk).toEqual(firstSnapshot);

    await store.refreshSystemDisk();
    expect(store.disk).toEqual(secondSnapshot);
    expect(getSystemVolume).toHaveBeenCalledTimes(2);
  });

  it('keeps the previous snapshot when a secondary refresh fails', async () => {
    vi.spyOn(VolumeService, 'getSystemVolume').mockRejectedValue(new Error('disk refresh failed'));
    const warn = vi.spyOn(LoggerService, 'warn').mockImplementation(() => undefined);
    const store = useAppStore();
    store.disk = currentDisk;

    await expect(store.refreshSystemDisk()).resolves.toBe(false);

    expect(store.disk).toEqual(currentDisk);
    expect(warn).toHaveBeenCalledWith(LOG_DOMAINS.applicationShell, LOG_EVENTS.diskRefreshFailed, {
      code: 'operationFailed',
    });
  });

  it('refreshes system and external volumes together', async () => {
    const disks = [
      { ...currentDisk, availableBytes: 700, usedBytes: 300 },
      { ...currentDisk, mountPoint: '/Volumes/External' },
    ];
    vi.spyOn(VolumeService, 'listVolumes').mockResolvedValue(disks);
    const store = useAppStore();
    store.disk = currentDisk;
    await expect(store.refreshDisks()).resolves.toBe(true);
    expect(store.disk).toEqual(disks[0]);
    expect(store.disks).toEqual(disks);
  });

  it('recovers the system capacity after a transient startup failure', async () => {
    vi.spyOn(LoggerService, 'error').mockImplementation(() => undefined);
    vi.spyOn(VolumeService, 'getSystemVolume')
      .mockRejectedValueOnce(new Error('temporary capacity failure'))
      .mockResolvedValue(currentDisk);
    const disks = [{ ...currentDisk, mountPoint: '/Volumes/External' }, currentDisk];
    vi.spyOn(VolumeService, 'listVolumes').mockResolvedValue(disks);
    const store = useAppStore();
    await store.initialize();
    expect(store.disk).toBeNull();
    await expect(store.refreshDisks()).resolves.toBe(true);
    expect(store.disk).toEqual(currentDisk);
    expect(store.disks).toEqual(disks);
  });

  it('does not publish a startup snapshot older than a completed forced refresh', async () => {
    let complete!: (disk: VolumeInfo) => void;
    const fresh = { ...currentDisk, availableBytes: 800, usedBytes: 200 };
    vi.spyOn(VolumeService, 'getSystemVolume')
      .mockImplementationOnce(() => new Promise(resolve => (complete = resolve)))
      .mockResolvedValue(fresh);
    vi.spyOn(VolumeService, 'listVolumes').mockResolvedValue([currentDisk]);
    const store = useAppStore();
    const initialization = store.initialize();
    await store.refreshSystemDisk();
    complete(currentDisk);
    await initialization;
    expect(store.disk).toEqual(fresh);
  });

  it('does not report a late startup failure after a newer capacity snapshot succeeds', async () => {
    let fail!: (error: Error) => void;
    vi.spyOn(LoggerService, 'error').mockImplementation(() => undefined);
    vi.spyOn(VolumeService, 'getSystemVolume')
      .mockImplementationOnce(() => new Promise((_, reject) => (fail = reject)))
      .mockResolvedValue(currentDisk);
    vi.spyOn(VolumeService, 'listVolumes').mockResolvedValue([currentDisk]);
    const store = useAppStore();
    const initialization = store.initialize();
    await store.refreshSystemDisk();
    fail(new Error('old startup failure'));
    await initialization;
    expect(store.disk).toEqual(currentDisk);
    expect(store.errorCode).toBeNull();
    expect(LoggerService.error).not.toHaveBeenCalled();
  });

  it('rejects a periodic response older than the post-cleanup snapshot', async () => {
    let complete!: (disks: VolumeInfo[]) => void;
    vi.spyOn(VolumeService, 'listVolumes').mockImplementation(
      () =>
        new Promise(resolve => {
          complete = resolve;
        })
    );
    const fresh = { ...currentDisk, availableBytes: 800, usedBytes: 200 };
    vi.spyOn(VolumeService, 'getSystemVolume').mockResolvedValue(fresh);
    const store = useAppStore();
    store.disk = currentDisk;
    store.disks = [currentDisk];
    const periodic = store.refreshDisks();
    await store.refreshSystemDisk();
    complete([currentDisk]);
    await expect(periodic).resolves.toBe(false);
    expect(store.disk).toEqual(fresh);
    expect(store.disks).toEqual([fresh]);
  });

  it('does not let an older forced response replace a newer forced refresh', async () => {
    const completions: ((disk: VolumeInfo) => void)[] = [];
    vi.spyOn(VolumeService, 'getSystemVolume').mockImplementation(
      () =>
        new Promise(resolve => {
          completions.push(resolve);
        })
    );
    const store = useAppStore();
    const first = store.refreshSystemDisk();
    const second = store.refreshSystemDisk();
    await expect(store.refreshDisks()).resolves.toBe(false);
    const fresh = { ...currentDisk, availableBytes: 900, usedBytes: 100 };
    completions[1]!(fresh);
    await expect(second).resolves.toBe(true);
    completions[0]!(currentDisk);
    await expect(first).resolves.toBe(false);
    expect(store.disk).toEqual(fresh);
    expect(store.pendingSystemDiskRefreshes).toBe(0);
  });
});
