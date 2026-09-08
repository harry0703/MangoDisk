import { beforeEach, describe, expect, it, vi } from 'vitest';

import { InstallationIdentityService } from '@/lib/services/installation-identity-service';

const { loadMock, saveMock, setMock, values } = vi.hoisted(() => {
  const storedValues = new Map<string, unknown>();
  const save = vi.fn(async () => undefined);
  const set = vi.fn(async (key: string, value: unknown) => {
    storedValues.set(key, value);
  });
  const store = {
    get: vi.fn(async (key: string) => storedValues.get(key)),
    set,
    save,
  };
  return {
    loadMock: vi.fn(async () => store),
    saveMock: save,
    setMock: set,
    values: storedValues,
  };
});

vi.mock('@tauri-apps/plugin-store', () => ({ load: loadMock }));

describe('InstallationIdentityService', () => {
  beforeEach(() => {
    values.clear();
    loadMock.mockClear();
    saveMock.mockClear();
    setMock.mockClear();
  });

  it('creates and persists a random installation identifier', async () => {
    const installId = await InstallationIdentityService.getOrCreateInstallId();

    expect(installId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
    expect(loadMock).toHaveBeenCalledWith('installation.json', { autoSave: false });
    expect(values.get('identity')).toEqual({
      schemaVersion: 1,
      installId,
    });
    expect(saveMock).toHaveBeenCalledOnce();
  });

  it('reuses a valid stored installation identifier', async () => {
    const installId = '019c0b3d-a9ef-7d11-89a3-d5ea10df4001';
    values.set('identity', {
      schemaVersion: 1,
      installId,
    });

    await expect(InstallationIdentityService.getOrCreateInstallId()).resolves.toBe(installId);
    expect(setMock).not.toHaveBeenCalled();
    expect(saveMock).not.toHaveBeenCalled();
  });

  it('shares one persisted identity across simultaneous callers', async () => {
    const ids = await Promise.all(Array.from({ length: 20 }, () => InstallationIdentityService.getOrCreateInstallId()));
    expect(new Set(ids).size).toBe(1);
    expect(saveMock).toHaveBeenCalledOnce();
  });

  it('replaces malformed installation data instead of accepting it', async () => {
    values.set('identity', {
      schemaVersion: 1,
      installId: 'hardware-derived-value',
    });

    const installId = await InstallationIdentityService.getOrCreateInstallId();

    expect(installId).not.toBe('hardware-derived-value');
    expect(values.get('identity')).toEqual({
      schemaVersion: 1,
      installId,
    });
    expect(saveMock).toHaveBeenCalledOnce();
  });
});
