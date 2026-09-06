<script setup lang="ts">
import { nextTick, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import MdIcon from '@/components/icons/md-icon.vue';
import MdSpinner from '@/components/custom/md-spinner.vue';
import { ICON_NAMES } from '@/lib/models/ui';

const props = defineProps<{ text: string; active: boolean; hasAnswer: boolean }>();
const emit = defineEmits<{ interact: [] }>();
const { t } = useI18n();
const contentId = useId();
const expanded = ref(props.active && !props.hasAnswer);
const manuallyControlled = ref(false);
const follow = ref(true);
const scroller = ref<HTMLElement | null>(null);

function interact() {
  manuallyControlled.value = true;
  // Stop the answer viewport from pulling this section out of view as well.
  emit('interact');
}
function toggle() {
  interact();
  expanded.value = !expanded.value;
}
function scroll() {
  const el = scroller.value;
  if (!el) return;
  follow.value = el.scrollHeight - el.clientHeight - el.scrollTop < 32;
  if (!follow.value) interact();
}
watch(
  () => props.hasAnswer,
  hasAnswer => {
    // Preserve an explicit expansion or text selection when the answer begins.
    if (hasAnswer && !manuallyControlled.value) expanded.value = false;
  }
);
watch(
  () => [props.text, expanded.value],
  async () => {
    if (!expanded.value || !follow.value) return;
    await nextTick();
    const el = scroller.value;
    const selection = window.getSelection();
    if (el && !(selection && !selection.isCollapsed && el.contains(selection.anchorNode))) {
      el.scrollTop = el.scrollHeight;
    }
  }
);
</script>

<template>
  <div class="mb-4 min-w-0 text-xs text-muted-foreground">
    <button
      type="button"
      class="flex w-full items-center gap-2 rounded-sm py-1 text-left transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      :aria-expanded="expanded"
      :aria-label="active ? t('ai.thinking') : t('ai.reasoningDetails')"
      :aria-controls="contentId"
      @click="toggle"
    >
      <MdSpinner v-if="active" size="small" />
      <span role="status" class="flex-1">{{ active ? t('ai.thinking') : t('ai.reasoningDetails') }}</span>
      <MdIcon :name="expanded ? ICON_NAMES.chevronUp : ICON_NAMES.chevronDown" :size="14" />
    </button>
    <!-- Readable provider text is inert and never participates in copy-answer or actions. -->
    <div
      v-show="expanded"
      :id="contentId"
      ref="scroller"
      class="reasoning-content mt-2 max-h-36 overflow-y-auto border-l-2 border-border pl-3 pr-2 leading-6 whitespace-pre-wrap break-words"
      @scroll="scroll"
      @pointerdown="interact"
    >
      {{ text }}
    </div>
  </div>
</template>

<style scoped>
.reasoning-content {
  -webkit-user-select: text;
  user-select: text;
  cursor: text;
}
</style>
