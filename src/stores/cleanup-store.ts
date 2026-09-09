import { defineStore } from 'pinia';

import type { ApplicationCloseBatchResult, ApplicationCloseMode } from '@/lib/models/application-close';
import {
  CLEANUP_OPERATION_IDS,
  CLEANUP_RULE_IDS,
  CLEANUP_SCAN_SCOPE_MODES,
  STANDARD_CLEANUP_SCAN_SCOPE,
} from '@/lib/models/cleanup';
import type {
  CleanupOperationId,
  CleanupExecutionProgress,
  CleanupResult,
  CleanupScanScope,
  CleanupScanResult,
  CleanupSourceSelection,
} from '@/lib/models/cleanup';
import type { TraversalProgress } from '@/lib/models/progress';
import { CleanupService } from '@/lib/services/cleanup-service';
import * as CleanupExecutionResultUtils from '@/lib/utils/cleanup-execution-result';
import * as CleanupRuleSelectionUtils from '@/lib/utils/cleanup-rule-selection';
import { parseCommandError } from '@/lib/utils/error';

import { useAppStore } from './app-store';
import { useHistoryStore } from './history-store';

interface CleanupState {
  scan: CleanupScanResult | null;
  scanScope: CleanupScanScope;
  scanProgress: TraversalProgress | null;
  executionProgress: CleanupExecutionProgress | null;
  executionStartedAtMs: number | null;
  executionRuleIds: string[];
  selectedRuleIds: string[];
  sourceSelections: CleanupSourceSelection[];
  result: CleanupResult | null;
  loading: boolean;
  operation: CleanupOperationId;
  closingApplications: boolean;
  applicationCloseResult: ApplicationCloseBatchResult | null;
  privilegedScanRuleId: string | null;
}

function isCleanupCancellation(operation: CleanupOperationId): boolean {
  return operation === CLEANUP_OPERATION_IDS.cancelling;
}

export const useCleanupStore = defineStore('cleanup', {
  state: (): CleanupState => ({
    scan: null,
    scanScope: STANDARD_CLEANUP_SCAN_SCOPE,
    scanProgress: null,
    executionProgress: null,
    executionStartedAtMs: null,
    executionRuleIds: [],
    selectedRuleIds: [],
    sourceSelections: [],
    result: null,
    loading: false,
    operation: CLEANUP_OPERATION_IDS.idle,
    closingApplications: false,
    applicationCloseResult: null,
    privilegedScanRuleId: null,
  }),
  getters: {
    selectedBytes(state): number {
      return state.scan
        ? CleanupRuleSelectionUtils.selectedBytes(state.scan.rules, state.selectedRuleIds, state.sourceSelections)
        : 0;
    },
  },
  actions: {
    initialize() {
      this.loading = false;
      this.scan = null;
      this.scanScope = STANDARD_CLEANUP_SCAN_SCOPE;
      this.scanProgress = null;
      this.executionProgress = null;
      this.executionStartedAtMs = null;
      this.executionRuleIds = [];
      this.selectedRuleIds = [];
      this.sourceSelections = [];
      this.result = null;
      this.operation = CLEANUP_OPERATION_IDS.idle;
      this.closingApplications = false;
      this.applicationCloseResult = null;
      this.privilegedScanRuleId = null;
    },
    async closeApplications(ruleIds: string[], mode: ApplicationCloseMode) {
      if (this.loading || this.closingApplications || !ruleIds.length) return null;
      const appStore = useAppStore();
      this.closingApplications = true;
      this.applicationCloseResult = null;
      appStore.clearError();
      try {
        const result = await CleanupService.closeApplications(ruleIds, mode);
        // The confirmation dialog may immediately continue into cleanup, so
        // publish the close result only after the shared busy state is clear.
        this.closingApplications = false;
        this.applicationCloseResult = result;
        return result;
      } catch (error) {
        appStore.reportError(error);
        return null;
      } finally {
        this.closingApplications = false;
      }
    },
    async scanCandidates(scanScope: CleanupScanScope = STANDARD_CLEANUP_SCAN_SCOPE): Promise<boolean> {
      if (this.loading || this.closingApplications) return false;
      const appStore = useAppStore();
      let completed = false;
      this.loading = true;
      this.operation = CLEANUP_OPERATION_IDS.scanning;
      this.scanProgress = null;
      appStore.clearError();
      try {
        const snapshot = await CleanupService.scanWithProgress(scanScope, progress => {
          this.scanProgress = progress;
        });
        this.scan = snapshot;
        if (scanScope.mode === CLEANUP_SCAN_SCOPE_MODES.selectedVolumes) {
          this.scanScope = { mode: scanScope.mode, volumeMountPoints: [...scanScope.volumeMountPoints] };
        } else if (scanScope.mode === CLEANUP_SCAN_SCOPE_MODES.custom) {
          this.scanScope = {
            mode: scanScope.mode,
            includeStandardRules: scanScope.includeStandardRules,
            scanId: snapshot.customScanId ?? undefined,
            rules: scanScope.rules.map(rule => ({
              ...rule,
              roots: [...rule.roots],
              namePatterns: [...rule.namePatterns],
              modifiedTime: { ...rule.modifiedTime },
            })),
          };
        } else {
          this.scanScope = STANDARD_CLEANUP_SCAN_SCOPE;
        }
        appStore.updateSystemDisk(snapshot.disk);
        this.selectedRuleIds = [];
        this.sourceSelections = [];
        this.setRulesSelected(CleanupRuleSelectionUtils.defaultSelectedRuleIds(snapshot.rules), true);
        this.result = null;
        completed = true;
      } catch (error) {
        if (!isCleanupCancellation(this.operation)) {
          appStore.reportError(error);
        }
      } finally {
        // Keep the final traversal snapshot while the deep-cleanup workflow
        // continues into application-leftover discovery. The next scan clears
        // it before starting, so metrics remain cumulative only within the
        // current user-initiated analysis.
        this.loading = false;
        this.operation = CLEANUP_OPERATION_IDS.idle;
      }
      return completed;
    },
    async cancelScan() {
      if (!this.loading || this.operation !== CLEANUP_OPERATION_IDS.scanning) return;
      this.operation = CLEANUP_OPERATION_IDS.cancelling;
      try {
        await CleanupService.cancelScan();
      } catch (error) {
        useAppStore().reportError(error);
        this.operation = CLEANUP_OPERATION_IDS.scanning;
      }
    },
    async scanPreviousInstallationsWithPrivileges(): Promise<boolean> {
      const ruleId = CLEANUP_RULE_IDS.windowsPreviousInstallations;
      const snapshotId = this.scan?.scannedAtMs;
      const currentRule = this.scan?.rules.find(rule => rule.ruleId === ruleId);
      if (!this.scan || this.privilegedScanRuleId || currentRule?.status !== 'requiresElevation') return false;
      this.privilegedScanRuleId = ruleId;
      try {
        const rule = await CleanupService.scanPreviousInstallationsWithPrivileges();
        // A standard rescan may finish while the UAC-owned native handler is still measuring.
        // Never merge an older privileged result into a newer cleanup snapshot.
        if (!this.scan || this.scan.scannedAtMs !== snapshotId) return false;
        const rules = this.scan.rules.map(candidate => (candidate.ruleId === ruleId ? rule : candidate));
        this.scan = {
          ...this.scan,
          rules,
          reclaimableBytes: rules
            .filter(candidate => candidate.selectable)
            .reduce((total, candidate) => total + candidate.bytes, 0),
          safeBytes: rules
            .filter(candidate => candidate.selectable && candidate.risk === 'safe')
            .reduce((total, candidate) => total + candidate.bytes, 0),
        };
        return rule.status === 'found' || rule.status === 'clean';
      } catch (error) {
        if (parseCommandError(error)?.code !== 'operationCancelled') useAppStore().reportError(error);
        return false;
      } finally {
        this.privilegedScanRuleId = null;
      }
    },
    async cancelExecution() {
      if (!this.loading || this.operation !== CLEANUP_OPERATION_IDS.cleaning) return;
      try {
        await CleanupService.cancelExecution();
      } catch (error) {
        useAppStore().reportError(error);
        throw error;
      }
    },
    toggleSource(ruleId: string, sourcePath: string) {
      const rule = this.scan?.rules.find(item => item.ruleId === ruleId);
      const source = rule?.sources.find(item => item.path === sourcePath);
      if (!rule?.selectable || !source || !CleanupRuleSelectionUtils.sourceSelectable(rule, source)) return;

      const ruleSelected = this.selectedRuleIds.includes(ruleId);
      const existing = this.sourceSelections.find(selection => selection.ruleId === ruleId);
      if (!ruleSelected) {
        this.selectedRuleIds = [...this.selectedRuleIds, ruleId];
        this.sourceSelections = [...this.sourceSelections, { ruleId, mode: 'include', paths: [sourcePath] }];
        return;
      }

      if (!existing) {
        this.sourceSelections = [...this.sourceSelections, { ruleId, mode: 'exclude', paths: [sourcePath] }];
        return;
      }

      const paths = existing.paths.includes(sourcePath)
        ? existing.paths.filter(path => path !== sourcePath)
        : [...existing.paths, sourcePath];
      if (!paths.length) {
        if (existing.mode === 'include') {
          this.selectedRuleIds = this.selectedRuleIds.filter(id => id !== ruleId);
        }
        this.sourceSelections = this.sourceSelections.filter(selection => selection.ruleId !== ruleId);
        return;
      }

      if (!rule.sourcesTruncated && paths.length === rule.sources.length) {
        if (existing.mode === 'include') {
          this.sourceSelections = this.sourceSelections.filter(selection => selection.ruleId !== ruleId);
        } else {
          this.selectedRuleIds = this.selectedRuleIds.filter(id => id !== ruleId);
          this.sourceSelections = this.sourceSelections.filter(selection => selection.ruleId !== ruleId);
        }
        return;
      }

      this.sourceSelections = this.sourceSelections.map(selection =>
        selection.ruleId === ruleId ? { ...selection, paths } : selection
      );
    },
    setRulesSelected(ruleIds: string[], selected: boolean) {
      const targetIds = new Set(ruleIds);
      if (!selected) {
        this.selectedRuleIds = this.selectedRuleIds.filter(id => !targetIds.has(id));
        this.sourceSelections = this.sourceSelections.filter(selection => !targetIds.has(selection.ruleId));
        return;
      }

      const selectedIds = new Set(this.selectedRuleIds);
      const sourceSelections = this.sourceSelections.filter(selection => !targetIds.has(selection.ruleId));
      for (const ruleId of ruleIds) {
        const rule = this.scan?.rules.find(item => item.ruleId === ruleId);
        if (!rule?.selectable) {
          selectedIds.delete(ruleId);
          continue;
        }
        const hasCompleteBlockedSources =
          !rule.sourcesTruncated &&
          rule.sources.some(source => !CleanupRuleSelectionUtils.sourceSelectable(rule, source));
        if (!hasCompleteBlockedSources) {
          selectedIds.add(ruleId);
          continue;
        }

        /*
         * A bulk selection must match the disabled checkbox state shown by the
         * UI. Keep blocked sources outside the execution scope instead of
         * selecting the whole aggregated rule and relying on Core to skip them.
         */
        const selectablePaths = rule.sources
          .filter(source => CleanupRuleSelectionUtils.sourceSelectable(rule, source))
          .map(source => source.path);
        if (!selectablePaths.length) {
          selectedIds.delete(ruleId);
          continue;
        }
        selectedIds.add(ruleId);
        sourceSelections.push({ ruleId, mode: 'include', paths: selectablePaths });
      }

      this.selectedRuleIds = [...selectedIds];
      this.sourceSelections = sourceSelections;
    },
    async execute(dryRun: boolean, deepCleanupOperationId = crypto.randomUUID()): Promise<boolean> {
      if (this.loading || this.closingApplications || !this.selectedRuleIds.length) return false;
      const appStore = useAppStore();
      let completed = false;
      this.loading = true;
      this.operation = dryRun ? CLEANUP_OPERATION_IDS.previewing : CLEANUP_OPERATION_IDS.cleaning;
      this.executionProgress = null;
      this.executionStartedAtMs = Date.now();
      this.executionRuleIds = [...this.selectedRuleIds];
      appStore.clearError();
      try {
        const executedSourceSelections = this.sourceSelections;
        const result = await CleanupService.executeWithProgress(
          this.selectedRuleIds,
          this.sourceSelections,
          dryRun,
          this.scanScope,
          deepCleanupOperationId,
          progress => {
            // Core owns the execution pipeline and therefore supplies the only
            // authoritative queue. Keeping it in Store state prevents the UI
            // from presenting user selection order as execution order.
            if (progress.plannedRuleIds.length) {
              this.executionRuleIds = [...progress.plannedRuleIds];
            }
            this.executionProgress = progress;
          }
        );
        this.result = result;
        completed = true;
        if (!dryRun && this.scan) {
          this.scan = CleanupExecutionResultUtils.apply(this.scan, result, executedSourceSelections);
          const completedRuleIds = CleanupExecutionResultUtils.completedRuleIds(result);
          const invalidatedSourceRuleIds = CleanupExecutionResultUtils.invalidatedSourceRuleIds(result);
          const explicitlyScopedRuleIds = new Set(executedSourceSelections.map(selection => selection.ruleId));
          this.selectedRuleIds = this.selectedRuleIds.filter(
            ruleId =>
              !completedRuleIds.has(ruleId) &&
              !(invalidatedSourceRuleIds.has(ruleId) && explicitlyScopedRuleIds.has(ruleId))
          );
          this.sourceSelections = this.sourceSelections.filter(
            selection => !completedRuleIds.has(selection.ruleId) && !invalidatedSourceRuleIds.has(selection.ruleId)
          );
        }
        /*
         * Keep the completed result visible without launching another cleanup
         * scan. History and disk capacity are secondary views, so refresh them
         * independently and never turn a successful cleanup into an error.
         */
        const secondaryRefreshes: Promise<unknown>[] = [];
        if (result.historySaved) {
          secondaryRefreshes.push(useHistoryStore().load({ reportError: false }));
        }
        if (!dryRun) {
          secondaryRefreshes.push(
            appStore.refreshSystemDisk().then(refreshed => {
              const disk = appStore.disk;
              if (refreshed && disk && this.scan?.disk.mountPoint === disk.mountPoint) {
                this.scan = { ...this.scan, disk };
              }
            })
          );
        }
        await Promise.all(secondaryRefreshes);
      } catch (error) {
        // A cancellation observed before the first filesystem mutation returns
        // a typed error instead of a partial result. It is an expected user
        // outcome; all unrelated execution failures remain visible.
        if (parseCommandError(error)?.code !== 'operationCancelled') appStore.reportError(error);
      } finally {
        this.loading = false;
        this.scanProgress = null;
        this.executionProgress = null;
        this.executionStartedAtMs = null;
        this.executionRuleIds = [];
        this.operation = CLEANUP_OPERATION_IDS.idle;
      }
      return completed;
    },
  },
});
