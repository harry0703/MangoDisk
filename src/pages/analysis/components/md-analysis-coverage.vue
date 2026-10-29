<script setup lang="ts">
import { computed, onDeactivated, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import { Button } from '@/components/ui/button';
import { Dialog, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import type { AnalysisResult } from '@/lib/models/storage/analysis';
import { OperatingSystemService } from '@/lib/services/settings/operating-system-service';
import { FormatService } from '@/lib/services/presentation/format-service';

const props = defineProps<{ result: AnalysisResult; disabled?: boolean; restarting?: boolean }>();
const emit = defineEmits<{ restartAsAdministrator: [scanId: number] }>();
const { t } = useI18n({ useScope: 'global' });
const open = ref(false);
onDeactivated(() => {
  open.value = false;
});
const omissions = computed(
  () =>
    props.result.coverage?.omissions ?? {
      permissionDenied: 0,
      readErrors: 0,
      excluded: 0,
      linkOrMount: 0,
      other: props.result.skippedCount,
    }
);
const reasonKeys = {
  permissionDenied: 'analysis.coverage.permissionDenied',
  readErrors: 'analysis.coverage.readErrors',
  excluded: 'analysis.coverage.excluded',
  linkOrMount: 'analysis.coverage.linkOrMount',
  other: 'analysis.coverage.other',
} as const;
const rows = computed(() =>
  (Object.keys(reasonKeys) as (keyof typeof reasonKeys)[])
    .map(reason => ({ reason, count: omissions.value[reason] }))
    .filter(row => row.count > 0)
);
const total = computed(() => rows.value.reduce((sum, row) => sum + row.count, 0));
const noticeLabel = computed(
  () =>
    `${t('analysis.coverage.summary', { count: FormatService.integer(total.value) })} · ${t('analysis.coverage.details')}`
);
const canElevate = computed(
  () =>
    OperatingSystemService.isWindows() &&
    props.result.coverage?.elevated === false &&
    omissions.value.permissionDenied > 0
);
watch(
  () => props.result.scanId,
  () => {
    open.value = false;
  }
);
</script>

<template>
  <span v-if="total > 0" class="inline-flex min-w-0 max-w-52 shrink-0 items-baseline" role="status">
    <MdTooltip :text="noticeLabel">
      <button
        type="button"
        class="inline-flex min-h-7 min-w-0 items-baseline gap-1.5 rounded-md px-2 py-1.5 text-content-secondary text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/35 disabled:cursor-default disabled:opacity-50"
        :aria-label="noticeLabel"
        aria-haspopup="dialog"
        :aria-expanded="open"
        :disabled="disabled"
        @click="open = true"
      >
        <MdIcon :name="ICON_NAMES.info" :size="13" class="shrink-0 self-center" aria-hidden="true" />
        <span class="coverage-label truncate">{{
          t('analysis.coverage.summary', { count: FormatService.integer(total) })
        }}</span>
        <span class="coverage-count tabular-nums" aria-hidden="true">{{ FormatService.integer(total) }}</span>
      </button>
    </MdTooltip>
  </span>
  <Dialog v-model:open="open">
    <MdDialogContent class="flex min-h-0 flex-col">
      <MdDialogHeader class="coverage-header">
        <DialogTitle>{{ t('analysis.coverage.title') }}</DialogTitle>
        <DialogDescription>{{ t('analysis.coverage.description') }}</DialogDescription>
      </MdDialogHeader>
      <div class="max-h-72 overflow-y-auto px-5 pb-5 scrollbar-stable-end">
        <dl class="divide-y divide-border/50 text-sm">
          <div v-for="{ reason, count } in rows" :key="reason" class="flex items-baseline justify-between gap-6 py-3">
            <dt>{{ t(reasonKeys[reason]) }}</dt>
            <dd class="shrink-0 font-semibold tabular-nums">
              {{ t('common.itemCount', { count: FormatService.integer(count) }, count) }}
            </dd>
          </div>
        </dl>
        <p class="mt-3 text-xs leading-relaxed text-muted-foreground">{{ t('analysis.coverage.countHint') }}</p>
      </div>
      <MdDialogFooter>
        <div class="flex w-full flex-wrap items-center justify-between gap-x-4 gap-y-3">
          <p
            v-if="
              OperatingSystemService.isWindows() &&
              result.coverage?.elevated !== null &&
              result.coverage?.elevated !== undefined
            "
            class="flex items-center gap-1 text-xs leading-relaxed text-muted-foreground"
          >
            <span>
              {{ t(result.coverage.elevated ? 'analysis.coverage.administrator' : 'analysis.coverage.standardUser') }}
            </span>
            <MdTooltip
              v-if="result.coverage.elevated && omissions.permissionDenied > 0"
              :text="t('analysis.coverage.administratorHint')"
            >
              <button
                type="button"
                class="inline-flex size-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/35"
                :aria-label="t('analysis.coverage.administratorHint')"
              >
                <MdIcon :name="ICON_NAMES.info" :size="13" aria-hidden="true" />
              </button>
            </MdTooltip>
          </p>
          <div class="ml-auto flex min-w-0 flex-wrap justify-end gap-2">
            <Button variant="outline" @click="open = false">{{ t('common.close') }}</Button>
            <Button
              v-if="canElevate"
              class="h-auto min-h-9 whitespace-normal text-center"
              :disabled="disabled || restarting"
              @click="emit('restartAsAdministrator', result.scanId)"
            >
              {{ t(restarting ? 'analysis.coverage.restarting' : 'analysis.coverage.restart') }}
            </Button>
          </div>
        </div>
      </MdDialogFooter>
    </MdDialogContent>
  </Dialog>
</template>

<style scoped>
.coverage-label {
  display: none;
}

/* Keep the compact label usable on older WebViews without container queries. */
@container analysis-visual (min-width: 640px) {
  .coverage-label {
    display: inline;
  }

  .coverage-count {
    display: none;
  }
}

.coverage-header {
  padding-inline-start: var(--layout-dialog-body-inline-padding);
}
</style>
