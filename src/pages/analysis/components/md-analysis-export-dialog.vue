<script setup lang="ts">
import { ref, watch } from 'vue';
import { storeToRefs } from 'pinia';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';

import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdOperationDialog from '@/components/custom/md-operation-dialog.vue';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Dialog, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import type { AnalysisResult } from '@/lib/models/storage/analysis';
import {
  ANALYSIS_EXPORT_DEPTHS,
  ANALYSIS_EXPORT_ENTRY_LIMITS,
  ANALYSIS_EXPORT_FORMATS,
} from '@/lib/models/storage/analysis-export';
import { useAnalysisExportPreferencesStore } from '@/stores/analysis-export-preferences-store';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import { AnalysisExportService } from '@/lib/services/storage/analysis-export-service';
import { FileManagerService } from '@/lib/services/filesystem/file-manager-service';
import { FormatService } from '@/lib/services/presentation/format-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import * as PathUtils from '@/lib/utils/filesystem/path';
import { parseCommandError, parseCommandErrorReason } from '@/lib/utils/diagnostics/error';

const props = defineProps<{ open: boolean; result: AnalysisResult | null }>();
const emit = defineEmits<{ 'update:open': [open: boolean]; rescan: [path: string] }>();
const { t } = useI18n({ useScope: 'global' });
const exportPreferences = useAnalysisExportPreferencesStore();
const { preferences } = storeToRefs(exportPreferences);
const saving = ref(false);
const unavailable = ref(false);
const failed = ref(false);

function changeDepth(value: unknown) {
  if (typeof value !== 'string') return;
  exportPreferences.setDepth(Number(value));
}
function changeLimit(value: unknown) {
  if (typeof value !== 'string') return;
  exportPreferences.setEntryLimit(Number(value));
}

function close(open: boolean) {
  if (!saving.value) emit('update:open', open);
}

async function reveal(path: string) {
  try {
    await FileManagerService.reveal(path);
  } catch (error) {
    LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.operationFailed, {
      operation: 'reveal_analysis_export',
      path,
      error,
    });
    toast.error(t('analysis.export.revealFailed'));
  }
}

async function save() {
  if (saving.value || !props.result) return;
  if (unavailable.value) {
    emit('update:open', false);
    emit('rescan', props.result.root);
    return;
  }
  saving.value = true;
  failed.value = false;
  try {
    const receipt = await AnalysisExportService.save(
      {
        scanId: props.result.scanId,
        includeFullPaths: preferences.value.includeFullPaths,
        maxDepth: preferences.value.maxDepth,
        maxEntries: preferences.value.maxEntries,
        format: preferences.value.format,
      },
      t('analysis.export.saveTitle')
    );
    if (!receipt) return;
    emit('update:open', false);
    toast.success(t('analysis.export.saved'), {
      description: PathUtils.fileName(receipt.path),
      action: {
        label: t('analysis.export.showFile'),
        onClick: () => {
          void reveal(receipt.path);
        },
      },
    });
  } catch (error) {
    unavailable.value = parseCommandErrorReason(parseCommandError(error)) === 'analysisExportUnavailable';
    failed.value = !unavailable.value;
  } finally {
    saving.value = false;
  }
}

watch(
  () => [props.open, props.result?.scanId] as const,
  ([open]) => {
    if (open) {
      unavailable.value = false;
      failed.value = false;
    }
  },
  { immediate: true }
);
</script>

<template>
  <Dialog :open="open && !saving" @update:open="close">
    <MdDialogContent size="compact" :blur-background="false" :scale-on-open="false" class="flex min-h-0 flex-col">
      <MdDialogHeader>
        <DialogTitle>{{ t('analysis.export.title') }}</DialogTitle>
        <DialogDescription>{{ t('analysis.export.description') }}</DialogDescription>
      </MdDialogHeader>
      <div class="export-body">
        <div v-if="result" class="export-current-list">
          <p class="export-list-label">
            {{ t('analysis.export.currentDirectory') }}
          </p>
          <MdTooltip :text="result.root"
            ><span class="export-path">{{ result.root }}</span></MdTooltip
          >
        </div>
        <section class="export-field">
          <p id="analysis-export-format" class="export-list-label">{{ t('analysis.export.format') }}</p>
          <Select :model-value="preferences.format" @update:model-value="exportPreferences.setFormat">
            <SelectTrigger size="sm" class="w-full" aria-labelledby="analysis-export-format">
              <SelectValue>{{
                t(preferences.format === 'json' ? 'analysis.export.formatJson' : 'analysis.export.formatMarkdown')
              }}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem v-for="format in ANALYSIS_EXPORT_FORMATS" :key="format" :value="format">{{
                t(format === 'json' ? 'analysis.export.formatJson' : 'analysis.export.formatMarkdown')
              }}</SelectItem>
            </SelectContent>
          </Select>
        </section>
        <section class="export-field">
          <p id="analysis-export-depth" class="export-list-label">{{ t('analysis.export.depth') }}</p>
          <Select :model-value="String(preferences.maxDepth)" @update:model-value="changeDepth">
            <SelectTrigger size="sm" class="w-full" aria-labelledby="analysis-export-depth">
              <SelectValue>{{ t('analysis.export.depthOption', { count: preferences.maxDepth }) }}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem v-for="depth in ANALYSIS_EXPORT_DEPTHS" :key="depth" :value="String(depth)">{{
                t('analysis.export.depthOption', { count: depth })
              }}</SelectItem>
            </SelectContent>
          </Select>
          <p class="export-hint">{{ t('analysis.export.depthHint') }}</p>
        </section>
        <section class="export-field">
          <p id="analysis-export-limit" class="export-list-label">{{ t('analysis.export.entryLimit') }}</p>
          <Select :model-value="String(preferences.maxEntries)" @update:model-value="changeLimit">
            <SelectTrigger size="sm" class="w-full" aria-labelledby="analysis-export-limit">
              <SelectValue>{{
                preferences.maxEntries === 0
                  ? t('analysis.export.allAvailable')
                  : t('analysis.export.entryOption', { count: FormatService.integer(preferences.maxEntries) })
              }}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem v-for="limit in ANALYSIS_EXPORT_ENTRY_LIMITS" :key="limit" :value="String(limit)">{{
                limit === 0
                  ? t('analysis.export.allAvailable')
                  : t('analysis.export.entryOption', { count: FormatService.integer(limit) })
              }}</SelectItem>
            </SelectContent>
          </Select>
          <p class="export-hint min-h-[2lh]">
            {{
              preferences.maxEntries === 0
                ? t('analysis.export.allLimitHint')
                : t('analysis.export.limitHint', { count: FormatService.integer(preferences.maxEntries) })
            }}
          </p>
        </section>
        <label class="export-paths">
          <Checkbox :model-value="preferences.includeFullPaths" @update:model-value="exportPreferences.setFullPaths" />
          <span>{{ t('analysis.export.fullPaths') }}</span>
        </label>
        <p v-if="unavailable" role="alert" class="text-destructive-text">{{ t('analysis.export.unavailable') }}</p>
        <p v-else-if="failed" role="alert" class="text-destructive-text">{{ t('analysis.export.saveFailed') }}</p>
      </div>
      <MdDialogFooter>
        <Button variant="outline" @click="close(false)">{{ t('common.cancel') }}</Button>
        <Button :disabled="!result" @click="save">{{
          t(unavailable ? 'analysis.export.rescan' : 'analysis.export.save')
        }}</Button>
      </MdDialogFooter>
    </MdDialogContent>
  </Dialog>
  <MdOperationDialog
    :open="open && saving"
    size="compact"
    :title="t('analysis.export.title')"
    :description="t('analysis.export.saving')"
    :icon-name="ICON_NAMES.download"
    :progress-label="t('analysis.export.saving')"
  />
</template>

<style scoped>
@reference "@assets/main.css";
.export-body {
  display: grid;
  gap: 20px;
  min-height: 0;
  overflow-y: auto;
  padding: 8px var(--layout-dialog-body-inline-padding) 24px;
}
.export-field,
.export-current-list {
  display: grid;
  min-width: 0;
  gap: 6px;
}
.export-hint,
.export-list-label {
  font-size: var(--font-content-meta);
  @apply text-muted-foreground;
}
.export-path {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-content-body);
}
.export-paths {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: var(--font-content-body);
}
</style>
