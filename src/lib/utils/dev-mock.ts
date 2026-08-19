/**
 * Development mock for browser preview (when running outside Tauri runtime).
 * This intercepts Tauri IPC calls and provides safe fallback/mock data so the
 * UI can be previewed and tested directly in any browser at localhost:1420.
 */

export function setupBrowserDevMock() {
  if (typeof window === 'undefined') return;

  // Only activate when NOT running inside Tauri
  const isTauri = Boolean(
    (window as unknown as { __TAURI_INTERNALS__?: { invoke?: unknown } }).__TAURI_INTERNALS__?.invoke
  );
  if (isTauri) return;

  console.log('[DevMock] Initializing browser development mock for MangoDisk UI preview...');

  const mockDisks = [
    {
      name: '系统盘 (C:)',
      mountPoint: 'C:\\',
      totalBytes: 512 * 1024 * 1024 * 1024,
      availableBytes: 128 * 1024 * 1024 * 1024,
      usedBytes: 384 * 1024 * 1024 * 1024,
      isSystem: true,
      isRemovable: false,
    },
    {
      name: '工作盘 (E:)',
      mountPoint: 'E:\\',
      totalBytes: 1024 * 1024 * 1024 * 1024,
      availableBytes: 394 * 1024 * 1024 * 1024,
      usedBytes: 630 * 1024 * 1024 * 1024,
      isSystem: false,
      isRemovable: false,
    },
  ];

  const mockLargeFiles = [
    {
      name: 'The.Irishman.2019.2160p.mkv',
      path: 'E:\\AA movie\\The.Irishman.2019.2160p.mkv',
      parentPath: 'E:\\AA movie',
      bytes: 68.61 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 30 * 86400000,
    },
    {
      name: 'Stand.by.Me.1986.2160p.mkv',
      path: 'E:\\AA movie\\Stand.by.Me.1986.2160p.mkv',
      parentPath: 'E:\\AA movie',
      bytes: 50.36 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 25 * 86400000,
    },
    {
      name: 'Fight.Club.1999.1080p.mkv',
      path: 'E:\\AA movie\\Fight.Club.1999.1080p.mkv',
      parentPath: 'E:\\AA movie',
      bytes: 29.0 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 15 * 86400000,
    },
    {
      name: 'Reacher.S01E02.2160p.mkv',
      path: 'E:\\AA perform\\侠探杰克\\Reacher.S01E02.2160p.mkv',
      parentPath: 'E:\\AA perform\\侠探杰克',
      bytes: 18.21 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 10 * 86400000,
    },
    {
      name: 'ext4.vhdx',
      path: 'E:\\WSL\\Ubuntu\\ext4.vhdx',
      parentPath: 'E:\\WSL\\Ubuntu',
      bytes: 6.97 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 5 * 86400000,
    },
    {
      name: 'Windows.iso',
      path: 'E:\\ClaudeCode\\PROJECTS\\seb-bypass\\Windows.iso',
      parentPath: 'E:\\ClaudeCode\\PROJECTS\\seb-bypass',
      bytes: 4.42 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 2 * 86400000,
    },
    {
      name: 'system.img',
      path: 'E:\\AndroidSDK\\system-images\\android-31\\google_apis\\x86_64\\system.img',
      parentPath: 'E:\\AndroidSDK\\system-images\\android-31\\google_apis\\x86_64',
      bytes: 4.01 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 60 * 86400000,
    },
    {
      name: 'Windows 10 x64-s001.vmdk',
      path: 'E:\\VMware\\Windows 10 x64-s001.vmdk',
      parentPath: 'E:\\VMware',
      bytes: 3.97 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 8 * 86400000,
    },
    {
      name: 'Windows 10 x64-s002.vmdk',
      path: 'E:\\VMware\\Windows 10 x64-s002.vmdk',
      parentPath: 'E:\\VMware',
      bytes: 3.97 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 8 * 86400000,
    },
    {
      name: 'pakchunk0_s11-WindowsClient.ucas',
      path: 'E:\\SteamLibrary\\steamapps\\downloading\\2073850\\Discovery\\Content\\Paks\\pakchunk0_s11-WindowsClient.ucas',
      parentPath: 'E:\\SteamLibrary\\steamapps\\downloading\\2073850\\Discovery\\Content\\Paks',
      bytes: 2.95 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 1 * 86400000,
    },
    {
      name: 'pakchunk0_s12-WindowsClient.ucas',
      path: 'E:\\SteamLibrary\\steamapps\\downloading\\2073850\\Discovery\\Content\\Paks\\pakchunk0_s12-WindowsClient.ucas',
      parentPath: 'E:\\SteamLibrary\\steamapps\\downloading\\2073850\\Discovery\\Content\\Paks',
      bytes: 2.74 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 1 * 86400000,
    },
    {
      name: 'pakchunk0_s9-WindowsClient.ucas',
      path: 'E:\\SteamLibrary\\steamapps\\common\\The Finals\\Discovery\\Content\\Paks\\pakchunk0_s9-WindowsClient.ucas',
      parentPath: 'E:\\SteamLibrary\\steamapps\\common\\The Finals\\Discovery\\Content\\Paks',
      bytes: 2.51 * 1024 * 1024 * 1024,
      modifiedAtMs: Date.now() - 40 * 86400000,
    },
  ];

  const storeData: Record<string, unknown> = JSON.parse(localStorage.getItem('mangodisk_dev_store') || '{}');

  const invokeHandler = async (cmd: string, args: Record<string, unknown> = {}): Promise<unknown> => {
    console.log('[DevMock IPC]', cmd, args);

    if (cmd === 'get_system_disk') return mockDisks[0];
    if (cmd === 'list_disks') return mockDisks;

    if (cmd === 'find_large_files') {
      const minBytes = Number(args.minimumBytes || 100 * 1024 * 1024);
      const filtered = mockLargeFiles.filter(f => f.bytes >= minBytes);
      const totalBytes = filtered.reduce((acc, f) => acc + f.bytes, 0);
      return {
        scanId: 1001,
        root: String(args.path || 'E:\\'),
        scannedAtMs: Date.now(),
        minimumBytes: minBytes,
        totalBytes,
        totalCount: filtered.length,
        returnedCount: filtered.length,
        truncated: false,
        skippedCount: 0,
        cacheReused: false,
        entries: filtered,
      };
    }

    if (cmd === 'plugin:store|load') return 1;
    if (cmd === 'plugin:store|get_store') return 1;
    if (cmd === 'plugin:store|get') {
      const key = String(args.key || '');
      const exists = key in storeData;
      return [storeData[key] ?? null, exists];
    }
    if (cmd === 'plugin:store|set') {
      const key = String(args.key || '');
      storeData[key] = args.value;
      localStorage.setItem('mangodisk_dev_store', JSON.stringify(storeData));
      return null;
    }
    if (cmd === 'plugin:store|save') return null;
    if (cmd === 'plugin:store|delete') {
      const key = String(args.key || '');
      delete storeData[key];
      localStorage.setItem('mangodisk_dev_store', JSON.stringify(storeData));
      return null;
    }

    if (cmd === 'plugin:os|platform') return 'windows';
    if (cmd === 'plugin:os|locale') return 'zh-CN';
    if (cmd === 'plugin:log|log') return null;
    if (cmd === 'plugin:process|exit') return null;

    if (cmd.startsWith('plugin:window')) return null;

    return null;
  };

  const transformCallback = (cb: unknown) => {
    return typeof cb === 'function' ? cb : () => undefined;
  };

  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
    invoke: invokeHandler,
    transformCallback,
    plugins: {
      path: { sep: () => '\\' },
    },
  };

  (window as unknown as Record<string, unknown>).__TAURI_OS_PLUGIN_INTERNALS__ = {
    eol: '\r\n',
    platform: 'windows',
    version: '10.0.0',
    family: 'windows',
    os_type: 'windows',
    arch: 'x86_64',
    exe_extension: 'exe',
  };
}
