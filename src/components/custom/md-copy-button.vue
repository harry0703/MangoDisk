<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';
import { ClipboardService } from '@/lib/services/clipboard-service';
import { LoggerService } from '@/lib/services/logger-service';

const props = defineProps<{ text: string }>();
const { t } = useI18n();
const copied = ref(false);
const pending = ref(false);
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
  <MdIconAction
    variant="ghost"
    :label="t(copied ? 'common.copied' : 'common.copy')"
    :disabled="!text || pending"
    @click="copy"
    ><MdIcon :name="copied ? ICON_NAMES.check : ICON_NAMES.copy" :size="17"
  /></MdIconAction>
</template>
