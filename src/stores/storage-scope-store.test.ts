import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { VolumeInfo } from '@/lib/models/storage/volume';
import { STORAGE_SCOPE_IDS } from '@/lib/models/storage/storage-scope';
import { FolderSelectionService } from '@/lib/services/storage/folder-selection-service';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import { STANDARD_SCAN_FOLDER_IDS } from '@/lib/models/storage/standard-scan-folder';
import { StandardScanFolderService } from '@/lib/services/storage/standard-scan-folder-service';

import { useStorageScopeStore } from './storage-scope-store';

const { loadMock, values } = vi.hoisted(() => {
  const storedValues = new Map<string, unknown>();
  const store = {
    get: vi.fn(async (key: string) => storedValues.get(key)),
    set: vi.fn(async (key: string, value: unknown) => {
      storedValues.set(key, value);
    }),
    delete: vi.fn(async (key: string) => storedValues.delete(key)),
    save: vi.fn(async () => undefined),
  };
  return {
    loadMock: vi.fn(async () => store),
    values: storedValues,
  };
});

vi.mock('@tauri-apps/plugin-store', () => ({ load: loadMock }));
const disks: VolumeInfo[] = [
  {
    name: 'System',
    mountPoint: '/',
    totalBytes: 1_000,
    availableBytes: 400,
    usedBytes: 600,
  },
];

describe('storage scope store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    values.clear();
    vi.restoreAllMocks();
    vi.spyOn(StandardScanFolderService, 'list').mockResolvedValue([
      { id: STANDARD_SCAN_FOLDER_IDS.downloads, path: '/Users/example/Downloads' },
    ]);
  });

  it('restores every selected directory independently of the eight-item history', async () => {
    const paths = Array.from({ length: 70 }, (_, index) => `/fixture/${index}`);
    const store = useStorageScopeStore();
    store.selectPaths('duplicate-files', paths, disks);
    expect(store.recentFolders).toHaveLength(8);
    const recent = [...store.recentFolders];
    store.selectPaths('duplicate-files', paths.slice(1), disks);
    expect(store.recentFolders).toEqual(recent);
    store.selectPaths('duplicate-files', paths, disks);
    const resolve = vi
      .spyOn(FolderSelectionService, 'resolveDirectories')
      .mockImplementation(async values => values.map(path => ({ requestedPath: path, path })));
    setActivePinia(createPinia());
    const restored = useStorageScopeStore();
    await restored.initialize();
    expect(restored.selectedPaths['duplicate-files']).toEqual(paths);
    expect(resolve).not.toHaveBeenCalled();
    expect(restored.recentFolders).toHaveLength(8);
    restored.removeFolder(paths[0]!);
    expect(restored.selectedPaths['duplicate-files']).toEqual(paths.slice(1));
  });

  it('persists deselecting every location and deduplicates Windows aliases', async () => {
    const store = useStorageScopeStore();
    store.selectPaths('duplicate-files', ['E:\\Work', 'e:/work/', 'F:\\Chat'], disks);
    expect(store.selectedPaths['duplicate-files']).toEqual(['E:\\Work', 'F:\\Chat']);
    store.selectPaths('duplicate-files', [], disks);
    await expect(PreferenceStorageService.loadStorageScopePreferences()).resolves.toMatchObject({
      selectedPaths: { 'duplicate-files': [] },
    });
  });

  it('keeps large-file and duplicate multi-selections independent', async () => {
    const store = useStorageScopeStore();
    store.selectPaths('duplicate-files', ['/chat'], disks);
    store.selectPaths('large-files', ['/work', '/downloads'], disks);
    store.selectPaths('large-files', [], disks);
    await expect(PreferenceStorageService.loadStorageScopePreferences()).resolves.toMatchObject({
      schemaVersion: 3,
      selectedPaths: { 'large-files': [], 'duplicate-files': ['/chat'] },
      duplicateFileProtectedPaths: [],
    });
  });

  it('persists separate page selections and shared recent folders', async () => {
    const store = useStorageScopeStore();

    store.select(STORAGE_SCOPE_IDS.analysis, '/Users/example/Downloads', disks);
    store.select(STORAGE_SCOPE_IDS.largeFiles, '/Users/example/Movies', disks);
    store.select(STORAGE_SCOPE_IDS.duplicateFiles, '/Users/example/Documents', disks);

    expect(store.selectedPath(STORAGE_SCOPE_IDS.analysis)).toBe('/Users/example/Downloads');
    expect(store.selectedPath(STORAGE_SCOPE_IDS.largeFiles)).toBe('/Users/example/Movies');
    expect(store.selectedPath(STORAGE_SCOPE_IDS.duplicateFiles)).toBe('/Users/example/Documents');
    expect(store.recentFolders).toEqual([
      '/Users/example/Documents',
      '/Users/example/Movies',
      '/Users/example/Downloads',
    ]);
    await expect(PreferenceStorageService.loadStorageScopePreferences()).resolves.toEqual({
      schemaVersion: 3,
      selectedPaths: {
        analysis: '/Users/example/Downloads',
        'large-files': ['/Users/example/Movies'],
        'duplicate-files': ['/Users/example/Documents'],
      },
      recentFolders: ['/Users/example/Documents', '/Users/example/Movies', '/Users/example/Downloads'],
      duplicateFileProtectedPaths: [],
    });
  });

  it('persists duplicate-file protection separately from scan selection', async () => {
    const store = useStorageScopeStore();
    store.selectPaths('duplicate-files', ['E:\\Work', 'F:\\Chat'], disks);
    store.setDuplicateFileProtectedPaths(['E:\\Work', 'G:\\Stale']);

    await expect(PreferenceStorageService.loadStorageScopePreferences()).resolves.toMatchObject({
      schemaVersion: 3,
      selectedPaths: { 'duplicate-files': ['E:\\Work', 'F:\\Chat'] },
      duplicateFileProtectedPaths: ['E:\\Work'],
    });

    store.selectPaths('duplicate-files', ['F:\\Chat'], disks);
    expect(store.duplicateFileProtectedPaths).toEqual([]);
  });

  it('does not add disk roots to folder history', () => {
    const store = useStorageScopeStore();

    store.select(STORAGE_SCOPE_IDS.analysis, '/', disks);

    expect(store.selectedPath(STORAGE_SCOPE_IDS.analysis)).toBe('/');
    expect(store.recentFolders).toEqual([]);
  });

  it('shares one in-flight initialization and publishes only the completed snapshot', async () => {
    const store = useStorageScopeStore();
    let resolveStandardFolders!: (folders: Awaited<ReturnType<typeof StandardScanFolderService.list>>) => void;
    vi.mocked(StandardScanFolderService.list).mockImplementationOnce(
      () =>
        new Promise(resolve => {
          resolveStandardFolders = resolve;
        })
    );

    const firstInitialization = store.initialize();
    const secondInitialization = store.initialize();
    let secondInitializationCompleted = false;
    void secondInitialization.then(() => {
      secondInitializationCompleted = true;
    });

    await Promise.resolve();
    expect(store.initialized).toBe(false);
    expect(secondInitializationCompleted).toBe(false);
    expect(StandardScanFolderService.list).toHaveBeenCalledTimes(1);

    resolveStandardFolders([{ id: STANDARD_SCAN_FOLDER_IDS.downloads, path: '/Users/example/Downloads' }]);
    await Promise.all([firstInitialization, secondInitialization]);
    await store.initialize();

    expect(store.initialized).toBe(true);
    expect(store.standardFolders).toEqual([
      { id: STANDARD_SCAN_FOLDER_IDS.downloads, path: '/Users/example/Downloads' },
    ]);
    expect(StandardScanFolderService.list).toHaveBeenCalledTimes(1);
  });

  it('allows initialization to retry after an unexpected failure', async () => {
    const store = useStorageScopeStore();
    vi.spyOn(store, 'loadStandardFolders').mockRejectedValueOnce(new Error('unexpected initialization failure'));

    await expect(store.initialize()).rejects.toThrow('unexpected initialization failure');
    expect(store.initialized).toBe(false);

    await expect(store.initialize()).resolves.toBeUndefined();
    expect(store.initialized).toBe(true);
    expect(StandardScanFolderService.list).toHaveBeenCalledTimes(1);
  });

  it('removes a folder from history and every page selection', () => {
    const store = useStorageScopeStore();
    store.select(STORAGE_SCOPE_IDS.analysis, '/Users/example/Downloads', disks);
    store.select(STORAGE_SCOPE_IDS.largeFiles, '/Users/example/Downloads', disks);
    store.select(STORAGE_SCOPE_IDS.duplicateFiles, '/Users/example/Downloads', disks);

    store.removeFolder('/Users/example/Downloads');

    expect(store.selectedPaths).toEqual({ 'large-files': [], 'duplicate-files': [] });
    expect(store.recentFolders).toEqual([]);
  });

  it('keeps saved aliases unchanged until an explicit scan', async () => {
    const alias = 'C:\\Mac\\Home\\Documents';
    const target = '\\\\psf\\Home\\Documents';
    await PreferenceStorageService.saveStorageScopePreferences({
      selectedPaths: { analysis: alias, 'large-files': target },
      recentFolders: [alias, target],
    });
    vi.spyOn(FolderSelectionService, 'resolveDirectories').mockResolvedValue([
      { requestedPath: alias, path: target },
      { requestedPath: target, path: target },
    ]);
    const store = useStorageScopeStore();
    await store.initialize();
    expect(store.recentFolders).toEqual([alias, target]);
    expect(store.selectedPaths).toEqual({ analysis: alias, 'large-files': [target] });
    expect(FolderSelectionService.resolveDirectories).not.toHaveBeenCalled();
  });

  it('does not rewrite unchanged folder preferences during restoration', async () => {
    const path = '/Users/example/Downloads';
    await PreferenceStorageService.saveStorageScopePreferences({
      selectedPaths: { analysis: path },
      recentFolders: [path],
    });
    vi.spyOn(FolderSelectionService, 'resolveDirectories').mockResolvedValue([{ requestedPath: path, path }]);
    const saveSpy = vi.spyOn(PreferenceStorageService, 'saveStorageScopePreferences');

    await useStorageScopeStore().initialize();

    expect(saveSpy).not.toHaveBeenCalled();
  });

  it('retains unavailable folders and never probes them during restoration', async () => {
    await PreferenceStorageService.saveStorageScopePreferences({
      selectedPaths: {
        analysis: '/Users/example/Missing',
        'large-files': '/Users/example/Downloads',
      },
      recentFolders: ['/Users/example/Missing', '/Users/example/Downloads'],
    });
    vi.spyOn(FolderSelectionService, 'resolveDirectories').mockResolvedValue([
      { requestedPath: '/Users/example/Downloads', path: '/Users/example/Downloads' },
    ]);

    const store = useStorageScopeStore();
    await store.initialize();

    expect(store.selectedPaths).toEqual({
      analysis: '/Users/example/Missing',
      'large-files': ['/Users/example/Downloads'],
    });
    expect(store.recentFolders).toEqual(['/Users/example/Missing', '/Users/example/Downloads']);
    expect(FolderSelectionService.resolveDirectories).not.toHaveBeenCalled();
  });
});
