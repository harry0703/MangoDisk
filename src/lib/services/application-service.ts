import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import type {
  ApplicationLeftoverPlanItem,
  ApplicationLeftoverResult,
  ApplicationLeftoverScanResult,
  ApplicationUninstallBatchPlan,
  ApplicationUninstallBatchPreparation,
  ApplicationUninstallBatchResult,
  ApplicationUninstallBatchSelection,
  ApplicationUninstallExecutionProgress,
  ApplicationUninstallScanResult,
} from '@/lib/models/application';
import type { TraversalProgress } from '@/lib/models/progress';
import { EVENT_NAMES } from '@/lib/models/telemetry';
import type { ApplicationCloseBatchResult, ApplicationCloseMode } from '@/lib/models/application-close';

interface ApplicationUninstallCloseResponse {
  closeResult: ApplicationCloseBatchResult;
  catalog: ApplicationUninstallScanResult;
}

export class ApplicationService {
  static recordUninstallDetailsOpened(applicationId: string, catalogRevision: string): Promise<void> {
    return invoke<void>('log_application_uninstall_details', { applicationId, catalogRevision });
  }

  static removeRecord(applicationId: string): Promise<void> {
    return invoke<void>('remove_application_record', { applicationId });
  }

  static openWindowsInstalledApps(): Promise<void> {
    return invoke<void>('open_windows_installed_apps');
  }

  static scanLeftovers(): Promise<ApplicationLeftoverScanResult> {
    return invoke<ApplicationLeftoverScanResult>('scan_application_leftovers');
  }

  static scanUninstallCatalog(): Promise<ApplicationUninstallScanResult> {
    return invoke<ApplicationUninstallScanResult>('scan_application_uninstall_catalog');
  }

  static cancelUninstallCatalogScan(): Promise<void> {
    return invoke<void>('cancel_application_uninstall_catalog_scan');
  }

  static closeUninstallApplications(
    applicationIds: string[],
    mode: ApplicationCloseMode,
    catalogRevision: string
  ): Promise<ApplicationUninstallCloseResponse> {
    return invoke<ApplicationUninstallCloseResponse>('close_application_uninstall_applications', {
      request: { applicationIds, mode },
      catalogRevision,
    });
  }

  static listenUninstallProgress(handler: (progress: TraversalProgress) => void): Promise<() => void> {
    return listen<TraversalProgress>(EVENT_NAMES.applicationUninstallProgress, event => handler(event.payload));
  }

  static prepareUninstallBatch(
    selections: ApplicationUninstallBatchSelection[],
    catalogRevision: string
  ): Promise<ApplicationUninstallBatchPreparation> {
    return invoke<ApplicationUninstallBatchPreparation>('prepare_application_uninstall_batch', {
      selections,
      catalogRevision,
    });
  }

  static executeUninstallBatch(
    plan: ApplicationUninstallBatchPlan,
    dryRun: boolean,
    authorizationPrompt: string
  ): Promise<ApplicationUninstallBatchResult> {
    return invoke<ApplicationUninstallBatchResult>('execute_application_uninstall_batch', {
      plan,
      dryRun,
      authorizationPrompt,
    });
  }

  static cancelUninstallExecution(): Promise<void> {
    return invoke<void>('cancel_application_uninstall_execution');
  }

  /**
   * Subscribe before invoking the batch so validation and the first
   * application boundary cannot be missed. The listener is always released
   * after success or failure, allowing repeated uninstall sessions safely.
   */
  static async executeUninstallBatchWithProgress(
    plan: ApplicationUninstallBatchPlan,
    dryRun: boolean,
    authorizationPrompt: string,
    handler: (progress: ApplicationUninstallExecutionProgress) => void
  ): Promise<ApplicationUninstallBatchResult> {
    const unlisten = await ApplicationService.listenUninstallExecutionProgress(handler);
    try {
      return await ApplicationService.executeUninstallBatch(plan, dryRun, authorizationPrompt);
    } finally {
      unlisten();
    }
  }

  static listenUninstallExecutionProgress(
    handler: (progress: ApplicationUninstallExecutionProgress) => void
  ): Promise<() => void> {
    return listen<ApplicationUninstallExecutionProgress>(EVENT_NAMES.applicationUninstallExecutionProgress, event =>
      handler(event.payload)
    );
  }

  static deleteLeftoversPermanently(
    reviewedItems: ApplicationLeftoverPlanItem[],
    dryRun: boolean,
    deepCleanupOperationId: string
  ): Promise<ApplicationLeftoverResult> {
    return invoke<ApplicationLeftoverResult>('execute_application_leftovers', {
      reviewedItems,
      dryRun,
      deepCleanupOperationId,
    });
  }

  static cancelLeftoverDeletion(): Promise<void> {
    return invoke<void>('cancel_application_leftovers');
  }
}
