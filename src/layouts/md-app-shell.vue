<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';

import { APP_UPDATE_AUTOMATIC_CHECK_DELAY_MS, APP_UPDATE_STATUS_IDS } from '@/lib/models/app-update';
import type { ApplicationLeftoverCandidate, ApplicationUninstallBatchSelection } from '@/lib/models/application';
import type { ApplicationCloseMode } from '@/lib/models/application-close';
import type { DirectoryEntryInfo } from '@/lib/models/analysis';
import type { DuplicateFileEntry } from '@/lib/models/duplicate-file';
import type { LargeFileEntry, LargeFileScanMode } from '@/lib/models/large-file';
import { CLEANUP_OPERATION_IDS, CLEANUP_SCAN_SCOPE_MODES, type CleanupScanScope } from '@/lib/models/cleanup';
import {
  createSidebarLayoutState,
  PAGE_IDS,
  resizeSidebarLayout,
  toggleSidebarLayout,
} from '@/lib/models/application-shell';
import type { AppSettings } from '@/lib/models/settings';
import type { PageId } from '@/lib/models/application-shell';
import { ApplicationMenuService } from '@/lib/services/application-menu-service';
import { FileManagerService } from '@/lib/services/file-manager-service';
import { LinkService } from '@/lib/services/link-service';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import * as CleanupRuleTextUtils from '@/lib/utils/cleanup-rule-text';
import { type CleanupRuleMessageResolver } from '@/lib/utils/cleanup-rule-text';
import * as CleanupScanScopeUtils from '@/lib/utils/cleanup-scan-scope';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import * as FormatUtils from '@/lib/utils/format';
import { useAnalysisStore } from '@/stores/analysis-store';
import { useApplicationStore } from '@/stores/application-store';
import { useAppUpdateStore } from '@/stores/app-update-store';
import { useAppStore } from '@/stores/app-store';
import { useCleanupStore } from '@/stores/cleanup-store';
import { useDuplicateFilesStore } from '@/stores/duplicate-files-store';
import { useHistoryStore } from '@/stores/history-store';
import { useLargeFilesStore } from '@/stores/large-files-store';
import { usePrivacyStore } from '@/stores/privacy-store';
import { useStorageScopeStore } from '@/stores/storage-scope-store';
import { useStartupStore } from '@/stores/startup-store';
import { useSystemSettingsStore } from '@/stores/system-settings-store';
import { useSystemMaintenanceStore } from '@/stores/system-maintenance-store';

import CleanupPage from '@/pages/cleanup/index.vue';

import MdSidebar from './components/md-sidebar.vue';
import MdCleanupOperationOverlay from './components/md-cleanup-operation-overlay.vue';
import MdGlobalErrorFeedback from './components/md-global-error-feedback.vue';
import MdPrivacyOperationOverlay from './components/md-privacy-operation-overlay.vue';
import MdWindowTitlebar from './components/md-window-titlebar.vue';

// Cleanup is the startup page. Secondary pages remain separate chunks, while
// idle preloading and guarded navigation prevent their first render from
// replacing the current page with an empty async-component placeholder.
const loadAnalysisPage = () => import('@/pages/analysis/index.vue');
const loadApplicationUninstallPage = () => import('@/pages/application-uninstall/index.vue');
const loadDuplicateFilesPage = () => import('@/pages/duplicate-files/index.vue');
const loadHistoryPage = () => import('@/pages/history/index.vue');
const loadLargeFilesPage = () => import('@/pages/large-files/index.vue');
const loadPrivacyPage = () => import('@/pages/privacy/index.vue');
const loadSettingsPage = () => import('@/pages/settings/index.vue');
const loadStartupPage = () => import('@/pages/startup/index.vue');
const loadSystemOptimizationPage = () => import('@/pages/system-optimization/index.vue');
const loadSystemMaintenancePage = () => import('@/pages/system-maintenance/index.vue');
const pageLoaders: Partial<Record<PageId, () => Promise<unknown>>> = {
  [PAGE_IDS.analysis]: loadAnalysisPage,
  [PAGE_IDS.applicationUninstall]: loadApplicationUninstallPage,
  [PAGE_IDS.duplicateFiles]: loadDuplicateFilesPage,
  [PAGE_IDS.history]: loadHistoryPage,
  [PAGE_IDS.largeFiles]: loadLargeFilesPage,
  [PAGE_IDS.privacy]: loadPrivacyPage,
  [PAGE_IDS.settings]: loadSettingsPage,
  [PAGE_IDS.startup]: loadStartupPage,
  [PAGE_IDS.systemOptimization]: loadSystemOptimizationPage,
  [PAGE_IDS.systemMaintenance]: loadSystemMaintenancePage,
};
const AnalysisPage = defineAsyncComponent(loadAnalysisPage);
const ApplicationUninstallPage = defineAsyncComponent(loadApplicationUninstallPage);
const DuplicateFilesPage = defineAsyncComponent(loadDuplicateFilesPage);
const HistoryPage = defineAsyncComponent(loadHistoryPage);
const LargeFilesPage = defineAsyncComponent(loadLargeFilesPage);
const PrivacyPage = defineAsyncComponent(loadPrivacyPage);
const SettingsPage = defineAsyncComponent(loadSettingsPage);
const StartupPage = defineAsyncComponent(loadStartupPage);
const SystemOptimizationPage = defineAsyncComponent(loadSystemOptimizationPage);
const SystemMaintenancePage = defineAsyncComponent(loadSystemMaintenancePage);
const MdAboutDialog = defineAsyncComponent(() => import('./components/md-about-dialog.vue'));

const { rt, t, te, tm } = useI18n({ useScope: 'global' });

const CLEANUP_RULE_ENTRY_KEY = /^cleanupRules\.entries\.(.+)\.(name|description|impact)$/u;
type CleanupRuleEntry = Partial<Record<'description' | 'impact' | 'name', Parameters<typeof rt>[0]>>;

// Rule IDs contain dots, so resolve them as exact keys inside the entries
// object rather than allowing vue-i18n to interpret them as nested paths.
const resolveCleanupRuleMessage: CleanupRuleMessageResolver = (key, parameters) => {
  const entryMatch = CLEANUP_RULE_ENTRY_KEY.exec(key);
  if (entryMatch) {
    const entries = tm('cleanupRules.entries') as Record<string, CleanupRuleEntry>;
    const message = entries[entryMatch[1]]?.[entryMatch[2] as keyof CleanupRuleEntry];
    return message === undefined ? undefined : rt(message);
  }
  return te(key) ? t(key, parameters ?? {}) : undefined;
};

const store = useAppStore();
const appUpdateStore = useAppUpdateStore();
const cleanupStore = useCleanupStore();
const systemMaintenanceStore = useSystemMaintenanceStore();
const analysisStore = useAnalysisStore();
const applicationStore = useApplicationStore();
const cleanupOrchestrating = ref(false);
const deepCleanupCancelling = ref(false);
const cleanupCancellationRetried = ref(false);
const settingsFocusRevision = ref(0);
const historyStore = useHistoryStore();
const largeFilesStore = useLargeFilesStore();
const privacyStore = usePrivacyStore();
const duplicateFilesStore = useDuplicateFilesStore();
const storageScopeStore = useStorageScopeStore();
const startupStore = useStartupStore();
const systemSettingsStore = useSystemSettingsStore();
// WebKit can leave range-based media-query utilities in their collapsed state
// after a native window is narrowed and widened again. The explicit state also
// keeps a user's toggle separate from the responsive window-width decision.
const sidebarLayout = ref(createSidebarLayoutState(window.innerWidth));
const sidebarExpanded = computed(() => sidebarLayout.value.expanded);
const UPDATE_CHECK_ERROR_TOAST_ID = 'app-update-check-error';
const LARGE_FILE_DELETE_TOAST_ID = 'large-file-delete-result';
const DUPLICATE_FILE_DELETE_TOAST_ID = 'duplicate-file-delete-result';
const DEEP_CLEANUP_TOAST_ID = 'deep-cleanup-result';
const customCleanupRuleNames = computed<Record<string, string>>(() =>
  cleanupStore.scanScope.mode === CLEANUP_SCAN_SCOPE_MODES.custom
    ? Object.fromEntries(cleanupStore.scanScope.rules.map(rule => [`custom.${rule.id}`, rule.name]))
    : {}
);
const localizedCleanupScan = computed(() =>
  cleanupStore.scan
    ? CleanupRuleTextUtils.snapshot(cleanupStore.scan, resolveCleanupRuleMessage, customCleanupRuleNames.value)
    : null
);
const localizedCleanupResult = computed(() =>
  cleanupStore.result
    ? CleanupRuleTextUtils.cleanupResult(cleanupStore.result, resolveCleanupRuleMessage, customCleanupRuleNames.value)
    : null
);
const localizedHistory = computed(() => CleanupRuleTextUtils.records(historyStore.records, resolveCleanupRuleMessage));
const cleanupBusy = computed(
  () =>
    cleanupOrchestrating.value ||
    cleanupStore.loading ||
    cleanupStore.closingApplications ||
    applicationStore.scanningLeftovers ||
    applicationStore.deletingLeftovers
);
// Custom title bars keep the application chrome visually continuous. macOS
// only needs a drag region beneath the native traffic lights, while Windows
// renders explicit controls because its native decorations are disabled.
const currentPlatform = OperatingSystemService.currentPlatform();
const isMacOs = currentPlatform === 'macos';
const isWindows = currentPlatform === 'windows';
const customTitlebarPlatform = computed<'macos' | 'windows' | null>(() => {
  if (isMacOs) return 'macos';
  if (isWindows) return 'windows';
  return null;
});
const cleanupLoadingMessage = computed(() => {
  if (cleanupStore.operation === CLEANUP_OPERATION_IDS.cancelling) return t('loading.cancelling');
  if (cleanupStore.operation === CLEANUP_OPERATION_IDS.previewing) return t('loading.previewing');
  if (cleanupStore.operation === CLEANUP_OPERATION_IDS.cleaning) return t('loading.cleaning');
  return t('loading.scanning');
});
const destructiveCleanupActive = computed(
  () =>
    (cleanupStore.loading && cleanupStore.operation === CLEANUP_OPERATION_IDS.cleaning) ||
    applicationStore.deletingLeftovers
);
watch(
  () => cleanupStore.executionProgress,
  progress => {
    if (!progress || !deepCleanupCancelling.value || cleanupCancellationRetried.value) return;
    // The execution listener is registered before Core starts. A very fast click
    // can therefore send cancellation while no guard exists yet. The first
    // progress event proves that the guard is active and safely retries the
    // idempotent request before cleanup leaves validation.
    cleanupCancellationRetried.value = true;
    void cleanupStore.cancelExecution().catch(() => undefined);
  }
);

async function openExternalLink(url: string) {
  try {
    await LinkService.open(url);
  } catch (error) {
    store.reportError(error);
  }
}
const busyPages = computed<PageId[]>(() => [
  ...(cleanupBusy.value ? [PAGE_IDS.cleanup] : []),
  ...(analysisStore.pending || analysisStore.deleting ? [PAGE_IDS.analysis] : []),
  ...(largeFilesStore.loading || largeFilesStore.deleting ? [PAGE_IDS.largeFiles] : []),
  ...(duplicateFilesStore.loading || duplicateFilesStore.deleting ? [PAGE_IDS.duplicateFiles] : []),
  ...(applicationStore.scanningUninstallCatalog ||
  applicationStore.preparingUninstall ||
  applicationStore.executingUninstall
    ? [PAGE_IDS.applicationUninstall]
    : []),
  ...(privacyStore.scanning || privacyStore.preparing || privacyStore.executing ? [PAGE_IDS.privacy] : []),
  ...(startupStore.scanning || startupStore.preparingChange || startupStore.executingChange ? [PAGE_IDS.startup] : []),
  ...(systemSettingsStore.scanning || systemSettingsStore.preparing || systemSettingsStore.executing
    ? [PAGE_IDS.systemOptimization]
    : []),
  ...(systemMaintenanceStore.scanning || systemMaintenanceStore.executing ? [PAGE_IDS.systemMaintenance] : []),
  ...(historyStore.loading ? [PAGE_IDS.history] : []),
]);
const noticePages = computed<PageId[]>(() => (appUpdateStore.updateNoticeUnread ? [PAGE_IDS.settings] : []));

let navigationRequest = 0;
let diskInitialization: Promise<void> | null = null;
let historyInitialization: Promise<void> | null = null;
let unlistenOpenAbout: (() => void) | null = null;
let shellMounted = true;
let automaticUpdateTimer: ReturnType<typeof setTimeout> | null = null;

function initializeDisks(): Promise<void> {
  diskInitialization ??= store.initialize().then(() => storageScopeStore.initialize(store.disks));
  return diskInitialization;
}

function initializePageData(page: PageId): Promise<void> {
  if (page === PAGE_IDS.history) {
    historyInitialization ??= historyStore.load();
    return historyInitialization;
  }
  if (page === PAGE_IDS.analysis || page === PAGE_IDS.largeFiles || page === PAGE_IDS.duplicateFiles) {
    return initializeDisks();
  }
  return Promise.resolve();
}

function preloadFeaturePages() {
  const preload = () => {
    void Promise.allSettled(Object.values(pageLoaders).map(loadPage => loadPage()));
    // Disk inventory is useful to two feature pages but is not required to
    // render the startup cleanup page. Begin it only after the first frame is
    // interactive, while guarded navigation still waits if users arrive first.
    void initializeDisks();
  };
  const requestIdleCallback = (window as Window & { requestIdleCallback?: Window['requestIdleCallback'] })
    .requestIdleCallback;
  if (requestIdleCallback) {
    requestIdleCallback.call(window, preload, { timeout: 1200 });
  } else {
    globalThis.setTimeout(preload, 200);
  }
}

function syncSidebarExpansion() {
  sidebarLayout.value = resizeSidebarLayout(sidebarLayout.value, window.innerWidth);
}

function toggleSidebar() {
  sidebarLayout.value = toggleSidebarLayout(sidebarLayout.value);
}

onMounted(() => {
  window.addEventListener('resize', syncSidebarExpansion);
  syncSidebarExpansion();
  cleanupStore.initialize();
  preloadFeaturePages();
  void appUpdateStore.initialize();
  // Update checks start after the first interactive frame and never delay
  // cleanup initialization or navigation. Development builds use the same
  // path so local update endpoints and the complete startup interaction can
  // be verified before packaging.
  automaticUpdateTimer = window.setTimeout(() => {
    automaticUpdateTimer = null;
    void appUpdateStore.check(store.settings.language, false);
  }, APP_UPDATE_AUTOMATIC_CHECK_DELAY_MS);
  void ApplicationMenuService.onOpenAbout(() => {
    void openAboutSettings();
  })
    .then(unlisten => {
      if (shellMounted) {
        unlistenOpenAbout = unlisten;
      } else {
        unlisten();
      }
    })
    .catch(error => store.reportError(error));
});

onBeforeUnmount(() => {
  shellMounted = false;
  window.removeEventListener('resize', syncSidebarExpansion);
  if (automaticUpdateTimer) window.clearTimeout(automaticUpdateTimer);
  unlistenOpenAbout?.();
});

async function navigate(page: PageId) {
  const request = ++navigationRequest;
  try {
    await Promise.all([pageLoaders[page]?.(), initializePageData(page)]);
    if (request === navigationRequest) {
      store.navigate(page);
      if (page === PAGE_IDS.settings && appUpdateStore.updateNoticeUnread) {
        settingsFocusRevision.value += 1;
      }
    }
  } catch (error) {
    store.reportError(error);
  }
}

async function openAboutSettings() {
  await navigate(PAGE_IDS.settings);
  settingsFocusRevision.value += 1;
  appUpdateStore.showAbout();
}

async function checkForUpdates() {
  await appUpdateStore.check(store.settings.language, true);
  if (appUpdateStore.status !== APP_UPDATE_STATUS_IDS.error) return;
  toast.error(t('settings.updateCheckFailedTitle'), {
    description: appUpdateStore.checkError || t('settings.updateCheckUnknownError'),
    id: UPDATE_CHECK_ERROR_TOAST_ID,
  });
}

function saveSettings(settings: AppSettings) {
  store.saveSettings(settings);
}

function scanStartupCatalog() {
  if (startupStore.scanning) return;
  return startupStore.scan();
}

async function executeStartupChange() {
  await startupStore.executeChange(t('startup.change.authorizationPromptMacos'));
  if (startupStore.lastChangeResult) await historyStore.load({ reportError: false });
}

async function clearHistoryData() {
  await historyStore.clear();
}

function analyze(path?: string, refresh = false, setHome = false) {
  // A rapid second navigation can arrive before Vue propagates the Store's
  // pending state back into the page props. Ignore that same-domain request
  // instead of submitting duplicate work to Core.
  if (analysisStore.pending || analysisStore.deleting) return;
  return analysisStore.analyze(path, refresh, setHome);
}

function deleteAnalysisEntryPermanently(entry: DirectoryEntryInfo) {
  return analysisStore.deletePermanently(entry);
}

function findLargeFiles(path: string | undefined, scanMode: LargeFileScanMode) {
  return largeFilesStore.find(path, store.settings.largeFileMinimumBytes, scanMode);
}

function updateLargeFileMinimum(minimumBytes: number) {
  if (minimumBytes === store.settings.largeFileMinimumBytes) return;
  saveSettings({ ...store.settings, largeFileMinimumBytes: minimumBytes });
  void largeFilesStore.filter(minimumBytes);
}

async function deleteLargeFilesPermanently(entries: LargeFileEntry[]) {
  const result = await largeFilesStore.deleteManyPermanently(entries);
  if (!result) return;
  const description = t(
    'largeFiles.deleteCompletedDescription',
    {
      count: FormatUtils.integer(result.removedPaths.length),
      size: ByteSizeService.bytes(result.releasedBytes),
      failed: FormatUtils.integer(result.failed.length),
    },
    result.removedPaths.length
  );
  const options = { description, id: LARGE_FILE_DELETE_TOAST_ID };
  if (result.failed.length) toast.warning(t('largeFiles.deleteCompletedWithWarnings'), options);
  else toast.success(t('largeFiles.deleteCompleted'), options);
}

function findDuplicateFiles(path: string) {
  return duplicateFilesStore.find([path], store.settings.duplicateFileMinimumBytes);
}

function updateDuplicateFileMinimum(minimumBytes: number) {
  if (minimumBytes === store.settings.duplicateFileMinimumBytes) return;
  saveSettings({ ...store.settings, duplicateFileMinimumBytes: minimumBytes });
}

function updateDuplicateKeeperRule(keeperRule: AppSettings['duplicateKeeperRule']) {
  if (keeperRule === store.settings.duplicateKeeperRule) return;
  saveSettings({ ...store.settings, duplicateKeeperRule: keeperRule });
}

async function deleteDuplicateFilesPermanently(entries: DuplicateFileEntry[]) {
  const result = await duplicateFilesStore.deletePermanently(entries);
  if (!result) return;
  const description = t(
    'duplicateFiles.deleteCompletedDescription',
    {
      count: FormatUtils.integer(result.removedPaths.length),
      size: ByteSizeService.bytes(result.releasedBytes),
      failed: FormatUtils.integer(result.failed.length),
    },
    result.removedPaths.length
  );
  const options = { description, id: DUPLICATE_FILE_DELETE_TOAST_ID };
  if (result.failed.length) toast.warning(t('duplicateFiles.deleteCompletedWithWarnings'), options);
  else toast.success(t('duplicateFiles.deleteCompleted'), options);
}

function scanApplications() {
  return applicationStore.scanUninstallCatalog();
}

function prepareApplicationUninstall(selections: ApplicationUninstallBatchSelection[]) {
  return applicationStore.prepareUninstall(selections);
}

function closeApplicationsBeforeCleanup(ruleIds: string[], mode: ApplicationCloseMode) {
  return cleanupStore.closeApplications(ruleIds, mode);
}

function closeApplicationsBeforeUninstall(applicationIds: string[], mode: ApplicationCloseMode) {
  return applicationStore.closeUninstallApplications(applicationIds, mode);
}

function executeApplicationUninstall() {
  return applicationStore.executePreparedUninstall(t('applicationUninstall.authorizationPromptMacos'));
}

async function openPath(path: string) {
  await executeFileManagerAction(() => FileManagerService.reveal(path));
}

async function executeFileManagerAction(action: () => Promise<void>) {
  try {
    await action();
  } catch (error) {
    store.reportError(error);
  }
}

async function openAnalysisEntry(scanId: number, path: string) {
  await executeFileManagerAction(() => FileManagerService.openAnalysisEntry(scanId, path));
}

async function openLargeFileEntry(scanId: number, path: string) {
  await executeFileManagerAction(() => FileManagerService.openLargeFileEntry(scanId, path));
}

async function openDuplicateFileEntry(scanId: number, path: string) {
  await executeFileManagerAction(() => FileManagerService.openDuplicateFileEntry(scanId, path));
}

async function scanCleanup(scanScope: CleanupScanScope) {
  cleanupOrchestrating.value = true;
  try {
    const completed = await cleanupStore.scanCandidates(scanScope);
    if (!completed) return;
    if (CleanupScanScopeUtils.includesStandardCleanup(scanScope)) {
      await applicationStore.scanLeftovers();
    } else {
      applicationStore.clearLeftoverResults();
    }
  } finally {
    cleanupOrchestrating.value = false;
  }
}

async function executeCleanup(leftovers: ApplicationLeftoverCandidate[]) {
  cleanupOrchestrating.value = true;
  deepCleanupCancelling.value = false;
  cleanupCancellationRetried.value = false;
  const deepCleanupOperationId = crypto.randomUUID();
  const executesCleanupRules = cleanupStore.selectedRuleIds.length > 0;
  try {
    if (executesCleanupRules) {
      const completed = await cleanupStore.execute(false, deepCleanupOperationId);
      // Do not clear the cleanup error by starting a second operation after a
      // fatal failure. Ordinary partial results may continue, while an explicit
      // user cancellation stops the complete deep-cleanup workflow.
      if (!completed || deepCleanupCancelling.value) return;
    }
    if (leftovers.length && !deepCleanupCancelling.value) {
      await applicationStore.deleteLeftoversPermanently(leftovers, deepCleanupOperationId);
      if (!applicationStore.lastResult || deepCleanupCancelling.value) return;
    }
    const cleanupResult = executesCleanupRules ? cleanupStore.result : null;
    const leftoverResult = leftovers.length ? applicationStore.lastResult : null;
    const releasedBytes = (cleanupResult?.releasedBytes ?? 0) + (leftoverResult?.releasedBytes ?? 0);
    const affectedItemCount = (cleanupResult?.affectedItemCount ?? 0) + (leftoverResult?.affectedItemCount ?? 0);
    const failedItemCount = (cleanupResult?.failedItemCount ?? 0) + (leftoverResult?.failedItemCount ?? 0);
    const description = t(
      'cleanup.completedDescription',
      {
        count: FormatUtils.integer(affectedItemCount),
        size: ByteSizeService.bytes(releasedBytes),
        failed: FormatUtils.integer(failedItemCount),
      },
      affectedItemCount
    );
    const options = { description, id: DEEP_CLEANUP_TOAST_ID };
    if (failedItemCount) toast.warning(t('cleanup.completedWithWarnings'), options);
    else toast.success(t('cleanup.completed'), options);
  } finally {
    cleanupOrchestrating.value = false;
    deepCleanupCancelling.value = false;
    cleanupCancellationRetried.value = false;
  }
}

async function cancelDeepCleanup() {
  if (deepCleanupCancelling.value || !destructiveCleanupActive.value) return;
  // Set the workflow flag before invoking Core. This closes the short boundary
  // between cache cleanup and leftover cleanup, where neither native operation
  // may be active yet but the second phase must still not start.
  deepCleanupCancelling.value = true;
  const requests: Promise<void>[] = [];
  if (cleanupStore.loading && cleanupStore.operation === CLEANUP_OPERATION_IDS.cleaning) {
    requests.push(cleanupStore.cancelExecution());
  }
  if (applicationStore.deletingLeftovers) {
    requests.push(applicationStore.cancelLeftoverDeletion());
  }
  await Promise.allSettled(requests);
}
</script>

<template>
  <main
    class="app-shell"
    :class="{
      'custom-titlebar': customTitlebarPlatform,
      'macos-overlay': isMacOs,
      'windows-custom-titlebar': isWindows,
      'sidebar-expanded': sidebarExpanded,
    }"
  >
    <MdWindowTitlebar
      v-if="customTitlebarPlatform"
      :platform="customTitlebarPlatform"
      :sidebar-expanded="sidebarExpanded"
    />
    <MdSidebar
      :current-page="store.currentPage"
      :busy-pages="busyPages"
      :notice-pages="noticePages"
      :expanded="sidebarExpanded"
      @navigate="navigate"
      @toggle="toggleSidebar"
    />
    <div class="content-shell">
      <KeepAlive>
        <SystemOptimizationPage v-if="store.currentPage === PAGE_IDS.systemOptimization" />
        <SystemMaintenancePage v-else-if="store.currentPage === PAGE_IDS.systemMaintenance" />
        <CleanupPage
          v-else-if="store.currentPage === PAGE_IDS.cleanup"
          :disk="store.disk"
          :disks="store.disks"
          :scan="localizedCleanupScan"
          :scan-scope="cleanupStore.scanScope"
          :selected-rule-ids="cleanupStore.selectedRuleIds"
          :source-selections="cleanupStore.sourceSelections"
          :selected-bytes="cleanupStore.selectedBytes"
          :result="localizedCleanupResult"
          :leftovers="applicationStore.leftovers"
          :leftover-result="applicationStore.lastResult"
          :scanning-leftovers="applicationStore.scanningLeftovers"
          :deleting-leftovers="applicationStore.deletingLeftovers"
          :progress="cleanupStore.scanProgress"
          :loading-message="cleanupLoadingMessage"
          :operation="cleanupStore.operation"
          :busy="cleanupBusy"
          :closing-applications="cleanupStore.closingApplications"
          :close-result="cleanupStore.applicationCloseResult"
          :privileged-scan-rule-id="cleanupStore.privilegedScanRuleId"
          @scan="scanCleanup"
          @toggle-source="cleanupStore.toggleSource"
          @select-all="cleanupStore.setRulesSelected"
          @execute="executeCleanup"
          @cancel="cleanupStore.cancelScan()"
          @close-applications="closeApplicationsBeforeCleanup"
          @open="openPath"
          @privileged-scan="cleanupStore.scanPreviousInstallationsWithPrivileges()"
        />
        <AnalysisPage
          v-else-if="store.currentPage === PAGE_IDS.analysis"
          :result="analysisStore.result"
          :home-path="analysisStore.homePath"
          :disk="store.disk"
          :disks="store.disks"
          :progress="analysisStore.progress"
          :busy="analysisStore.pending"
          :cancelling="analysisStore.cancelling"
          :deleting="analysisStore.deleting"
          @analyze="analyze"
          @cancel="analysisStore.cancel()"
          @error="store.reportError"
          @open-entry="openAnalysisEntry"
          @reveal="openPath"
          @delete="deleteAnalysisEntryPermanently"
        />
        <LargeFilesPage
          v-else-if="store.currentPage === PAGE_IDS.largeFiles"
          :disk="store.disk"
          :disks="store.disks"
          :result="largeFilesStore.result"
          :progress="largeFilesStore.progress"
          :minimum-bytes="store.settings.largeFileMinimumBytes"
          :busy="largeFilesStore.loading"
          :cancelling="largeFilesStore.cancelling"
          :deleting="largeFilesStore.deleting"
          @find="findLargeFiles"
          @update-minimum="updateLargeFileMinimum"
          @cancel="largeFilesStore.cancel()"
          @error="store.reportError"
          @open-entry="openLargeFileEntry"
          @reveal="openPath"
          @delete-many="deleteLargeFilesPermanently"
        />
        <DuplicateFilesPage
          v-else-if="store.currentPage === PAGE_IDS.duplicateFiles"
          :disk="store.disk"
          :disks="store.disks"
          :result="duplicateFilesStore.result"
          :result-complete="duplicateFilesStore.resultComplete"
          :has-more="duplicateFilesStore.hasMore"
          :loading-more="duplicateFilesStore.loadingMore"
          :progress="duplicateFilesStore.progress"
          :busy="duplicateFilesStore.loading"
          :cancelling="duplicateFilesStore.cancelling"
          :deleting="duplicateFilesStore.deleting"
          :minimum-bytes="store.settings.duplicateFileMinimumBytes"
          :keeper-rule="store.settings.duplicateKeeperRule"
          @find="findDuplicateFiles"
          @update-minimum="updateDuplicateFileMinimum"
          @update-keeper-rule="updateDuplicateKeeperRule"
          @cancel="duplicateFilesStore.cancel()"
          @error="store.reportError"
          @open-entry="openDuplicateFileEntry"
          @reveal="openPath"
          @delete="deleteDuplicateFilesPermanently"
          @load-more="duplicateFilesStore.loadMore"
        />
        <ApplicationUninstallPage
          v-else-if="store.currentPage === PAGE_IDS.applicationUninstall"
          :catalog="applicationStore.uninstallCatalog"
          :scanning="applicationStore.scanningUninstallCatalog"
          :cancelling="applicationStore.cancellingUninstallCatalog"
          :progress="applicationStore.uninstallProgress"
          :execution-progress="applicationStore.uninstallExecutionProgress"
          :plan="applicationStore.uninstallPlan"
          :preview="applicationStore.uninstallPreview"
          :last-result="applicationStore.uninstallLastResult"
          :preparing="applicationStore.preparingUninstall"
          :executing="applicationStore.executingUninstall"
          :cancelling-execution="applicationStore.cancellingUninstall"
          :cancellation-revision="applicationStore.uninstallCancellationRevision"
          :closing-applications="applicationStore.closingUninstallApplications"
          :close-result="applicationStore.uninstallCloseResult"
          @scan="scanApplications"
          @record-removed="applicationStore.removeUninstallCatalogRecord"
          @cancel-scan="applicationStore.cancelUninstallCatalogScan()"
          @prepare="prepareApplicationUninstall"
          @cancel-plan="applicationStore.clearPreparedUninstall()"
          @execute="executeApplicationUninstall"
          @cancel-execution="applicationStore.cancelUninstallExecution()"
          @close-applications="closeApplicationsBeforeUninstall"
          @open="openPath"
        />
        <PrivacyPage v-else-if="store.currentPage === PAGE_IDS.privacy" />
        <StartupPage
          v-else-if="store.currentPage === PAGE_IDS.startup"
          :catalog="startupStore.catalog"
          :scanning="startupStore.scanning"
          :cancelling="startupStore.cancelling"
          :preparing-change="startupStore.preparingChange"
          :executing-change="startupStore.executingChange"
          :cancelling-change="startupStore.cancellingChange"
          :pending-plan="startupStore.pendingPlan"
          :last-change-result="startupStore.lastChangeResult"
          @scan="scanStartupCatalog"
          @cancel="startupStore.cancelScan()"
          @prepare-change="startupStore.prepareChange($event.itemIds, $event.desiredState)"
          @cancel-change="startupStore.clearPendingPlan()"
          @cancel-change-execution="startupStore.cancelChange()"
          @execute-change="executeStartupChange"
          @open="openPath"
          @error="store.reportError"
        />
        <HistoryPage
          v-else-if="store.currentPage === PAGE_IDS.history"
          :history="localizedHistory"
          :busy="historyStore.loading"
          @clear="clearHistoryData"
        />
        <SettingsPage
          v-else-if="store.currentPage === PAGE_IDS.settings"
          :settings="store.settings"
          :focus-revision="settingsFocusRevision"
          @save="saveSettings"
          @error="store.reportError"
        />
      </KeepAlive>
    </div>

    <MdGlobalErrorFeedback />
    <MdCleanupOperationOverlay
      :rules="localizedCleanupScan?.rules ?? []"
      :cancelling="deepCleanupCancelling"
      @cancel="cancelDeepCleanup"
    />
    <MdPrivacyOperationOverlay @cancel="privacyStore.cancelExecution()" />

    <MdAboutDialog
      v-if="appUpdateStore.dialogOpen"
      :open="appUpdateStore.dialogOpen"
      :status="appUpdateStore.status"
      :action="appUpdateStore.update?.action ?? null"
      :current-version="appUpdateStore.currentVersion"
      :version="appUpdateStore.update?.version ?? ''"
      :notes="appUpdateStore.update?.notes ?? ''"
      :check-error="appUpdateStore.checkError"
      :downloaded-bytes="appUpdateStore.downloadedBytes"
      :total-bytes="appUpdateStore.totalBytes"
      :action-error="appUpdateStore.actionError"
      :failure-stage="appUpdateStore.failureStage"
      @close="appUpdateStore.dismiss()"
      @check="checkForUpdates"
      @download="appUpdateStore.download()"
      @manual-download="appUpdateStore.openManualDownload()"
      @install="appUpdateStore.installDownloaded()"
      @restart="appUpdateStore.restartApplication()"
      @open-link="openExternalLink"
    />
  </main>
</template>

<style scoped>
@reference "@assets/main.css";
.app-shell {
  --titlebar-height: 0px;
  --window-controls-width: 144px;
  --sidebar-width: var(--layout-sidebar-collapsed-width);
  --sidebar-transition-duration: 240ms;
  --sidebar-transition-easing: cubic-bezier(0.22, 1, 0.36, 1);
  display: flex;
  width: 100%;
  height: 100vh;
  overflow: hidden;
  @apply bg-sidebar text-foreground;
}
.app-shell.sidebar-expanded {
  --sidebar-width: var(--layout-sidebar-expanded-width);
}
.macos-overlay {
  --titlebar-height: 34px;
}
.windows-custom-titlebar {
  --titlebar-height: var(--layout-page-header-height);
}
.custom-titlebar :deep(.sidebar) {
  padding-top: var(--titlebar-height);
}
.content-shell {
  flex: 1;
  min-width: 0;
  height: 100vh;
  overflow: hidden;
  border-radius: 12px 0 0;
  @apply bg-background;
}
</style>
