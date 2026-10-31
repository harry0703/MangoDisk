<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';

import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdSplitActionButton from '@/components/custom/md-split-action-button.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ANALYSIS_SCAN_MODES, type AnalysisScanMode } from '@/lib/models/storage/analysis';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import { OperatingSystemService } from '@/lib/services/settings/operating-system-service';

const props = withDefaults(
  defineProps<{
    mode: AnalysisScanMode;
    disabled: boolean;
    rescan?: boolean;
    scanning?: boolean;
    large?: boolean;
  }>(),
  { rescan: false, scanning: false, large: false }
);
const emit = defineEmits<{ scan: [mode: AnalysisScanMode] }>();
const { t } = useI18n({ useScope: 'global' });
const hasScanModes = OperatingSystemService.isWindows();
const label = computed(() =>
  t(props.scanning ? 'loading.currentStage' : props.rescan ? 'analysis.rescan' : 'analysis.start')
);
const items = computed(() =>
  hasScanModes
    ? [
        {
          value: ANALYSIS_SCAN_MODES.standard,
          icon: ICON_NAMES.hardDrive,
          label: t('analysis.scanMode.standard'),
          description: t('analysis.scanMode.standardDescription'),
        },
        {
          value: ANALYSIS_SCAN_MODES.fast,
          icon: ICON_NAMES.search,
          label: t('analysis.scanMode.fast'),
          description: t('analysis.scanMode.fastDescription'),
        },
      ]
    : []
);
function select(value: string) {
  if (hasScanModes && (value === ANALYSIS_SCAN_MODES.standard || value === ANALYSIS_SCAN_MODES.fast))
    emit('scan', value);
}
</script>

<template>
  <MdTooltip :text="label">
    <MdSplitActionButton
      :accessible-label="t('analysis.scanMode.label')"
      :disabled="disabled"
      :items="items"
      :selected-value="mode"
      :primary-icon="rescan ? ICON_NAMES.refresh : ICON_NAMES.analysis"
      :primary-label="label"
      :size="large ? 'lg' : 'default'"
      :variant="rescan ? 'outline' : 'default'"
      @primary="emit('scan', hasScanModes ? mode : ANALYSIS_SCAN_MODES.standard)"
      @select="select"
    >
      <template #primary>
        <MdIcon
          :name="scanning || rescan ? ICON_NAMES.refresh : ICON_NAMES.analysis"
          :class="{ 'icon-spin': scanning }"
          :size="17"
        />
        <span class="labels" :aria-label="label">
          <span :class="{ visible: !scanning && !rescan }">{{ t('analysis.start') }}</span>
          <span :class="{ visible: !scanning && rescan }">{{ t('analysis.rescan') }}</span>
          <span :class="{ visible: scanning }">{{ t('loading.currentStage') }}</span>
        </span>
      </template>
    </MdSplitActionButton>
  </MdTooltip>
</template>

<style scoped>
.labels {
  display: grid;
}
.labels > span {
  visibility: hidden;
  grid-area: 1 / 1;
}
.labels > span.visible {
  visibility: visible;
}
</style>
