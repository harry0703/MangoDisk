<script setup lang="ts">
withDefaults(
  defineProps<{
    title: string;
    description: string;
    as?: 'div' | 'button';
    disabled?: boolean;
    compact?: boolean;
    controls?: 'inline' | 'responsive' | 'field';
    labelFor?: string;
    titleId?: string;
    descriptionId?: string;
  }>(),
  {
    as: 'div',
    disabled: false,
    compact: false,
    controls: 'inline',
    labelFor: undefined,
    titleId: undefined,
    descriptionId: undefined,
  }
);
</script>

<template>
  <!-- Only action rows are buttons. Rows containing switches or selects must
       remain containers so nested controls retain their native interactions. -->
  <component
    :is="as"
    class="setting-row"
    :class="[
      `setting-row--${controls}`,
      {
        'action-row': as === 'button',
        'setting-row--compact': compact,
        'setting-row--without-icon': compact && !$slots.icon,
      },
    ]"
    :type="as === 'button' ? 'button' : undefined"
    :disabled="as === 'button' ? disabled : undefined"
  >
    <span v-if="!compact || $slots.icon" class="section-icon" aria-hidden="true"><slot name="icon" /></span>
    <component
      :is="labelFor && !$slots.help ? 'label' : 'span'"
      class="setting-copy"
      :for="!$slots.help ? labelFor : undefined"
    >
      <!-- Help stays outside the label so opening it cannot toggle the control. -->
      <span v-if="$slots.help" class="setting-title">
        <component :is="labelFor ? 'label' : 'span'" :for="labelFor">
          <strong :id="titleId"
            ><slot name="title">{{ title }}</slot></strong
          >
        </component>
        <slot name="help" />
      </span>
      <strong v-else :id="titleId"
        ><slot name="title">{{ title }}</slot></strong
      >
      <small v-if="description" :id="descriptionId">{{ description }}</small>
    </component>
    <span class="setting-controls"><slot /></span>
  </component>
</template>

<style scoped>
@reference "@assets/main.css";
.setting-row {
  display: grid;
  grid-template-columns: 40px minmax(0, 1fr) auto;
  width: 100%;
  min-height: 60px;
  align-items: center;
  gap: 10px;
  border: 0;
  padding: 7px 14px;
  background: transparent;
  text-align: left;
  font: inherit;
  @apply text-card-foreground transition-colors duration-200 hover:bg-muted/50;
}
.setting-row--responsive,
.setting-row--field {
  grid-template-columns: 40px minmax(0, 1fr);
}
.section-icon {
  display: grid;
  width: 34px;
  height: 34px;
  place-items: center;
  @apply text-muted-foreground;
}
.setting-copy {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}
.setting-title {
  display: flex;
  align-items: center;
  gap: 4px;
}
/* Flex removes the inherited inline baseline strut beside the help button. */
.setting-title > label,
.setting-title > span {
  display: flex;
  align-items: center;
}
.setting-copy strong {
  font-size: var(--font-content-primary);
  font-weight: 650;
}
.setting-copy small {
  font-size: var(--font-content-secondary);
  line-height: 1.5;
  overflow-wrap: anywhere;
  @apply text-muted-foreground;
}
.setting-controls {
  display: flex;
  min-width: 0;
  align-items: center;
  justify-content: flex-end;
  gap: 12px;
  font-size: var(--font-content-body);
}
.setting-row--responsive > .setting-controls,
.setting-row--field > .setting-controls {
  grid-column: 2;
}
.setting-row--field > .setting-controls :deep([role='combobox']) {
  width: 100%;
  height: 36px;
}
.action-row {
  cursor: pointer;
}
.action-row:focus-visible {
  position: relative;
  z-index: 1;
  @apply outline-none ring-2 ring-inset ring-ring/50;
}
.action-row:disabled {
  cursor: default;
  opacity: 0.6;
}
.action-row .setting-controls {
  @apply text-muted-foreground transition-colors duration-200;
}
.action-row:hover .setting-controls,
.action-row:focus-visible .setting-controls {
  @apply text-primary;
}
@container settings (min-width: 42rem) {
  .setting-row {
    grid-template-columns: 42px minmax(0, 1fr) auto;
  }
  .setting-row > .setting-controls {
    grid-column: auto;
  }
  .setting-row--field > .setting-controls {
    width: 13.75rem;
  }
}
/* Safari 15.6 needs the same desktop layout without container queries. */
@supports not (container-type: inline-size) {
  @media (min-width: 900px) {
    .setting-row {
      grid-template-columns: 42px minmax(0, 1fr) auto;
    }
    .setting-row > .setting-controls {
      grid-column: auto;
    }
    .setting-row--field > .setting-controls {
      width: 13.75rem;
    }
  }
}
.setting-row--compact {
  grid-template-columns: 24px minmax(0, 1fr) auto;
  min-height: 36px;
  gap: 8px;
  padding: 4px 0;
}
.setting-row--compact:hover {
  background: transparent;
}
.setting-row--compact .section-icon {
  width: 24px;
  height: 24px;
}
.setting-row--compact .setting-copy strong {
  font-size: var(--font-content-body);
  font-weight: 400;
}
.setting-row--compact.setting-row--without-icon {
  grid-template-columns: minmax(0, 1fr) auto;
}
</style>
