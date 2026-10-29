<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import MdSettingsRow from '@/components/custom/md-settings-row.vue';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import type { TaskbarMonitor } from '@/lib/models/resident/taskbar-monitors';
import { TaskbarMonitorService } from '@/lib/services/resident/taskbar-monitor-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';

const props = defineProps<{ value: string | null }>();
const emit = defineEmits<{ change: [id: string | null] }>();
const { t } = useI18n();
const monitors = ref<TaskbarMonitor[]>([]);
const loaded = ref(false);
const failed = ref(false);
const identifying = ref(false);
const failedAction = ref<'identify' | 'settings' | null>(null);
const selection = computed(() => monitors.value.find(monitor => monitor.id === props.value));
const unavailable = computed(() => loaded.value && props.value !== null && !selection.value);
const noTaskbar = computed(() => selection.value?.hasTaskbar === false);
const selectedLabel = computed(() =>
  props.value === null
    ? t('systemStatus.taskbarMonitorPrimary')
    : selection.value
      ? label(selection.value)
      : t(loaded.value ? 'systemStatus.taskbarMonitorDisconnected' : 'systemStatus.taskbarMonitor')
);
function label(monitor: TaskbarMonitor): string {
  return t('systemStatus.taskbarMonitorName', { number: monitor.number, name: monitor.name });
}
function change(value: unknown) {
  if (typeof value === 'string' && (value === 'primary' || monitors.value.some(monitor => monitor.id === value)))
    emit('change', value === 'primary' ? null : value);
}
let disposed = false;
let loading = false;
let timer: ReturnType<typeof setInterval> | undefined;
let identifyTimer: ReturnType<typeof setTimeout> | undefined;
async function load() {
  if (loading) return;
  loading = true;
  try {
    const value = await TaskbarMonitorService.catalogue();
    if (value.schemaVersion !== 1) throw new Error('unsupported taskbar monitor catalogue');
    if (disposed) return;
    monitors.value = value.monitors;
    loaded.value = true;
    failed.value = false;
  } catch (error) {
    if (!disposed) {
      if (!failed.value) LoggerService.warn('resident', 'taskbar_monitors_load_failed', { error });
      failed.value = true;
    }
  } finally {
    loading = false;
  }
}
async function action(kind: 'identify' | 'settings') {
  failedAction.value = null;
  if (kind === 'identify') {
    clearTimeout(identifyTimer);
    identifying.value = true;
  }
  try {
    if (kind === 'identify') await TaskbarMonitorService.identify();
    else await TaskbarMonitorService.openSettings();
  } catch (error) {
    if (!disposed) {
      failedAction.value = kind;
      LoggerService.warn('resident', 'taskbar_monitor_action_failed', { kind, error });
    }
  } finally {
    if (!disposed && kind === 'identify') identifyTimer = setTimeout(() => (identifying.value = false), 3000);
  }
}
onMounted(() => {
  void load();
  timer = setInterval(() => void load(), 3000);
});
onBeforeUnmount(() => {
  disposed = true;
  clearInterval(timer);
  clearTimeout(identifyTimer);
});
</script>

<template>
  <MdSettingsRow
    compact
    controls="responsive"
    :title="t('systemStatus.taskbarMonitor')"
    description=""
    label-for="taskbar-monitor"
  >
    <div class="monitor-controls">
      <Select :model-value="value ?? 'primary'" :disabled="!loaded" @update:model-value="change">
        <SelectTrigger id="taskbar-monitor" class="min-w-0 flex-1">
          <SelectValue class="truncate">{{ selectedLabel }}</SelectValue>
        </SelectTrigger>
        <SelectContent class="max-w-(--reka-select-content-available-width) [overflow-wrap:anywhere]">
          <SelectItem value="primary">{{ t('systemStatus.taskbarMonitorPrimary') }}</SelectItem>
          <SelectItem v-if="unavailable && value" :value="value" disabled>
            {{ t('systemStatus.taskbarMonitorDisconnected') }}
          </SelectItem>
          <SelectItem v-for="monitor in monitors" :key="monitor.id" :value="monitor.id">
            {{ label(monitor) }}
          </SelectItem>
        </SelectContent>
      </Select>
      <button type="button" class="monitor-action" :disabled="!loaded || identifying" @click="action('identify')">
        {{ t('systemStatus.taskbarMonitorIdentify') }}
      </button>
    </div>
  </MdSettingsRow>
  <p v-if="unavailable || noTaskbar" class="monitor-hint" role="status">
    {{ t(unavailable ? 'systemStatus.taskbarMonitorFallback' : 'systemStatus.taskbarMonitorNoTaskbar') }}
    <button v-if="noTaskbar" type="button" class="monitor-link" @click="action('settings')">
      {{ t('systemStatus.taskbarMonitorSettings') }}
    </button>
  </p>
  <p v-if="failed || failedAction" class="monitor-hint" role="status">
    {{ t('systemStatus.taskbarMonitorFailed') }}
    <button type="button" class="monitor-link" @click="failedAction ? action(failedAction) : load()">
      {{ t('monitoring.refresh') }}
    </button>
  </p>
</template>

<style scoped>
@reference "@assets/main.css";
.monitor-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  width: 100%;
}
.monitor-action {
  @apply rounded-md px-2 text-muted-foreground whitespace-nowrap;
  min-height: 32px;
  flex: none;
  font: inherit;
}
.monitor-action:hover:not(:disabled) {
  @apply bg-muted text-foreground;
}
.monitor-action:disabled {
  @apply opacity-50;
}
.monitor-action:focus-visible,
.monitor-link:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
.monitor-hint {
  @apply text-xs text-muted-foreground;
}
.monitor-link {
  @apply text-primary-text underline;
}
</style>
