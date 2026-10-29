<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import { ClipboardService } from '@/lib/services/shell/clipboard-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';

const props = withDefaults(
  defineProps<{
    text: string;
    showText?: boolean;
    label?: string;
    active?: boolean;
    hint?: string;
    truncate?: boolean;
  }>(),
  {
    active: true,
    truncate: false,
    label: undefined,
    hint: undefined,
  }
);
const { t } = useI18n();
const copied = ref(false);
const pending = ref(false);
const hint = computed(() => {
  if (copied.value) return t('common.copied');
  const action = props.hint || t('common.copy');
  return props.truncate ? `${props.text}\n${action}` : action;
});
let timer: ReturnType<typeof setTimeout> | undefined;
let disposed = false;

async function copy() {
  if (!props.text || pending.value) return;
  pending.value = true;
  try {
    // Snapshot the visible text at click time, including a partial stream.
    await ClipboardService.writeText(props.text);
    if (disposed) return;
    copied.value = true;
    clearTimeout(timer);
    timer = setTimeout(() => {
      copied.value = false;
    }, 1000);
  } catch {
    LoggerService.warn('clipboard', 'write_failed');
    if (!disposed) toast.error(t('common.copyFailed'));
  } finally {
    pending.value = false;
  }
}

onBeforeUnmount(() => {
  disposed = true;
  clearTimeout(timer);
});
</script>

<template>
  <MdTooltip v-if="showText" :active="active" :text="hint" multiline>
    <button
      type="button"
      class="copy-value"
      :class="{ copied, 'copy-value--truncate': truncate }"
      :aria-label="`${t(copied ? 'common.copied' : 'common.copy')}: ${label ?? text}`"
      :disabled="!text || pending"
      @click="copy"
    >
      <span>{{ text }}</span>
    </button>
    <template #content>{{ hint }}</template>
  </MdTooltip>
  <MdIconAction
    v-else
    variant="ghost"
    :label="t(copied ? 'common.copied' : 'common.copy')"
    :disabled="!text || pending"
    @click="copy"
    ><MdIcon :name="copied ? ICON_NAMES.check : ICON_NAMES.copy" :size="17"
  /></MdIconAction>
</template>

<style scoped>
@reference "@assets/main.css";
.copy-value {
  display: inline-flex;
  align-items: baseline;
  justify-content: flex-end;
  max-width: 100%;
  text-align: right;
  cursor: pointer;
  border-radius: 3px;
}
.copy-value span {
  min-width: 0;
  overflow-wrap: anywhere;
}
.copy-value--truncate {
  min-width: 0;
}
.copy-value--truncate span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.copy-value:hover,
.copy-value:focus-visible,
.copy-value.copied {
  @apply text-primary-text;
}
.copy-value:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
</style>
