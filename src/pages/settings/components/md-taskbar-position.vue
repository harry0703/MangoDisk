<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import type { ResidentPreferences } from '@/lib/models/resident/resident';
import MdSettingsRow from '@/components/custom/md-settings-row.vue';
defineProps<{ preferences: ResidentPreferences }>();
const emit = defineEmits<{
  position: [position: ResidentPreferences['taskbarPosition']];
}>();
const { t } = useI18n({ useScope: 'global' });
</script>
<template>
  <MdSettingsRow
    compact
    controls="responsive"
    :title="t('systemStatus.taskbarPosition')"
    description=""
    title-id="taskbar-position-label"
  >
    <div class="segmented-choice" role="radiogroup" aria-labelledby="taskbar-position-label">
      <label v-for="position in ['auto', 'left', 'right'] as const" :key="position">
        <input
          type="radio"
          name="taskbar-position"
          :value="position"
          :checked="preferences.taskbarPosition === position"
          @change="emit('position', position)"
        />
        <span>{{
          t(
            position === 'auto'
              ? 'systemStatus.taskbarAuto'
              : position === 'left'
                ? 'systemStatus.taskbarLeft'
                : 'systemStatus.taskbarRight'
          )
        }}</span>
      </label>
    </div>
  </MdSettingsRow>
</template>

<style scoped>
@reference "@assets/main.css";
.segmented-choice {
  @apply bg-muted rounded-lg;
  display: flex;
  padding: 3px;
  gap: 2px;
  width: 100%;
}
.segmented-choice label {
  position: relative;
  flex: 1;
  min-width: 0;
  cursor: pointer;
}
.segmented-choice input {
  @apply sr-only;
}
.segmented-choice span {
  @apply text-muted-foreground rounded-md;
  display: grid;
  align-items: center;
  min-height: 26px;
  padding: 3px 8px;
  text-align: center;
  overflow-wrap: anywhere;
  line-height: 1.4;
  transition:
    color 150ms,
    background-color 150ms,
    box-shadow 150ms;
}
.segmented-choice label:hover span {
  @apply text-foreground;
}
/* Native radios retain arrow-key navigation and a single Tab stop. A neutral
   selected surface keeps display choices secondary to the dialog's main action. */
.segmented-choice input:checked + span {
  @apply bg-card text-foreground shadow-sm;
}
.segmented-choice input:focus-visible + span {
  outline: 2px solid var(--ring);
  outline-offset: 1px;
}
@media (prefers-reduced-motion: reduce) {
  .segmented-choice span {
    transition: none;
  }
}
</style>
