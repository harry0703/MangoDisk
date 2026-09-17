import { platform, type Platform } from '@tauri-apps/plugin-os';

/**
 * Owns frontend platform detection so UI adapters do not infer the operating
 * system from mutable browser user-agent strings.
 */
export class OperatingSystemService {
  static currentPlatform(): Platform {
    return platform();
  }

  static isMacOs(): boolean {
    return this.currentPlatform() === 'macos';
  }

  static isWindows(): boolean {
    return this.currentPlatform() === 'windows';
  }

  static isLinux(): boolean {
    return this.currentPlatform() === 'linux';
  }
}
