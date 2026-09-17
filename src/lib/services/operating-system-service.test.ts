import { beforeEach, describe, expect, it, vi } from 'vitest';

import { OperatingSystemService } from '@/lib/services/operating-system-service';

const { platformMock } = vi.hoisted(() => ({ platformMock: vi.fn() }));

vi.mock('@tauri-apps/plugin-os', () => ({ platform: platformMock }));

describe('OperatingSystemService', () => {
  beforeEach(() => {
    platformMock.mockReset();
  });

  it('uses the official platform value for macOS detection', () => {
    platformMock.mockReturnValue('macos');

    expect(OperatingSystemService.currentPlatform()).toBe('macos');
    expect(OperatingSystemService.isMacOs()).toBe(true);
    expect(OperatingSystemService.isWindows()).toBe(false);
  });

  it('uses the official platform value for Windows detection', () => {
    platformMock.mockReturnValue('windows');

    expect(OperatingSystemService.currentPlatform()).toBe('windows');
    expect(OperatingSystemService.isMacOs()).toBe(false);
    expect(OperatingSystemService.isWindows()).toBe(true);
    expect(OperatingSystemService.isLinux()).toBe(false);
  });

  it('uses the official platform value for Linux detection', () => {
    platformMock.mockReturnValue('linux');

    expect(OperatingSystemService.currentPlatform()).toBe('linux');
    expect(OperatingSystemService.isMacOs()).toBe(false);
    expect(OperatingSystemService.isWindows()).toBe(false);
    expect(OperatingSystemService.isLinux()).toBe(true);
  });
});
