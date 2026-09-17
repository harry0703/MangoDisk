<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { BackgroundUpdateService } from '@/lib/services/background-update-service';
import { ResidentService } from '@/lib/services/resident-service';
import { LoggerService } from '@/lib/services/logger-service';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';

const { t } = useI18n();
const version = ref<string | null>(null);
const opening = ref(false);
const failed = ref(false);
let disposed = false;
let stop: (() => void) | undefined;
onMounted(() => {
  void BackgroundUpdateService.watch(notice => {
    version.value = notice.version;
  })
    .then(dispose => {
      if (disposed) dispose();
      else stop = dispose;
    })
    .catch(error => LoggerService.warn('app-update', 'update_notice_subscribe_failed', { error }));
});
onBeforeUnmount(() => {
  disposed = true;
  stop?.();
});
async function openUpdate() {
  if (opening.value) return;
  opening.value = true;
  failed.value = false;
  try {
    await ResidentService.openMain('about');
    LoggerService.info('app-update', 'update_notice_opened', { version: version.value, source: 'resource_panel' });
  } catch (error) {
    failed.value = true;
    LoggerService.warn('app-update', 'update_notice_open_failed', { error });
  } finally {
    opening.value = false;
  }
}
</script>

<template>
  <button v-if="version" type="button" class="update-notice" :disabled="opening" @click="openUpdate">
    {{ t(failed ? 'updates.noticeRetry' : 'updates.noticeAvailable') }}
    <MdIcon :name="ICON_NAMES.external" :size="12" aria-hidden="true" />
  </button>
</template>

<style scoped>
@reference "@assets/main.css";
.update-notice {
  @apply text-primary focus-visible:outline-2 focus-visible:outline-ring;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: none;
  margin-left: auto;
  padding: 0 0 4px;
  background: transparent;
  border: 0;
  font-size: 11px;
  cursor: pointer;
}
.update-notice:hover {
  @apply bg-transparent text-primary;
  text-decoration: underline;
}
.update-notice:disabled {
  opacity: 0.6;
  cursor: default;
}
</style>
