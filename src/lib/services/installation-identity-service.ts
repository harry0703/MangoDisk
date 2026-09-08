import { load } from '@tauri-apps/plugin-store';

const INSTALLATION_STORE_FILE_NAME = 'installation.json';
const INSTALLATION_IDENTITY_KEY = 'identity';
const INSTALLATION_IDENTITY_SCHEMA_VERSION = 1;
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

interface InstallationIdentity {
  schemaVersion: typeof INSTALLATION_IDENTITY_SCHEMA_VERSION;
  installId: string;
}

/**
 * Owns the random identifier used to count one MangoDisk installation.
 *
 * The identifier is deliberately independent from user preferences so
 * resetting scan settings cannot silently create a second installation. It is
 * random application data and is never derived from hardware or account data.
 */
export class InstallationIdentityService {
  private static pending: Promise<string> | null = null;

  static getOrCreateInstallId(): Promise<string> {
    // Update telemetry and AI panels can initialize concurrently. Share only
    // the pending write, so a failed save can be retried normally.
    this.pending ??= this.readOrCreate().finally(() => {
      this.pending = null;
    });
    return this.pending;
  }

  private static async readOrCreate(): Promise<string> {
    const store = await load(INSTALLATION_STORE_FILE_NAME, { autoSave: false });
    const stored = await store.get<unknown>(INSTALLATION_IDENTITY_KEY);
    if (this.isIdentity(stored)) return stored.installId.toLowerCase();

    const identity: InstallationIdentity = {
      schemaVersion: INSTALLATION_IDENTITY_SCHEMA_VERSION,
      installId: crypto.randomUUID(),
    };
    await store.set(INSTALLATION_IDENTITY_KEY, identity);
    await store.save();
    return identity.installId;
  }

  private static isIdentity(value: unknown): value is InstallationIdentity {
    if (!value || typeof value !== 'object') return false;
    const candidate = value as Partial<InstallationIdentity>;
    return (
      candidate.schemaVersion === INSTALLATION_IDENTITY_SCHEMA_VERSION &&
      typeof candidate.installId === 'string' &&
      UUID_PATTERN.test(candidate.installId)
    );
  }
}
