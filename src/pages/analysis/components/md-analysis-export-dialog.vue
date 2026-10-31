<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';

import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdOperationDialog from '@/components/custom/md-operation-dialog.vue';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Dialog, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import type { AnalysisResult } from '@/lib/models/storage/analysis';
import type {
  AnalysisExportDetail,
  AnalysisExportOptions,
  AnalysisExportScope,
} from '@/lib/models/storage/analysis-export';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import { AnalysisExportService } from '@/lib/services/storage/analysis-export-service';
import { FileManagerService } from '@/lib/services/filesystem/file-manager-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { ANALYSIS_EXPORT_MINIMUM_SIZE_PRESETS } from '@/lib/models/storage/analysis-export';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import * as PathUtils from '@/lib/utils/filesystem/path';
import { parseCommandError, parseCommandErrorReason } from '@/lib/utils/diagnostics/error';

const props = defineProps<{ open: boolean; result: AnalysisResult | null }>();
const emit = defineEmits<{ 'update:open': [open: boolean]; loading: [loading: boolean]; rescan: [path: string] }>();
const { t } = useI18n({ useScope: 'global' });
const options = shallowRef<AnalysisExportOptions | null>(null);
const scope = ref<AnalysisExportScope>('scanRoot');
const includeFullPaths = ref(false);
const detail = ref<AnalysisExportDetail>('summary');
const minDirectoryBytes = ref(0);
const minimumSizeOptions = ByteSizeService.presetOptions(ANALYSIS_EXPORT_MINIMUM_SIZE_PRESETS);
const minimumSizeLabel = computed(
  () =>
    minimumSizeOptions.find(option => option.bytes === minDirectoryBytes.value)?.label ?? t('analysis.export.noMinimum')
);
const loading = ref(false);
const saving = ref(false);
const unavailable = ref(false);
const failure = ref<'load' | 'save' | null>(null);
let request = 0;

watch(loading, value => emit('loading', value), { flush: 'sync' });

const selected = computed(() => options.value?.[scope.value === 'scanRoot' ? 'scanRoot' : 'currentDirectory']);
const hasScopeChoice = computed(
  () =>
    options.value &&
    PathUtils.comparisonKey(options.value.scanRoot.path) !==
      PathUtils.comparisonKey(options.value.currentDirectory.path)
);

function changeMinimumSize(value: unknown) {
  if (typeof value !== 'string') return;
  const bytes = Number(value);
  if (bytes === 0 || minimumSizeOptions.some(option => option.bytes === bytes)) minDirectoryBytes.value = bytes;
}

function changeDetail(value: unknown) {
  if (value === 'summary' || value === 'full') detail.value = value;
}

function close(open: boolean) {
  if (!saving.value) emit('update:open', open);
}

async function load() {
  const id = ++request;
  const scanId = props.result?.scanId;
  options.value = null;
  failure.value = null;
  unavailable.value = false;
  if (!scanId) return;
  loading.value = true;
  try {
    const response = await AnalysisExportService.options(scanId);
    if (id === request && props.open) options.value = response;
  } catch (error) {
    if (id !== request || !props.open) return;
    unavailable.value = parseCommandErrorReason(parseCommandError(error)) === 'analysisExportUnavailable';
    failure.value = unavailable.value ? null : 'load';
    LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.operationFailed, {
      operation: 'analysis_export_options',
      scanId,
      error,
    });
  } finally {
    if (id === request) loading.value = false;
  }
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
  if (saving.value || loading.value) return;
  if (unavailable.value) {
    const path = selected.value?.path ?? props.result?.root;
    if (path) {
      emit('update:open', false);
      emit('rescan', path);
    }
    return;
  }
  if (failure.value === 'load') {
    await load();
    return;
  }
  if (!options.value) return;
  saving.value = true;
  failure.value = null;
  try {
    const receipt = await AnalysisExportService.save(
      options.value.scanId,
      scope.value,
      includeFullPaths.value,
      t('analysis.export.saveTitle'),
      detail.value,
      minDirectoryBytes.value
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
    failure.value = unavailable.value ? null : 'save';
  } finally {
    saving.value = false;
  }
}

watch(
  () => [props.open, props.result?.scanId] as const,
  ([open]) => {
    request++;
    if (open) {
      scope.value = 'scanRoot';
      includeFullPaths.value = false;
      detail.value = 'summary';
      minDirectoryBytes.value = 0;
      void load();
    } else {
      loading.value = false;
    }
  },
  { immediate: true }
);
onBeforeUnmount(() => {
  request++;
});
</script>

<template>
  <Dialog :open="open && !saving && !loading" @update:open="close">
    <MdDialogContent
      size="compact"
      :blur-background="false"
      :scale-on-open="false"
      class="analysis-export-dialog flex min-h-0 flex-col"
    >
      <MdDialogHeader>
        <DialogTitle>{{ t('analysis.export.title') }}</DialogTitle>
        <DialogDescription>{{ t('analysis.export.description') }}</DialogDescription>
      </MdDialogHeader>
      <div class="export-body">
        <p v-if="loading" role="status" class="text-muted-foreground">{{ t('analysis.export.loading') }}</p>
        <p v-else-if="unavailable" role="status" class="export-notice">{{ t('analysis.export.unavailable') }}</p>
        <p v-else-if="failure === 'load'" role="alert" class="text-destructive-text">
          {{ t('analysis.export.loadFailed') }}
        </p>
        <template v-if="options && !unavailable">
          <section class="export-section">
            <fieldset v-if="hasScopeChoice" class="export-scopes">
              <legend>{{ t('analysis.export.scope') }}</legend>
              <label
                v-for="item in ['scanRoot', 'currentDirectory'] as const"
                :key="item"
                class="export-scope"
                :class="{ selected: scope === item }"
              >
                <input
                  v-model="scope"
                  type="radio"
                  name="analysis-export-scope"
                  :value="item"
                  class="peer sr-only"
                  :aria-labelledby="`analysis-export-scope-${item}`"
                />
                <span
                  aria-hidden="true"
                  class="mt-0.5 grid size-4 shrink-0 place-items-center rounded-full border border-muted-foreground/60 bg-background peer-checked:border-primary peer-checked:bg-primary peer-focus-visible:ring-2 peer-focus-visible:ring-ring/35 peer-focus-visible:ring-offset-2 peer-focus-visible:ring-offset-background"
                >
                  <span
                    class="size-1.5 rounded-full bg-primary-foreground"
                    :class="scope === item ? 'opacity-100' : 'opacity-0'"
                  />
                </span>
                <span class="export-scope-copy">
                  <span :id="`analysis-export-scope-${item}`" class="export-label">{{
                    t(item === 'scanRoot' ? 'analysis.export.scanRoot' : 'analysis.export.currentDirectory')
                  }}</span>
                  <MdTooltip :text="options[item].path"
                    ><span class="export-path">{{ options[item].path }}</span></MdTooltip
                  >
                </span>
              </label>
            </fieldset>
            <template v-else>
              <p class="export-section-label">{{ t('analysis.export.scope') }}</p>
              <div class="export-root">
                <span class="export-label">{{ t('analysis.export.scanRoot') }}</span>
                <MdTooltip :text="options.scanRoot.path"
                  ><span class="export-path">{{ options.scanRoot.path }}</span></MdTooltip
                >
              </div>
            </template>
          </section>
          <section class="export-section">
            <p id="analysis-export-detail-label" class="export-section-label">{{ t('analysis.export.detail') }}</p>
            <Select :model-value="detail" @update:model-value="changeDetail">
              <SelectTrigger size="sm" class="w-full" aria-labelledby="analysis-export-detail-label">
                <SelectValue>{{
                  t(detail === 'summary' ? 'analysis.export.summary' : 'analysis.export.full')
                }}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="summary">{{ t('analysis.export.summary') }}</SelectItem>
                <SelectItem value="full">{{ t('analysis.export.full') }}</SelectItem>
              </SelectContent>
            </Select>
            <div class="export-detail-hints export-field-hint">
              <p
                v-for="mode in ['summary', 'full'] as const"
                :key="mode"
                class="export-hint"
                :class="{ 'export-detail-hint-hidden': detail !== mode }"
                :aria-hidden="detail !== mode"
              >
                {{ t(mode === 'summary' ? 'analysis.export.summaryHint' : 'analysis.export.fullHint') }}
              </p>
            </div>
          </section>
          <section class="export-section">
            <p id="analysis-export-minimum-label" class="export-section-label">
              {{ t('analysis.export.minimumSize') }}
            </p>
            <Select :model-value="String(minDirectoryBytes)" @update:model-value="changeMinimumSize">
              <SelectTrigger size="sm" class="w-full" aria-labelledby="analysis-export-minimum-label">
                <SelectValue>{{ minimumSizeLabel }}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="0">{{ t('analysis.export.noMinimum') }}</SelectItem>
                <SelectItem v-for="option in minimumSizeOptions" :key="option.bytes" :value="String(option.bytes)">{{
                  option.label
                }}</SelectItem>
              </SelectContent>
            </Select>
            <p class="export-hint export-field-hint">{{ t('analysis.export.sizeFilterHint') }}</p>
          </section>
          <label class="export-paths">
            <Checkbox :model-value="includeFullPaths" @update:model-value="includeFullPaths = $event === true" />
            <span class="export-label">{{ t('analysis.export.fullPaths') }}</span>
          </label>
          <p v-if="failure === 'save'" role="alert" class="text-destructive-text">
            {{ t('analysis.export.saveFailed') }}
          </p>
        </template>
      </div>
      <MdDialogFooter>
        <Button variant="outline" @click="close(false)">{{ t('common.cancel') }}</Button>
        <Button :disabled="loading || (!options && !unavailable && failure !== 'load')" @click="save">{{
          t(
            unavailable
              ? 'analysis.export.rescan'
              : failure === 'load'
                ? 'analysis.export.retry'
                : 'analysis.export.save'
          )
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
  gap: 16px;
  min-height: 0;
  overflow-y: auto;
  padding: 8px var(--layout-dialog-body-inline-padding) 20px;
}
.export-section-label,
.export-scopes legend {
  margin-bottom: 8px;
  font-size: var(--font-content-meta);
  font-weight: 500;
  @apply text-muted-foreground;
}
.export-section {
  display: grid;
  min-width: 0;
}
.export-field-hint {
  margin-top: 8px;
}
/* Share the taller localized hint's natural height so changing detail never shifts the dialog. */
.export-detail-hints {
  display: grid;
}
.export-detail-hints > p {
  grid-area: 1 / 1;
}
.export-detail-hint-hidden {
  visibility: hidden;
}
.export-scopes {
  display: grid;
  gap: 8px;
  min-width: 0;
}
.export-scope,
.export-root {
  min-width: 0;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: 8px;
}
.export-root {
  display: grid;
  gap: 4px;
  @apply bg-muted/30;
}
.export-scope {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  @apply cursor-pointer border-border/70 transition-colors;
}
.export-scope.selected {
  @apply border-primary/30 bg-accent/15;
}
.export-scope-copy {
  display: grid;
  min-width: 0;
  gap: 4px;
}
.export-label {
  font-size: var(--font-content-body);
  font-weight: 500;
  line-height: 1.5;
}
.export-path {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-content-meta);
  line-height: 1.5;
  @apply text-muted-foreground;
}
.export-paths {
  display: flex;
  align-items: center;
  gap: 10px;
}
.export-hint,
.export-notice {
  font-size: var(--font-content-meta);
  line-height: 1.5;
  @apply text-muted-foreground;
}
.export-notice {
  padding: 10px 12px;
  border-radius: 8px;
  @apply bg-muted;
}
</style>
