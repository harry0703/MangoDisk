<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import MdFloatingPanel from '@/components/custom/md-floating-panel.vue';
import MdAiSettingsDialog from '@/components/custom/md-ai-settings-dialog.vue';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdAiReasoning from '@/components/custom/md-ai-reasoning.vue';
import MdSafeRichText from '@/components/custom/md-safe-rich-text.vue';
import MdCopyButton from '@/components/custom/md-copy-button.vue';
import MdSpinner from '@/components/custom/md-spinner.vue';
import MdAiQuotaStatus from '@/components/custom/md-ai-quota-status.vue';
import { aiQuotaCooldownSeconds, isAiServiceUnavailable } from '@/lib/utils/ai-quota';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';
import { useAiStore } from '@/stores/ai-store';
import { AI_ERROR_LABELS, type AiSubject } from '@/lib/models/ai';

const { t } = useI18n({ useScope: 'global' });
const props = defineProps<{ module: AiSubject['module'] }>();
const aiStore = useAiStore();
const store = computed(() => aiStore.workspaces[props.module]);
const settingsOpen = ref(false);
const scroller = ref<HTMLElement | null>(null);
const follow = ref(true);
const generating = computed(() => store.value.status === 'generating');
const freeMode = computed(() => store.value.settings?.mode === 'free');
const freeUnavailable = computed(
  () =>
    freeMode.value &&
    !generating.value &&
    (isAiServiceUnavailable(aiStore.quota) || store.value.error === 'freeUnavailable')
);
const freeReady = computed(
  () => !freeMode.value || (store.value.settings?.freeConsent && store.value.settings.freeAvailable)
);
const now = ref(performance.now());
let quotaTimer: ReturnType<typeof setInterval> | undefined;
const cooldown = computed(() => aiQuotaCooldownSeconds(aiStore.quota, aiStore.quotaReadAt, now.value));
watch(
  () => store.value.open && freeMode.value,
  active => {
    clearInterval(quotaTimer);
    if (active)
      quotaTimer = setInterval(() => {
        now.value = performance.now();
      }, 1000);
  },
  { immediate: true }
);
function refreshAfterFocus() {
  if (store.value.open && freeMode.value && store.value.settings?.freeAvailable)
    void aiStore.refreshQuota(store.value.language, true);
}
onMounted(() => window.addEventListener('focus', refreshAfterFocus));
onBeforeUnmount(() => {
  clearInterval(quotaTimer);
  window.removeEventListener('focus', refreshAfterFocus);
});
const statusLabel = computed(() =>
  generating.value
    ? store.value.reasoning && !store.value.text
      ? t('ai.thinking')
      : t('ai.generating')
    : store.value.status === 'completed'
      ? t('ai.completed')
      : t('ai.explain')
);
watch(
  () => [store.value.text, store.value.minimized],
  async () => {
    if (!follow.value) return;
    await nextTick();
    if (scroller.value) scroller.value.scrollTop = scroller.value.scrollHeight;
  }
);
watch(
  () => store.value.context,
  () => {
    follow.value = true;
  }
);
function scroll() {
  const el = scroller.value;
  if (el) follow.value = el.scrollHeight - el.clientHeight - el.scrollTop < 40;
}
// The store, not the page lifecycle, owns the stream. Navigation only hides
// this module's panel; explicit close and native result changes cancel it.
</script>

<template>
  <MdFloatingPanel
    :open="store.open"
    :minimized="store.minimized"
    :title="t('ai.explain')"
    :subtitle="store.context?.title"
    :busy="generating"
    :status-label="statusLabel"
    @minimize="aiStore.minimize(module)"
    @restore="aiStore.restore(module)"
    @close="aiStore.close(module)"
  >
    <template #actions>
      <MdIconAction
        variant="ghost"
        :disabled="generating || store.loadingSettings || aiStore.changingConfiguration"
        :label="t('ai.settingsTitle')"
        @click="settingsOpen = true"
        ><MdIcon :name="ICON_NAMES.settings" :size="17"
      /></MdIconAction>
    </template>
    <div ref="scroller" class="min-h-0 flex-1 overflow-y-auto p-4" @scroll="scroll">
      <div v-if="store.loadingSettings" role="status" class="flex items-center gap-2 text-sm text-muted-foreground">
        <MdSpinner />{{ t('ai.loadingSettings') }}
      </div>
      <div v-else-if="freeUnavailable" role="status" class="grid gap-2 py-3 text-sm text-muted-foreground">
        <p class="font-medium text-foreground">{{ t('ai.freeTemporarilyUnavailable') }}</p>
        <p>{{ t('ai.freeUseCustom') }}</p>
        <Button variant="outline" size="sm" class="w-fit" @click="settingsOpen = true">{{ t('ai.configure') }}</Button>
      </div>
      <div v-else-if="freeMode && !freeReady && !store.text" class="grid gap-4 py-3 text-sm text-muted-foreground">
        <template v-if="store.settings?.freeAvailable">
          <p class="font-medium text-foreground">{{ t('ai.freeDescription') }}</p>
          <p class="leading-relaxed">{{ t('ai.freeDisclosure') }}</p>
          <Button
            class="w-fit"
            :disabled="store.loadingSettings || aiStore.changingConfiguration"
            @click="aiStore.acceptFree(module)"
            >{{ t('ai.freeAccept') }}</Button
          >
        </template>
        <template v-else>
          <p>{{ t('ai.freeBuildUnavailable') }}</p>
          <Button class="w-fit" @click="settingsOpen = true">{{ t('ai.configure') }}</Button>
        </template>
      </div>
      <div v-else-if="!store.settings && !store.text" class="grid gap-4 py-3 text-sm text-muted-foreground">
        <p>{{ t('ai.notConfigured') }}</p>
        <p class="leading-relaxed">{{ t('ai.disclosure') }}</p>
        <Button class="w-fit" @click="settingsOpen = true">{{ t('ai.configure') }}</Button>
      </div>
      <MdAiReasoning
        v-if="store.reasoning"
        :key="store.responseVersion"
        :text="store.reasoning"
        :active="generating && !store.text"
        :has-answer="Boolean(store.text)"
        @interact="follow = false"
      />
      <!-- Provider output stays inert: no links, remote images or actions. -->
      <MdSafeRichText
        v-if="store.text"
        class="ai-response"
        :content="store.text"
        variant="answer"
        :allow-links="false"
      />
      <div
        v-if="generating && !store.reasoning && !store.text"
        role="status"
        class="mt-3 flex items-center gap-2 text-sm text-muted-foreground"
      >
        <MdSpinner />{{ t('ai.waiting') }}
      </div>
      <p v-if="store.status === 'cancelled'" role="status" class="mt-3 text-sm text-muted-foreground">
        {{ t(freeMode ? 'ai.freeStopped' : 'ai.stopped') }}
      </p>
      <p v-else-if="store.error && !freeUnavailable" role="alert" class="mt-3 text-sm text-destructive">
        {{ t(AI_ERROR_LABELS[store.error]) }}
      </p>
    </div>
    <template #footer>
      <div class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs leading-relaxed text-muted-foreground">
        <!-- Once an answer starts, keep progress outside the scrolling document.
             The existing footer slot avoids moving the text as chunks arrive. -->
        <p v-if="generating && store.text" role="status" class="flex items-center gap-2">
          <MdSpinner />{{ t('ai.generating') }}
        </p>
        <p v-else>{{ t('ai.disclaimer') }}</p>
        <MdAiQuotaStatus v-if="freeMode && freeReady && aiStore.quota && !freeUnavailable" :quota="aiStore.quota" />
      </div>
      <div class="flex min-h-8 flex-none items-center gap-2">
        <Button v-if="generating" variant="outline" size="sm" @click="aiStore.stop(module)">{{ t('ai.stop') }}</Button>
        <Button
          v-else-if="store.settings && freeReady && (store.status === 'failed' || store.status === 'cancelled')"
          variant="outline"
          size="sm"
          :disabled="store.loadingSettings || aiStore.changingConfiguration || (freeMode && cooldown > 0)"
          @click="aiStore.generate(module)"
          >{{ t('ai.retry') }}</Button
        >
        <MdCopyButton v-if="store.text" :text="store.text" />
      </div>
    </template>
  </MdFloatingPanel>
  <MdAiSettingsDialog
    v-model:open="settingsOpen"
    :quota="aiStore.quota"
    @refresh-quota="aiStore.refreshQuota"
    @configured="aiStore.configurationChanged($event, module)"
  />
</template>
