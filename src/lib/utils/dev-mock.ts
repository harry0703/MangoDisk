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

  console.log('[DevMock] Initializing full browser development mock for MangoDisk UI preview...');

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
  ];

  const mockDuplicateGroups = [
    {
      id: 'grp-1',
      hash: 'hash-photo-1',
      kind: 'file',
      bytesPerFile: 18.5 * 1024 * 1024,
      fileCountPerEntry: 1,
      reclaimableBytes: 18.5 * 1024 * 1024,
      entries: [
        {
          name: 'IMG_20260715_RAW.dng',
          path: 'E:\\Photos\\2026-Summer\\IMG_20260715_RAW.dng',
          parentPath: 'E:\\Photos\\2026-Summer',
          bytes: 18.5 * 1024 * 1024,
          modifiedAtMs: Date.now() - 30 * 86400000,
        },
        {
          name: 'IMG_20260715_RAW (1).dng',
          path: 'C:\\Users\\26502\\Downloads\\IMG_20260715_RAW (1).dng',
          parentPath: 'C:\\Users\\26502\\Downloads',
          bytes: 18.5 * 1024 * 1024,
          modifiedAtMs: Date.now() - 5 * 86400000,
        },
      ],
    },
    {
      id: 'grp-2',
      hash: 'hash-installer-2',
      kind: 'file',
      bytesPerFile: 145 * 1024 * 1024,
      fileCountPerEntry: 1,
      reclaimableBytes: 290 * 1024 * 1024,
      entries: [
        {
          name: 'NodeJS_v22.exe',
          path: 'E:\\Software\\Dev\\NodeJS_v22.exe',
          parentPath: 'E:\\Software\\Dev',
          bytes: 145 * 1024 * 1024,
          modifiedAtMs: Date.now() - 60 * 86400000,
        },
        {
          name: 'NodeJS_v22_copy.exe',
          path: 'C:\\Users\\26502\\Desktop\\NodeJS_v22_copy.exe',
          parentPath: 'C:\\Users\\26502\\Desktop',
          bytes: 145 * 1024 * 1024,
          modifiedAtMs: Date.now() - 10 * 86400000,
        },
        {
          name: 'NodeJS_v22 (2).exe',
          path: 'C:\\Users\\26502\\Downloads\\NodeJS_v22 (2).exe',
          parentPath: 'C:\\Users\\26502\\Downloads',
          bytes: 145 * 1024 * 1024,
          modifiedAtMs: Date.now() - 2 * 86400000,
        },
      ],
    },
    {
      id: 'grp-3',
      hash: 'hash-archive-3',
      kind: 'file',
      bytesPerFile: 520 * 1024 * 1024,
      fileCountPerEntry: 1,
      reclaimableBytes: 520 * 1024 * 1024,
      entries: [
        {
          name: 'Dataset_v1.zip',
          path: 'E:\\Datasets\\Dataset_v1.zip',
          parentPath: 'E:\\Datasets',
          bytes: 520 * 1024 * 1024,
          modifiedAtMs: Date.now() - 45 * 86400000,
        },
        {
          name: 'Dataset_v1_backup.zip',
          path: 'E:\\Backup\\Temp\\Dataset_v1_backup.zip',
          parentPath: 'E:\\Backup\\Temp',
          bytes: 520 * 1024 * 1024,
          modifiedAtMs: Date.now() - 20 * 86400000,
        },
      ],
    },
  ];

  const mockCleanupRules = [
    {
      ruleId: 'system.windows-temp',
      category: 'system',
      group: 'system',
      risk: 'safe',
      defaultSelected: true,
      recommendedSelected: true,
      bytes: 3.8 * 1024 * 1024 * 1024,
      fileCount: 1420,
      available: true,
      selectable: true,
      status: 'ready',
      runningProcesses: [],
      requiresAppClose: false,
      sources: [
        {
          path: 'C:\\Windows\\Temp',
          bytes: 3.8 * 1024 * 1024 * 1024,
          fileCount: 1420,
          selectable: true,
          defaultSelected: true,
        },
      ],
      sourceCount: 1,
      sourcesTruncated: false,
      scanElapsedMs: 120,
    },
    {
      ruleId: 'browser.chrome-cache',
      category: 'browser',
      group: 'browser',
      risk: 'safe',
      defaultSelected: true,
      recommendedSelected: true,
      bytes: 1.9 * 1024 * 1024 * 1024,
      fileCount: 3820,
      available: true,
      selectable: true,
      status: 'ready',
      runningProcesses: [],
      requiresAppClose: false,
      sources: [
        {
          path: 'C:\\Users\\26502\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Cache',
          bytes: 1.9 * 1024 * 1024 * 1024,
          fileCount: 3820,
          selectable: true,
          defaultSelected: true,
        },
      ],
      sourceCount: 1,
      sourcesTruncated: false,
      scanElapsedMs: 80,
    },
    {
      ruleId: 'development.npm-cache',
      category: 'development',
      group: 'development',
      risk: 'safe',
      defaultSelected: true,
      recommendedSelected: true,
      bytes: 4.2 * 1024 * 1024 * 1024,
      fileCount: 8900,
      available: true,
      selectable: true,
      status: 'ready',
      runningProcesses: [],
      requiresAppClose: false,
      sources: [
        {
          path: 'C:\\Users\\26502\\AppData\\Local\\npm-cache',
          bytes: 4.2 * 1024 * 1024 * 1024,
          fileCount: 8900,
          selectable: true,
          defaultSelected: true,
        },
      ],
      sourceCount: 1,
      sourcesTruncated: false,
      scanElapsedMs: 150,
    },
    {
      ruleId: 'special.docker-build-cache',
      category: 'container',
      group: 'container',
      risk: 'moderate',
      defaultSelected: false,
      recommendedSelected: false,
      bytes: 12.4 * 1024 * 1024 * 1024,
      fileCount: 420,
      available: true,
      selectable: true,
      status: 'ready',
      runningProcesses: [],
      requiresAppClose: false,
      sources: [
        {
          path: 'C:\\ProgramData\\Docker\\build-cache',
          bytes: 12.4 * 1024 * 1024 * 1024,
          fileCount: 420,
          selectable: true,
          defaultSelected: false,
        },
      ],
      sourceCount: 1,
      sourcesTruncated: false,
      scanElapsedMs: 200,
    },
  ];

  const mockAppUninstallCandidates = [
    {
      applicationId: 'app-vscode',
      primaryIdentifier: 'Visual Studio Code',
      sourceIdentities: [],
      name: 'Visual Studio Code',
      version: '1.96.2',
      publisher: 'Microsoft Corporation',
      estimatedBytes: 850 * 1024 * 1024,
      lastUsedAtMs: Date.now() - 1000,
      installedAtMs: Date.now() - 180 * 86400000,
      platform: 'windows',
      installerKind: 'system',
      executionMode: 'native',
      capability: 'uninstallable',
      recordState: 'installed',
      applicationPath: 'C:\\Program Files\\Microsoft VS Code\\Code.exe',
      possibleRelatedPaths: [],
      iconPath: null,
      runningProcesses: [],
      totalBytes: 850 * 1024 * 1024,
      defaultSelectedBytes: 850 * 1024 * 1024,
      associatedDataComplete: true,
      components: [],
    },
    {
      applicationId: 'app-baidu-netdisk',
      primaryIdentifier: 'BaiduNetdisk',
      sourceIdentities: [],
      name: '百度网盘 (Baidu Netdisk)',
      version: '7.42.1',
      publisher: 'Baidu, Inc.',
      estimatedBytes: 1.2 * 1024 * 1024 * 1024,
      lastUsedAtMs: Date.now() - 60 * 86400000,
      installedAtMs: Date.now() - 90 * 86400000,
      platform: 'windows',
      installerKind: 'system',
      executionMode: 'native',
      capability: 'uninstallable',
      recordState: 'installed',
      applicationPath: 'C:\\Program Files (x86)\\BaiduNetdisk\\BaiduNetdisk.exe',
      possibleRelatedPaths: [],
      iconPath: null,
      runningProcesses: [],
      totalBytes: 1.2 * 1024 * 1024 * 1024,
      defaultSelectedBytes: 1.2 * 1024 * 1024,
      associatedDataComplete: true,
      components: [],
    },
    {
      applicationId: 'app-steam',
      primaryIdentifier: 'Steam',
      sourceIdentities: [],
      name: 'Steam',
      version: '3.1.0',
      publisher: 'Valve Corporation',
      estimatedBytes: 3.4 * 1024 * 1024 * 1024,
      lastUsedAtMs: Date.now() - 2 * 86400000,
      installedAtMs: Date.now() - 365 * 86400000,
      platform: 'windows',
      installerKind: 'system',
      executionMode: 'native',
      capability: 'uninstallable',
      recordState: 'installed',
      applicationPath: 'E:\\SteamLibrary\\steam.exe',
      possibleRelatedPaths: [],
      iconPath: null,
      runningProcesses: [],
      totalBytes: 3.4 * 1024 * 1024 * 1024,
      defaultSelectedBytes: 3.4 * 1024 * 1024,
      associatedDataComplete: true,
      components: [],
    },
    {
      applicationId: 'app-toolbar-helper',
      primaryIdentifier: 'WebSearchToolbar',
      sourceIdentities: [],
      name: 'SearchAssistant Toolbar (流氓推广助手)',
      version: '1.0.0',
      publisher: 'Unknown Adware Co.',
      estimatedBytes: 45 * 1024 * 1024,
      lastUsedAtMs: null,
      installedAtMs: Date.now() - 10 * 86400000,
      platform: 'windows',
      installerKind: 'system',
      executionMode: 'native',
      capability: 'uninstallable',
      recordState: 'installed',
      applicationPath: 'C:\\Program Files\\WebSearchToolbar\\helper.exe',
      possibleRelatedPaths: [],
      iconPath: null,
      runningProcesses: [],
      totalBytes: 45 * 1024 * 1024,
      defaultSelectedBytes: 45 * 1024 * 1024,
      associatedDataComplete: true,
      components: [],
    },
  ];

  const mockStartupArtifacts = [
    {
      artifactId: 'art-steam-client',
      ownerGroupId: 'grp-steam',
      name: 'Steam Client Bootstrapper',
      sourceKind: 'registryRun',
      scope: 'currentUser',
      configuredState: 'enabled',
      runtimeState: 'stopped',
      controlCapability: 'toggleable',
      trustState: 'verified',
      summarySource: 'bundleMetadata',
      summary: 'Valve Steam 客户端开机自动启动项',
      target: {
        kind: 'executable',
        path: 'E:\\SteamLibrary\\steam.exe -silent',
        executableName: 'steam.exe',
        arguments: ['-silent'],
      },
    },
    {
      artifactId: 'art-baidu-updater',
      ownerGroupId: 'grp-baidu',
      name: 'BaiduNetdisk AutoUpdate Service',
      sourceKind: 'scheduledTask',
      scope: 'system',
      configuredState: 'enabled',
      runtimeState: 'running',
      controlCapability: 'toggleable',
      trustState: 'unsigned',
      summarySource: 'taskDescription',
      summary: '百度网盘后台静默自动更新守护进程',
      target: {
        kind: 'executable',
        path: 'C:\\Program Files (x86)\\BaiduNetdisk\\updater.exe /background',
        executableName: 'updater.exe',
        arguments: ['/background'],
      },
    },
    {
      artifactId: 'art-edge-helper',
      ownerGroupId: 'grp-edge',
      name: 'Microsoft Edge Assistant Agent',
      sourceKind: 'registryRun',
      scope: 'currentUser',
      configuredState: 'enabled',
      runtimeState: 'running',
      controlCapability: 'toggleable',
      trustState: 'system',
      summarySource: 'serviceDescription',
      summary: 'Microsoft Edge 启动预热服务',
      target: {
        kind: 'executable',
        path: 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe --no-startup-window',
        executableName: 'msedge.exe',
        arguments: ['--no-startup-window'],
      },
    },
  ];

  const mockStartupGroups = [
    {
      groupId: 'grp-steam',
      name: 'Steam',
      publisher: 'Valve Corporation',
      aggregateConfiguredState: 'allEnabled',
      aggregateControlState: 'allToggleable',
      artifactCount: 1,
      artifacts: [mockStartupArtifacts[0]],
    },
    {
      groupId: 'grp-baidu',
      name: '百度网盘后台服务',
      publisher: 'Baidu, Inc.',
      aggregateConfiguredState: 'allEnabled',
      aggregateControlState: 'allToggleable',
      artifactCount: 1,
      artifacts: [mockStartupArtifacts[1]],
    },
    {
      groupId: 'grp-edge',
      name: 'Microsoft Edge',
      publisher: 'Microsoft Corporation',
      aggregateConfiguredState: 'allEnabled',
      aggregateControlState: 'allToggleable',
      artifactCount: 1,
      artifacts: [mockStartupArtifacts[2]],
    },
  ];

  const storeData: Record<string, unknown> = JSON.parse(localStorage.getItem('mangodisk_dev_store') || '{}');

  const invokeHandler = async (cmd: string, args: Record<string, unknown> = {}): Promise<unknown> => {
    console.log('[DevMock IPC]', cmd, args);

    if (cmd === 'get_system_disk') return mockDisks[0];
    if (cmd === 'list_disks') return mockDisks;

    // 1. Large Files
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

    // 2. Duplicate Files
    if (cmd === 'find_duplicate_files') {
      const totalReclaimable = mockDuplicateGroups.reduce((acc, g) => acc + g.reclaimableBytes, 0);
      const totalBytes = mockDuplicateGroups.reduce((acc, g) => acc + g.bytesPerFile * g.entries.length, 0);
      const duplicateCount = mockDuplicateGroups.reduce((acc, g) => acc + g.entries.length, 0);
      return {
        scanId: 2001,
        roots: (args.roots as string[]) || ['C:\\'],
        scannedAtMs: Date.now(),
        scannedFileCount: 12500,
        skippedCount: 12,
        duplicateFileCount: duplicateCount,
        totalDuplicateBytes: totalBytes,
        reclaimableBytes: totalReclaimable,
        totalGroupCount: mockDuplicateGroups.length,
        returnedGroupCount: mockDuplicateGroups.length,
        truncated: false,
        groups: mockDuplicateGroups,
      };
    }
    if (cmd === 'get_duplicate_file_groups') {
      return {
        scanId: 2001,
        offset: 0,
        limit: 40,
        totalCount: mockDuplicateGroups.length,
        groups: mockDuplicateGroups,
      };
    }

    // 3. Deep Cleanup
    if (cmd === 'scan_cleanup_candidates') {
      const safeBytes = mockCleanupRules
        .filter(r => r.risk === 'safe')
        .reduce((acc, r) => acc + r.bytes, 0);
      const reclaimableBytes = mockCleanupRules.reduce((acc, r) => acc + r.bytes, 0);
      return {
        schemaVersion: '1.0',
        scannedAtMs: Date.now(),
        disk: mockDisks[0],
        rules: mockCleanupRules,
        applicationIcons: [],
        warningCount: 0,
        safeBytes,
        reclaimableBytes,
        applicabilityElapsedMs: 15,
        applicableRuleCount: mockCleanupRules.length,
        filteredRuleCount: 0,
        inventoryApplicationCount: 12,
        inventoryProcessCount: 45,
        elapsedMs: 250,
      };
    }

    // 4. Application Uninstall
    if (cmd === 'scan_application_uninstall_catalog') {
      return {
        schemaVersion: 1,
        scannedAtMs: Date.now(),
        supported: true,
        executionSupported: true,
        inventoryComplete: true,
        catalogRevision: 'rev-101',
        candidates: mockAppUninstallCandidates,
        readyCount: mockAppUninstallCandidates.length,
        blockedCount: 0,
        hiddenCount: 0,
        relatedDirectoryCount: 8,
        relatedPathScanElapsedMs: 45,
        elapsedMs: 310,
      };
    }
    if (cmd === 'scan_application_leftovers') {
      return {
        schemaVersion: 1,
        scannedAtMs: Date.now(),
        supported: true,
        inventoryComplete: true,
        accessLimited: false,
        candidates: [],
        totalBytes: 0,
        totalFileCount: 0,
        skippedCount: 0,
        elapsedMs: 30,
      };
    }

    // 5. Startup Catalog
    if (cmd === 'scan_startup_catalog') {
      return {
        schemaVersion: 1,
        scanId: 'startup-scan-1',
        catalogRevision: 'rev-201',
        scannedAtMs: Date.now(),
        complete: true,
        artifacts: mockStartupArtifacts,
        groups: mockStartupGroups,
        coverage: [],
        summary: {
          totalArtifactCount: mockStartupArtifacts.length,
          enabledArtifactCount: mockStartupArtifacts.length,
          disabledArtifactCount: 0,
        },
        elapsedMs: 180,
      };
    }

    // 6. Cancellations & General Actions
    if (
      cmd === 'cancel_cleanup_scan' ||
      cmd === 'cancel_duplicate_files' ||
      cmd === 'cancel_large_files' ||
      cmd === 'cancel_application_uninstall_catalog_scan' ||
      cmd === 'cancel_startup_catalog_scan'
    ) {
      return null;
    }

    if (cmd === 'get_application_icons' || cmd === 'get_file_icons') {
      return {};
    }

    if (cmd === 'list_history') return [];

    // Storage plugin
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

    // OS Plugin
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
