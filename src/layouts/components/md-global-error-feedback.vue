<script setup lang="ts">
import { computed, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';

import { useAppStore } from '@/stores/app-store';

const APPLICATION_ERROR_TOAST_ID = 'application-error';
const store = useAppStore();
const { t } = useI18n({ useScope: 'global' });

const errorMessage = computed(() => {
  switch (store.errorReason) {
    case 'analysisRefreshFailedAfterDelete':
      return t('errorReasons.analysisRefreshFailedAfterDelete.message');
    case 'directoryNotEmpty':
      return t('errorReasons.directoryNotEmpty.message');
    case 'deleteIncomplete':
      return t('errorReasons.deleteIncomplete.message');
    case 'deleteRecoveryFailed':
      return t('errorReasons.deleteRecoveryFailed.message');
    case 'resourceBusy':
      return t('errorReasons.resourceBusy.message');
    case 'systemManagedFile':
      return t('errorReasons.systemManagedFile.message');
    case 'accessDeniedOrBusy':
      return t('errorReasons.accessDeniedOrBusy.message');
    case 'itemChanged':
      return t('errorReasons.itemChanged.message');
    case 'folderUnavailable':
      return t('errorReasons.folderUnavailable.message');
    case 'analysisRootExcluded':
      return t('errorReasons.analysisRootExcluded.message');
    case 'analysisExportUnavailable':
      return t('errorReasons.analysisExportUnavailable.message');
    case 'scanExclusionsChanged':
      return t('errorReasons.scanExclusionsChanged.message');
    case 'folderSelectionLimitExceeded':
      return t('errorReasons.folderSelectionLimitExceeded.message');
    case 'scanResourcesReleasing':
      return t('errorReasons.scanResourcesReleasing.message');
    case 'quickScanUnavailable':
      return t('errorReasons.quickScanUnavailable.message');
    default:
      return store.errorCode ? t(`errors.${store.errorCode}`) : '';
  }
});

const errorTitle = computed(() => {
  switch (store.errorReason) {
    case 'analysisRefreshFailedAfterDelete':
      return t('errorReasons.analysisRefreshFailedAfterDelete.title');
    case 'directoryNotEmpty':
      return t('errorReasons.directoryNotEmpty.title');
    case 'deleteIncomplete':
      return t('errorReasons.deleteIncomplete.title');
    case 'deleteRecoveryFailed':
      return t('errorReasons.deleteRecoveryFailed.title');
    case 'resourceBusy':
      return t('errorReasons.resourceBusy.title');
    case 'systemManagedFile':
      return t('errorReasons.systemManagedFile.title');
    case 'accessDeniedOrBusy':
      return t('errorReasons.accessDeniedOrBusy.title');
    case 'itemChanged':
      return t('errorReasons.itemChanged.title');
    case 'folderUnavailable':
      return t('errorReasons.folderUnavailable.title');
    case 'analysisRootExcluded':
      return t('errorReasons.analysisRootExcluded.title');
    case 'analysisExportUnavailable':
      return t('errorReasons.analysisExportUnavailable.title');
    case 'scanExclusionsChanged':
      return t('errorReasons.scanExclusionsChanged.title');
    case 'folderSelectionLimitExceeded':
      return t('errorReasons.folderSelectionLimitExceeded.title');
    case 'scanResourcesReleasing':
      return t('errorReasons.scanResourcesReleasing.title');
    case 'quickScanUnavailable':
      return t('errorReasons.quickScanUnavailable.title');
    default:
      return store.errorCode ? t(`errorTitles.${store.errorCode}`) : t('common.operationFailed');
  }
});

watch(
  [() => store.errorCode, () => store.errorReason, errorTitle, errorMessage],
  ([errorCode, errorReason, title, message]) => {
    if (!errorCode) {
      toast.dismiss(APPLICATION_ERROR_TOAST_ID);
      return;
    }
    // One notification renderer owns measurement and stacking for every command error. Keeping
    // this adapter outside the application shell also prevents page navigation from owning error
    // presentation details.
    toast.error(title, {
      id: APPLICATION_ERROR_TOAST_ID,
      description: message,
      duration: Infinity,
      onDismiss: () => {
        if (store.errorCode === errorCode && store.errorReason === errorReason) store.clearError();
      },
    });
  },
  { immediate: true }
);
</script>

<template><span class="hidden" aria-hidden="true" /></template>
