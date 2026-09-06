<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import MdIconAction from './md-icon-action.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';

defineProps<{ name: string; disabled?: boolean }>();
const emit = defineEmits<{ explain: [] }>();
const { t } = useI18n({ useScope: 'global' });
</script>

<template>
  <MdIconAction
    class="md-ai-action"
    variant="ghost"
    :label="t('ai.explain')"
    :aria-label="t('ai.explainItem', { name })"
    :disabled="disabled"
    @click.stop="emit('explain')"
  >
    <MdIcon :name="ICON_NAMES.sparkles" :size="17" />
  </MdIconAction>
</template>

<style>
/* The explicit row marker works across slot/style boundaries. Reserve the hit
   target so hovering never moves metrics, switches or disclosure controls. */
.md-ai-hover-row .md-ai-action {
  opacity: 0;
  pointer-events: none;
  transition: opacity 140ms ease;
}
.md-ai-hover-row:hover .md-ai-action,
.md-ai-hover-row:focus-within .md-ai-action {
  opacity: 1;
  pointer-events: auto;
}
@media (hover: none) {
  .md-ai-hover-row .md-ai-action {
    opacity: 1;
    pointer-events: auto;
  }
}
</style>
