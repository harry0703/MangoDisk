import { beforeEach, describe, expect, it, vi } from 'vitest';

const pathMocks = vi.hoisted(() => ({
  audioDir: vi.fn(),
  documentDir: vi.fn(),
  downloadDir: vi.fn(),
  pictureDir: vi.fn(),
  videoDir: vi.fn(),
  resolveDirectories: vi.fn(),
}));

vi.mock('@tauri-apps/api/path', () => ({
  audioDir: pathMocks.audioDir,
  documentDir: pathMocks.documentDir,
  downloadDir: pathMocks.downloadDir,
  pictureDir: pathMocks.pictureDir,
  videoDir: pathMocks.videoDir,
}));

vi.mock('@/lib/services/storage/folder-selection-service', () => ({
  FolderSelectionService: {
    resolveDirectories: pathMocks.resolveDirectories,
  },
}));

import { STANDARD_SCAN_FOLDER_IDS } from '@/lib/models/storage/standard-scan-folder';
import { StandardScanFolderService } from '@/lib/services/storage/standard-scan-folder-service';

describe('StandardScanFolderService', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    pathMocks.downloadDir.mockResolvedValue('/Users/test/Downloads');
    pathMocks.documentDir.mockResolvedValue('/Users/test/Documents');
    pathMocks.pictureDir.mockResolvedValue('/Users/test/Pictures');
    pathMocks.videoDir.mockResolvedValue('/Users/test/Movies');
    pathMocks.audioDir.mockResolvedValue('/Users/test/Music');
    pathMocks.resolveDirectories.mockImplementation(async (paths: string[]) =>
      paths.map(path => ({ requestedPath: path, path }))
    );
  });

  it('returns standard folders in a stable product order', async () => {
    const folders = await StandardScanFolderService.list();

    expect(folders.map(folder => folder.id)).toEqual([
      STANDARD_SCAN_FOLDER_IDS.downloads,
      STANDARD_SCAN_FOLDER_IDS.documents,
      STANDARD_SCAN_FOLDER_IDS.pictures,
      STANDARD_SCAN_FOLDER_IDS.videos,
      STANDARD_SCAN_FOLDER_IDS.music,
    ]);
  });

  it('omits unresolved or empty operating-system paths without filesystem access', async () => {
    pathMocks.pictureDir.mockRejectedValue(new Error('picture directory unavailable'));
    pathMocks.videoDir.mockResolvedValue('');

    const folders = await StandardScanFolderService.list();

    expect(folders.map(folder => folder.id)).toEqual([
      STANDARD_SCAN_FOLDER_IDS.downloads,
      STANDARD_SCAN_FOLDER_IDS.documents,
      STANDARD_SCAN_FOLDER_IDS.music,
    ]);
  });

  it('keeps redirected Windows shortcut labels without inspecting their targets', async () => {
    pathMocks.documentDir.mockResolvedValue('C:\\Mac\\Home\\Documents');
    const folders = await StandardScanFolderService.list();
    expect(folders).toContainEqual({ id: 'documents', path: 'C:\\Mac\\Home\\Documents' });
    expect(pathMocks.resolveDirectories).not.toHaveBeenCalled();
  });

  it('does not render duplicate shortcuts when operating-system paths are identical', async () => {
    for (const resolve of [
      pathMocks.downloadDir,
      pathMocks.documentDir,
      pathMocks.pictureDir,
      pathMocks.videoDir,
      pathMocks.audioDir,
    ]) {
      resolve.mockResolvedValue('\\\\server\\share\\folder');
    }
    expect(await StandardScanFolderService.list()).toEqual([{ id: 'downloads', path: '\\\\server\\share\\folder' }]);
    expect(pathMocks.resolveDirectories).not.toHaveBeenCalled();
  });
});
