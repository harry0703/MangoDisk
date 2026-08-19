import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { LargeFileEntry } from '@/lib/models/large-file';
import { AiAdvisorService } from '@/lib/services/ai-advisor-service';
import { LoggerService } from '@/lib/services/logger-service';

function createEntry(name: string, path: string): LargeFileEntry {
  return {
    name,
    path,
    parentPath: '/test',
    bytes: 1024 * 1024 * 100,
    modifiedAtMs: 1700000000000,
  };
}

describe('AiAdvisorService', () => {
  let fetchMock: any;

  beforeEach(() => {
    vi.spyOn(LoggerService, 'info').mockImplementation(() => {});
    vi.spyOn(LoggerService, 'warn').mockImplementation(() => {});
    fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        choices: [{
          message: {
            content: JSON.stringify([0, 1, 2, 3, 4, 5])
          }
        }]
      })
    });
    global.fetch = fetchMock;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('filters recommended files matching deletion extensions', async () => {
    const files: LargeFileEntry[] = [
      createEntry('disk.vhdx', '/test/disk.vhdx'),
      createEntry('installer.iso', '/test/installer.iso'),
      createEntry('system.IMG', '/test/system.IMG'),
      createEntry('virtual.vmdk', '/test/virtual.vmdk'),
      createEntry('data.ucas', '/test/data.ucas'),
      createEntry('app.log', '/test/app.log'),
      createEntry('document.pdf', '/test/document.pdf'),
      createEntry('movie.mp4', '/test/movie.mp4'),
      createEntry('archive.zip', '/test/archive.zip'),
    ];

    const result = await AiAdvisorService.analyzeLargeFiles(files, { baseUrl: 'https://test.api' });

    expect(result).toEqual([
      '/test/disk.vhdx',
      '/test/installer.iso',
      '/test/system.IMG',
      '/test/virtual.vmdk',
      '/test/data.ucas',
      '/test/app.log',
    ]);
    expect(LoggerService.info).toHaveBeenCalledWith('ai-advisor', 'analyze_large_files_started', {
      fileCount: 9,
    });
    expect(LoggerService.info).toHaveBeenCalledWith('ai-advisor', 'analyze_large_files_completed', {
      fileCount: 9,
      recommendedCount: 6,
    });
  });

  it('aborts when abort signal is triggered', async () => {
    const controller = new AbortController();
    const files = [createEntry('disk.vhdx', '/test/disk.vhdx')];

    fetchMock.mockImplementation(async () => {
      return new Promise((_, reject) => {
        controller.signal.addEventListener('abort', () => {
          const error = new Error('AbortError');
          error.name = 'AbortError';
          reject(error);
        });
      });
    });

    const promise = AiAdvisorService.analyzeLargeFiles(files, {}, controller.signal);
    controller.abort();

    await expect(promise).rejects.toThrow();
  });

  it('immediately rejects if already aborted', async () => {
    const controller = new AbortController();
    controller.abort();
    const files = [createEntry('disk.vhdx', '/test/disk.vhdx')];

    await expect(AiAdvisorService.analyzeLargeFiles(files, {}, controller.signal)).rejects.toThrow();
  });

  it('analyzeCleanupRules recommends rules containing cache or temp in name', async () => {
    const rules = [
      { id: '1', name: 'System Cache' },
      { id: '2', name: 'Browser Temp Files' },
      { id: '3', name: 'Important Data' }
    ];
    const result = await AiAdvisorService.analyzeCleanupRules(rules);
    expect(result).toEqual(['1', '2']);
  });

  it('analyzeDuplicateFiles recommends deleting newer copies', async () => {
    const groups = [
      {
        files: [
          { path: '/a/new.txt', modifiedTime: 2000 },
          { path: '/b/old.txt', modifiedTime: 1000 }
        ]
      }
    ];
    const result = await AiAdvisorService.analyzeDuplicateFiles(groups);
    expect(result).toEqual(['/a/new.txt']);
  });

  it('analyzeApplications recommends toolbars and bloatware', async () => {
    const apps = [
      { id: 'app1', name: 'Ask Toolbar' },
      { id: 'app2', name: 'OEM Bloatware' },
      { id: 'app3', name: 'Good Software' }
    ];
    const result = await AiAdvisorService.analyzeApplications(apps);
    expect(result).toEqual(['app1', 'app2']);
  });

  it('analyzeStartupItems recommends updaters and helpers', async () => {
    const items = [
      { id: 's1', name: 'Adobe Updater' },
      { id: 's2', name: 'iTunes Helper' },
      { id: 's3', name: 'Windows Defender' }
    ];
    const result = await AiAdvisorService.analyzeStartupItems(items);
    expect(result).toEqual(['s1', 's2']);
  });
});
