<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import {
  NumberField,
  NumberFieldContent,
  NumberFieldDecrement,
  NumberFieldIncrement,
  NumberFieldInput,
} from '@/components/ui/number-field';

const props = withDefaults(
  defineProps<{
    id: string;
    label: string;
    min: number;
    max: number;
    step: number;
    stepSnapping?: boolean;
    disabled?: boolean;
    placeholder?: string;
  }>(),
  { stepSnapping: true, disabled: false, placeholder: undefined }
);
const model = defineModel<number | null>({ required: true });
const { t, locale } = useI18n({ useScope: 'global' });

// Reka emits undefined for a cleared field. Keep the application-facing value
// nullable so an empty optional setting never becomes zero or NaN in IPC.
function update(value: number | undefined) {
  model.value = value !== undefined && Number.isFinite(value) ? value : null;
}
</script>

<template>
  <NumberField
    :id="props.id"
    :model-value="model"
    :min="props.min"
    :max="props.max"
    :step="props.step"
    :step-snapping="props.stepSnapping"
    :disabled="props.disabled"
    :locale="locale"
    :format-options="{ useGrouping: false, maximumFractionDigits: 20 }"
    disable-wheel-change
    @update:model-value="update"
  >
    <NumberFieldContent
      class="[&>[data-slot=input]]:has-[[data-slot=increment]]:pr-9 [&>[data-slot=input]]:has-[[data-slot=decrement]]:pl-9"
    >
      <NumberFieldDecrement
        class="top-1 left-1 grid size-7 translate-y-0 cursor-pointer place-items-center rounded-sm p-0 text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/35"
        :aria-label="t('common.decreaseValue', { label: props.label })"
      />
      <NumberFieldInput
        class="min-w-0 shadow-xs focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/30"
        :placeholder="props.placeholder"
      />
      <NumberFieldIncrement
        class="top-1 right-1 grid size-7 translate-y-0 cursor-pointer place-items-center rounded-sm p-0 text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/35"
        :aria-label="t('common.increaseValue', { label: props.label })"
      />
    </NumberFieldContent>
  </NumberField>
</template>
