export const APP_UPDATE_STATUS_IDS = {
  idle: 'idle',
  checking: 'checking',
  upToDate: 'upToDate',
  available: 'available',
  downloading: 'downloading',
  downloaded: 'downloaded',
  installing: 'installing',
  restartRequired: 'restartRequired',
  restarting: 'restarting',
  error: 'error',
} as const;

export type AppUpdateStatus = (typeof APP_UPDATE_STATUS_IDS)[keyof typeof APP_UPDATE_STATUS_IDS];

export const APP_DISTRIBUTION_IDS = {
  installed: 'installed',
  portable: 'portable',
} as const;

export type AppDistribution = (typeof APP_DISTRIBUTION_IDS)[keyof typeof APP_DISTRIBUTION_IDS];

export const APP_UPDATE_ACTION_IDS = {
  automaticInstall: 'automaticInstall',
  manualDownload: 'manualDownload',
} as const;

export type AppUpdateAction = (typeof APP_UPDATE_ACTION_IDS)[keyof typeof APP_UPDATE_ACTION_IDS];

export const APP_UPDATE_FAILURE_STAGE_IDS = {
  download: 'download',
  install: 'install',
  restart: 'restart',
} as const;

export type AppUpdateFailureStage = (typeof APP_UPDATE_FAILURE_STAGE_IDS)[keyof typeof APP_UPDATE_FAILURE_STAGE_IDS];

export interface AppUpdateInfo {
  currentVersion: string;
  version: string;
  date?: string;
  notes: string;
  action: AppUpdateAction;
  manualDownloadUrl?: string;
}

export interface AppUpdateDownloadProgress {
  downloadedBytes: number;
  totalBytes: number | null;
  finished: boolean;
}

export const APP_UPDATE_DOWNLOAD_TIMEOUT_MS = 5 * 60_000;
export interface AppUpdateNotice {
  schemaVersion: 1;
  revision: number;
  version: string | null;
  checked: boolean;
}
