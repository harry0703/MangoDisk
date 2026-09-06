<script setup lang="ts">
import { ref, watch, onBeforeUnmount } from 'vue';
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
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';
import { AiService, AiSession } from '@/lib/services/ai-service';
import { LoggerService } from '@/lib/services/logger-service';
import { aiErrorCode } from '@/lib/utils/ai-error';
import type { AiErrorCode, AiReasoningMode } from '@/lib/models/ai';
import { AI_ERROR_LABELS } from '@/lib/models/ai';

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ 'update:open': [value: boolean]; configured: [testError: AiErrorCode | null] }>();
const { t, locale } = useI18n({ useScope: 'global' });
const configured = ref(false);
const endpoint = ref('');
const model = ref('');
const apiKey = ref('');
const showKey = ref(false);
// Provider-specific reasoning flags are opt-in; new endpoints must work with
// the standard request shape without silently changing existing preferences.
const reasoning = ref<AiReasoningMode>('default');
const busy = ref(false);
const testing = ref(false);
const error = ref<AiErrorCode | null>(null);
let testSession: AiSession | null = null;
let loadRevision = 0;

watch(
  () => props.open,
  async open => {
    const revision = ++loadRevision;
    showKey.value = false;
    if (!open) {
      apiKey.value = '';
      return;
    }
    busy.value = true;
    error.value = null;
    try {
      const settings = await AiService.configuration();
      // Closing during IPC must not repopulate a hidden editor with a secret.
      if (revision !== loadRevision || !props.open) return;
      configured.value = settings !== null;
      endpoint.value = settings?.endpoint ?? '';
      model.value = settings?.model ?? '';
      apiKey.value = settings?.apiKey ?? '';
      reasoning.value = settings?.reasoning ?? 'default';
    } catch (cause) {
      if (revision === loadRevision) error.value = aiErrorCode(cause);
    } finally {
      if (revision === loadRevision) busy.value = false;
    }
  },
  { immediate: true }
);

async function save(test: boolean) {
  if (busy.value) return;
  busy.value = true;
  error.value = null;
  let saved = false;
  try {
    await AiService.save({
      endpoint: endpoint.value,
      model: model.value,
      apiKey: apiKey.value,
      reasoning: reasoning.value,
    });
    configured.value = true;
    showKey.value = false;
    saved = true;
    if (test) {
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
    emit('configured', null);
    apiKey.value = '';
    endpoint.value = '';
    model.value = '';
    reasoning.value = 'default';
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
  ++loadRevision;
  void cancelTest();
  apiKey.value = '';
});
</script>

<template>
  <Dialog :open="open" @update:open="close">
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
        <div class="grid gap-2">
          <label for="ai-endpoint" class="text-sm font-medium">{{ t('ai.endpoint') }}</label>
          <Input
            id="ai-endpoint"
            v-model="endpoint"
            :disabled="busy"
            placeholder="https://api.example.com/v1"
            autocomplete="off"
            spellcheck="false"
          />
          <p class="text-xs text-muted-foreground">{{ t('ai.endpointHint') }}</p>
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
        <p v-if="error" role="alert" class="text-sm text-destructive">{{ t(AI_ERROR_LABELS[error]) }}</p>
      </div>
      <MdDialogFooter class="flex-wrap" align="between">
        <Button v-if="testing" variant="ghost" @click="cancelTest">{{ t('ai.stop') }}</Button>
        <Button v-else variant="ghost" :disabled="busy || (!configured && !error)" @click="remove">{{
          t('ai.deleteConfiguration')
        }}</Button>
        <div class="flex flex-wrap items-center gap-2">
          <!-- Local saves need no flashing spinner; reserve the same space for
               the slower connection test so its indicator never shifts actions. -->
          <span class="grid size-4 place-items-center"><MdSpinner v-if="testing" /></span>
          <Button variant="outline" :disabled="busy || !endpoint.trim() || !model.trim()" @click="save(true)">{{
            t('ai.saveAndTest')
          }}</Button>
          <Button :disabled="busy || !endpoint.trim() || !model.trim()" @click="save(false)">{{ t('ai.save') }}</Button>
        </div>
      </MdDialogFooter>
    </MdDialogContent>
  </Dialog>
</template>
