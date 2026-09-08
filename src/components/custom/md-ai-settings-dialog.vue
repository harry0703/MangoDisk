<script setup lang="ts">
import { computed, ref, watch, onMounted, onBeforeUnmount } from 'vue';
import { useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Dialog, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdSpinner from '@/components/custom/md-spinner.vue';
import MdNumberField from '@/components/custom/md-number-field.vue';
import MdAiQuotaStatus from '@/components/custom/md-ai-quota-status.vue';
import { isAiServiceUnavailable } from '@/lib/utils/ai-quota';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';
import { AiService, AiSession } from '@/lib/services/ai-service';
import { LoggerService } from '@/lib/services/logger-service';
import { LinkService } from '@/lib/services/link-service';
import { projectWebsiteUrl } from '@/lib/utils/project-website';
import { aiErrorCode } from '@/lib/utils/ai-error';
import type { AiErrorCode, AiReasoningMode, AiServiceMode, AiQuota } from '@/lib/models/ai';
import { AI_ERROR_LABELS } from '@/lib/models/ai';

const props = defineProps<{ open: boolean; quota?: AiQuota | null }>();
const emit = defineEmits<{
  'update:open': [value: boolean];
  configured: [testError: AiErrorCode | null];
  refreshQuota: [language: string, force: boolean];
}>();
const { t, locale } = useI18n({ useScope: 'global' });
const guideUrl = computed(() => projectWebsiteUrl(locale.value, '/docs/ai#custom-service'));

async function openGuide() {
  try {
    await LinkService.open(guideUrl.value);
  } catch {
    // Do not include the editor draft or native error: it may contain private configuration.
    LoggerService.warn('ai', 'configuration_guide_open_failed');
    toast.error(t('ai.guideOpenFailed'));
  }
}
const configured = ref(false);
const mode = ref<AiServiceMode>('free');
const freeAvailable = ref(false);
const freeConsent = ref(false);
const endpoint = ref('');
const model = ref('');
const apiKey = ref('');
const showKey = ref(false);
// Provider-specific reasoning flags are opt-in; new endpoints must work with
// the standard request shape without silently changing existing preferences.
const reasoning = ref<AiReasoningMode>('default');
const temperature = ref<number | null>(null);
const maxTokens = ref<number | null>(null);
const advanced = ref(false);
const busy = ref(false);
const loading = ref(false);
const loaded = ref(false);
const showFreeQuota = computed(() => props.open && loaded.value && mode.value === 'free' && freeAvailable.value);
// Quota reads are independent of editor initialization: neither a slow network
// nor a failed refresh may delay the entrance animation or resize the form.
watch(showFreeQuota, active => {
  if (active) emit('refreshQuota', locale.value, false);
});
function refreshQuotaAfterFocus() {
  if (showFreeQuota.value) emit('refreshQuota', locale.value, true);
}
onMounted(() => window.addEventListener('focus', refreshQuotaAfterFocus));
const testing = ref(false);
const error = ref<AiErrorCode | null>(null);
let testSession: AiSession | null = null;
let loadRevision = 0;
const canSave = computed(
  () =>
    loaded.value &&
    !loading.value &&
    (temperature.value === null ||
      (Number.isFinite(Number(temperature.value)) &&
        Number(temperature.value) >= 0 &&
        Number(temperature.value) <= 2)) &&
    (maxTokens.value === null ||
      (Number.isSafeInteger(Number(maxTokens.value)) &&
        Number(maxTokens.value) > 0 &&
        Number(maxTokens.value) <= 4294967295)) &&
    (mode.value === 'free' || (Boolean(endpoint.value.trim()) && Boolean(model.value.trim())))
);

watch(
  () => props.open,
  async open => {
    const revision = ++loadRevision;
    showKey.value = false;
    if (!open) {
      apiKey.value = '';
      return;
    }
    loading.value = true;
    advanced.value = false;
    loaded.value = false;
    error.value = null;
    const started = performance.now();
    try {
      const { configuration: settings, freeAvailable: available } = await AiService.editorState();
      // Closing during IPC must not repopulate a hidden editor with a secret.
      if (revision !== loadRevision || !props.open) return;
      configured.value = settings !== null;
      mode.value = settings?.mode ?? 'free';
      freeConsent.value = settings?.freeConsent ?? false;
      freeAvailable.value = available;
      endpoint.value = settings?.endpoint ?? '';
      model.value = settings?.model ?? '';
      apiKey.value = settings?.apiKey ?? '';
      reasoning.value = settings?.reasoning ?? 'default';
      temperature.value = settings?.temperature ?? null;
      maxTokens.value = settings?.maxTokens ?? null;
      loaded.value = true;
      LoggerService.info('ai', 'settings_editor_ready', { durationMs: Math.round(performance.now() - started) });
    } catch (cause) {
      if (revision === loadRevision) error.value = aiErrorCode(cause);
    } finally {
      if (revision === loadRevision) loading.value = false;
    }
  },
  { immediate: true }
);

async function save(test: boolean) {
  if (busy.value || !canSave.value) return;
  busy.value = true;
  error.value = null;
  let saved = false;
  try {
    await AiService.save({
      mode: mode.value,
      freeConsent: freeConsent.value,
      endpoint: endpoint.value,
      model: model.value,
      apiKey: apiKey.value,
      reasoning: reasoning.value,
      temperature: temperature.value,
      maxTokens: maxTokens.value,
    });
    configured.value = true;
    showKey.value = false;
    saved = true;
    if (test && mode.value === 'custom') {
      testing.value = true;
      testSession = new AiSession();
      await testSession.run(null, locale.value, () => undefined);
    }
    // Success feedback must not add a form row and move the dialog or its
    // actions. Report test success only after streaming finishes successfully.
    toast.success(t(test ? 'ai.connected' : 'ai.saved'));
  } catch (cause) {
    error.value = aiErrorCode(cause);
  } finally {
    busy.value = false;
    testSession = null;
    testing.value = false;
    // Publish after the connection-test session releases its reservation;
    // an open explanation panel can then start automatically without Busy.
    if (saved) emit('configured', error.value);
  }
}

async function remove() {
  busy.value = true;
  error.value = null;
  try {
    await AiService.delete();
    configured.value = false;
    loaded.value = true;
    mode.value = 'free';
    freeConsent.value = false;
    emit('configured', null);
    apiKey.value = '';
    endpoint.value = '';
    model.value = '';
    reasoning.value = 'default';
    temperature.value = null;
    maxTokens.value = null;
    advanced.value = false;
    showKey.value = false;
    toast.success(t('ai.deleted'));
  } catch (cause) {
    error.value = aiErrorCode(cause);
  } finally {
    busy.value = false;
  }
}

function close(open: boolean) {
  if (busy.value && !testing.value) return;
  if (!open) void cancelTest();
  emit('update:open', open);
}
async function cancelTest() {
  try {
    await testSession?.cancel();
  } catch {
    LoggerService.warn('ai', 'connection_test_cancel_failed');
  }
}
onBeforeUnmount(() => {
  window.removeEventListener('focus', refreshQuotaAfterFocus);
  ++loadRevision;
  void cancelTest();
  apiKey.value = '';
});
</script>

<template>
  <!-- Start the entrance only with the final form, not a shorter loading shell.
       Resizing a centered dialog during its animation changes its origin. -->
  <Dialog :open="open && !loading" @update:open="close">
    <!-- close() guards busy actions; keep the header control mounted to avoid
         a visible flash during quick local saves. -->
    <MdDialogContent
      class="flex min-h-0 flex-col"
      @interact-outside.prevent
      @escape-key-down="busy && !testing && $event.preventDefault()"
    >
      <MdDialogHeader class="flex-none border-b border-border/70">
        <DialogTitle>{{ t('ai.settingsTitle') }}</DialogTitle>
        <DialogDescription>{{ t('ai.settingsDescription') }}</DialogDescription>
      </MdDialogHeader>
      <div class="grid min-h-0 gap-4 overflow-y-auto p-5">
        <!-- Native radios communicate a persistent choice and provide arrow-key
             navigation. Selection edits the draft; only Save changes the service. -->
        <fieldset v-if="loaded" class="grid min-w-0 gap-3" :disabled="busy">
          <legend class="sr-only">{{ t('ai.serviceMode') }}</legend>
          <div
            class="min-w-0 rounded-xl border transition-colors"
            :class="mode === 'free' ? 'border-primary/60 bg-accent/30' : 'border-border/70'"
          >
            <label class="flex cursor-pointer items-start gap-3 p-4" :class="{ 'cursor-default': busy }">
              <input
                v-model="mode"
                type="radio"
                name="ai-service-mode"
                value="free"
                class="peer sr-only"
                aria-labelledby="ai-mode-free-title"
              />
              <!-- WebKit can clip the native radio artwork inside a fixed-size
                   input. Keep native semantics and draw only the visible circle. -->
              <span
                aria-hidden="true"
                class="mt-0.5 grid size-4 shrink-0 place-items-center rounded-full border border-muted-foreground/60 bg-background peer-checked:border-primary peer-checked:bg-primary peer-focus-visible:ring-2 peer-focus-visible:ring-ring/35 peer-focus-visible:ring-offset-2 peer-focus-visible:ring-offset-background peer-disabled:opacity-50"
              >
                <span
                  class="size-1.5 rounded-full bg-primary-foreground"
                  :class="mode === 'free' ? 'opacity-100' : 'opacity-0'"
                />
              </span>
              <span class="grid min-w-0 gap-1">
                <span id="ai-mode-free-title" class="text-sm font-medium">{{ t('ai.freeService') }}</span>
                <span class="text-sm leading-relaxed text-muted-foreground">{{ t('ai.freeNoKey') }}</span>
              </span>
            </label>
            <div
              v-if="mode === 'free'"
              class="grid gap-2 border-t border-border/70 p-4 text-sm leading-relaxed text-muted-foreground"
            >
              <MdAiQuotaStatus :quota="freeAvailable ? quota : null" />
              <p v-if="freeAvailable && isAiServiceUnavailable(quota)">{{ t('ai.freeUseCustom') }}</p>
              <p v-if="!freeAvailable" class="text-xs">{{ t('ai.freeBuildUnavailable') }}</p>
            </div>
          </div>
          <div
            class="min-w-0 rounded-xl border transition-colors"
            :class="mode === 'custom' ? 'border-primary/60 bg-accent/30' : 'border-border/70'"
          >
            <!-- Keep the help link outside the radio label so opening help never changes the draft mode. -->
            <div class="relative">
              <label class="flex cursor-pointer items-start gap-3 p-4" :class="{ 'cursor-default': busy }">
                <input
                  v-model="mode"
                  type="radio"
                  name="ai-service-mode"
                  value="custom"
                  class="peer sr-only"
                  aria-labelledby="ai-mode-custom-title"
                />
                <span
                  aria-hidden="true"
                  class="mt-0.5 grid size-4 shrink-0 place-items-center rounded-full border border-muted-foreground/60 bg-background peer-checked:border-primary peer-checked:bg-primary peer-focus-visible:ring-2 peer-focus-visible:ring-ring/35 peer-focus-visible:ring-offset-2 peer-focus-visible:ring-offset-background peer-disabled:opacity-50"
                >
                  <span
                    class="size-1.5 rounded-full bg-primary-foreground"
                    :class="mode === 'custom' ? 'opacity-100' : 'opacity-0'"
                  />
                </span>
                <span class="grid min-w-0 flex-1 gap-1">
                  <span id="ai-mode-custom-title" class="pr-32 text-sm font-medium">{{ t('ai.customService') }}</span>
                  <span class="text-sm leading-relaxed text-muted-foreground">{{ t('ai.customDescription') }}</span>
                </span>
              </label>
              <a
                :href="guideUrl"
                target="_blank"
                rel="noopener noreferrer"
                class="absolute top-4 right-4 inline-flex items-center gap-1 rounded-sm text-xs leading-5 text-muted-foreground underline-offset-4 transition-colors hover:text-foreground hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/35"
                :title="t('ai.guideOpenInBrowser')"
                @click.prevent="openGuide"
              >
                {{ t('ai.configurationGuide') }}
                <MdIcon :name="ICON_NAMES.external" :size="13" aria-hidden="true" />
              </a>
            </div>
            <div v-if="mode === 'custom'" class="grid min-w-0 gap-4 border-t border-border/70 p-4">
              <div class="grid gap-2">
                <div class="flex flex-wrap items-baseline gap-x-2 gap-y-1">
                  <label for="ai-endpoint" class="text-sm font-medium">{{ t('ai.endpoint') }}</label>
                  <span id="ai-endpoint-hint" class="text-xs text-muted-foreground">{{ t('ai.endpointHint') }}</span>
                </div>
                <Input
                  id="ai-endpoint"
                  v-model="endpoint"
                  aria-describedby="ai-endpoint-hint"
                  :disabled="busy"
                  placeholder="https://api.example.com/v1"
                  autocomplete="off"
                  spellcheck="false"
                />
              </div>
              <div class="grid gap-2">
                <label for="ai-model" class="text-sm font-medium">{{ t('ai.model') }}</label>
                <Input
                  id="ai-model"
                  v-model="model"
                  :disabled="busy"
                  placeholder="deepseek/deepseek-v4-flash"
                  autocomplete="off"
                  spellcheck="false"
                />
              </div>
              <div class="grid gap-2">
                <label for="ai-key" class="text-sm font-medium">{{ t('ai.apiKey') }}</label>
                <div class="relative">
                  <Input
                    id="ai-key"
                    v-model="apiKey"
                    class="pr-11"
                    :disabled="busy"
                    :type="showKey ? 'text' : 'password'"
                    :placeholder="t('ai.keyPlaceholder')"
                    autocomplete="new-password"
                    spellcheck="false"
                  />
                  <button
                    type="button"
                    class="absolute inset-y-1 right-1 grid w-8 cursor-pointer place-items-center rounded-md text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/35 disabled:cursor-not-allowed disabled:opacity-50"
                    :disabled="busy"
                    :aria-label="t(showKey ? 'ai.hideKey' : 'ai.showKey')"
                    :aria-pressed="showKey"
                    @click="showKey = !showKey"
                  >
                    <MdIcon :name="showKey ? ICON_NAMES.eyeOff : ICON_NAMES.eye" :size="17" />
                  </button>
                </div>
              </div>
              <button
                type="button"
                class="flex items-center gap-2 rounded-md py-1 text-sm text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/35"
                :aria-expanded="advanced"
                aria-controls="ai-advanced-settings"
                :disabled="busy"
                @click="advanced = !advanced"
              >
                <MdIcon :name="advanced ? ICON_NAMES.chevronUp : ICON_NAMES.chevronDown" :size="16" />
                {{ t('ai.advancedSettings') }}
              </button>
              <div v-if="advanced" id="ai-advanced-settings" class="grid gap-4">
                <div class="grid gap-2">
                  <label for="ai-reasoning" class="text-sm font-medium">{{ t('ai.reasoning') }}</label>
                  <Select v-model="reasoning" :disabled="busy">
                    <SelectTrigger id="ai-reasoning" class="w-full"
                      ><SelectValue>{{
                        t(reasoning === 'default' ? 'ai.reasoningDefault' : 'ai.reasoningDisabled')
                      }}</SelectValue></SelectTrigger
                    >
                    <SelectContent>
                      <SelectItem value="default">{{ t('ai.reasoningDefault') }}</SelectItem>
                      <SelectItem value="disabled">{{ t('ai.reasoningDisabled') }}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div class="grid grid-cols-2 gap-3">
                  <div class="grid gap-2">
                    <label for="ai-temperature" class="text-sm font-medium">{{ t('ai.temperature') }}</label>
                    <MdNumberField
                      id="ai-temperature"
                      v-model="temperature"
                      :label="t('ai.temperature')"
                      :min="0"
                      :max="2"
                      :step="0.1"
                      :step-snapping="false"
                      :placeholder="t('ai.providerDefault')"
                      :disabled="busy"
                    />
                  </div>
                  <div class="grid gap-2">
                    <label for="ai-max-tokens" class="text-sm font-medium">{{ t('ai.maxTokens') }}</label>
                    <MdNumberField
                      id="ai-max-tokens"
                      v-model="maxTokens"
                      :label="t('ai.maxTokens')"
                      :min="1"
                      :max="4294967295"
                      :step="1"
                      :placeholder="t('ai.providerDefault')"
                      :disabled="busy"
                    />
                  </div>
                </div>
              </div>
            </div>
          </div>
        </fieldset>
        <p v-if="error" role="alert" class="text-sm text-destructive">{{ t(AI_ERROR_LABELS[error]) }}</p>
      </div>
      <MdDialogFooter class="flex-wrap" align="between">
        <Button v-if="testing" variant="ghost" @click="cancelTest">{{ t('ai.stop') }}</Button>
        <Button v-else variant="ghost" :disabled="loading || busy || (!configured && !error)" @click="remove">{{
          t('ai.deleteConfiguration')
        }}</Button>
        <div class="flex flex-wrap items-center gap-2">
          <!-- Local saves need no flashing spinner; reserve the same space for
               the slower connection test so its indicator never shifts actions. -->
          <span class="grid size-4 place-items-center"><MdSpinner v-if="testing" /></span>
          <Button v-if="mode === 'custom'" variant="outline" :disabled="busy || !canSave" @click="save(true)">{{
            t('ai.saveAndTest')
          }}</Button>
          <Button :disabled="busy || !canSave" @click="save(false)">{{ t('ai.save') }}</Button>
        </div>
      </MdDialogFooter>
    </MdDialogContent>
  </Dialog>
</template>
