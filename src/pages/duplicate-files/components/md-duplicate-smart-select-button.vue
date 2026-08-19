<script setup lang="ts">
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuItemIndicator,
  DropdownMenuLabel,
  DropdownMenuPortal,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuRoot,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from 'reka-ui';
import { useI18n } from 'vue-i18n';

import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { DUPLICATE_KEEPER_RULE_IDS, type DuplicateKeeperRuleId } from '@/lib/models/duplicate-file';
import { ICON_NAMES } from '@/lib/models/ui';
import { FormatUtils } from '@/lib/utils/format';

defineProps<{
  keeperRule: DuplicateKeeperRuleId;
  selectedCount: number;
  disabled: boolean;
  analyzing?: boolean;
}>();

const emit = defineEmits<{
  toggle: [];
  selectRule: [rule: DuplicateKeeperRuleId];
  aiSelect: [];
}>();

const { t } = useI18n({ useScope: 'global' });
const keeperRules = Object.values(DUPLICATE_KEEPER_RULE_IDS);

function selectRule(value: unknown) {
  if (typeof value !== 'string' || !keeperRules.includes(value as DuplicateKeeperRuleId)) return;
  emit('selectRule', value as DuplicateKeeperRuleId);
}
</script>

<template>
  <div
    class="smart-select-split"
    :data-active="selectedCount > 0 || analyzing"
    role="group"
    :aria-label="t('duplicateFiles.smartSelect')"
  >
    <Button
      class="smart-select-primary rounded-r-none shadow-none"
      size="sm"
      variant="ghost"
      type="button"
      :disabled="disabled || analyzing"
      @click="emit('toggle')"
    >
      <MdIcon v-if="analyzing" class="icon-spin" :name="ICON_NAMES.refresh" :size="14" />
      <MdIcon v-else :name="selectedCount ? ICON_NAMES.close : ICON_NAMES.smartSelect" :size="14" />
      <span v-if="analyzing">{{ t('largeFiles.selectionMode.analyzing') }}</span>
      <span v-else>{{ t(selectedCount ? 'duplicateFiles.clearSelection' : 'duplicateFiles.smartSelect') }}</span>
      <small v-if="selectedCount && !analyzing">{{ FormatUtils.integer(selectedCount) }}</small>
    </Button>

    <DropdownMenuRoot>
      <DropdownMenuTrigger as-child>
        <Button
          class="smart-select-trigger rounded-l-none px-0 shadow-none"
          size="sm"
          variant="ghost"
          type="button"
          :disabled="disabled || analyzing"
          :aria-label="t('duplicateFiles.smartSelectMenuLabel')"
        >
          <MdIcon :name="ICON_NAMES.chevronDown" :size="14" />
        </Button>
      </DropdownMenuTrigger>

      <DropdownMenuPortal>
        <DropdownMenuContent
          align="end"
          :side-offset="6"
          class="smart-select-menu z-50 w-72 max-w-[calc(100vw-32px)] overflow-hidden rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-xl data-[state=closed]:animate-out data-[state=open]:animate-in data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95"
        >
          <DropdownMenuLabel class="smart-select-menu-label">
            {{ t('duplicateFiles.smartSelectMenuLabel') }}
          </DropdownMenuLabel>
          <DropdownMenuItem
            class="smart-select-menu-item smart-select-ai-item cursor-pointer text-primary font-medium"
            @select="emit('aiSelect')"
          >
            <span class="smart-select-check" aria-hidden="true">
              <MdIcon :name="ICON_NAMES.smartSelect" :size="15" />
            </span>
            <span>{{ t('duplicateFiles.aiSelect') }}</span>
          </DropdownMenuItem>
          <DropdownMenuSeparator class="mx-1 my-1 h-px bg-border" />
          <DropdownMenuRadioGroup :model-value="keeperRule" @update:model-value="selectRule">
            <DropdownMenuRadioItem v-for="rule in keeperRules" :key="rule" :value="rule" class="smart-select-menu-item">
              <span class="smart-select-check" aria-hidden="true">
                <DropdownMenuItemIndicator>
                  <MdIcon :name="ICON_NAMES.check" :size="15" />
                </DropdownMenuItemIndicator>
              </span>
              <span>{{ t(`duplicateFiles.keeperRuleLabels.${rule}`) }}</span>
            </DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
          <DropdownMenuSeparator class="mx-1 my-1 h-px bg-border" />
          <p class="smart-select-menu-hint">{{ t('duplicateFiles.smartSelectSafetyHint') }}</p>
        </DropdownMenuContent>
      </DropdownMenuPortal>
    </DropdownMenuRoot>
  </div>
</template>

<style scoped>
@reference "@assets/main.css";

.smart-select-split {
  display: inline-flex;
  height: 34px;
  border-radius: var(--radius-sm);
  @apply bg-muted/55 text-foreground;
}

.smart-select-split[data-active='true'] {
  @apply bg-primary/10 text-primary;
}

.smart-select-primary,
.smart-select-trigger {
  height: 34px;
  color: inherit;
  box-shadow: none;
}

.smart-select-primary {
  gap: 7px;
  padding-inline: 10px;
  font-size: var(--font-content-body);
  font-weight: 600;
}

.smart-select-primary small {
  min-width: 16px;
  padding: 2px;
  color: inherit;
  font-size: var(--font-content-meta);
  text-align: center;
}

.smart-select-trigger {
  width: 30px;
}

.smart-select-primary:hover,
.smart-select-trigger:hover {
  color: inherit;
  transform: none;
  @apply bg-accent/70;
}

.smart-select-menu-label {
  padding: 7px 9px 5px;
  color: var(--muted-foreground);
  font-size: var(--font-content-meta);
  font-weight: 600;
}

.smart-select-menu-item {
  @apply focus:bg-accent focus:text-accent-foreground;
  display: flex;
  cursor: default;
  user-select: none;
  align-items: center;
  gap: 8px;
  border-radius: calc(var(--radius) - 2px);
  padding: 8px;
  font-size: var(--font-content-secondary);
  outline: none;
}

.smart-select-ai-item {
  @apply text-primary font-medium hover:bg-primary/10;
}

.smart-select-check {
  display: inline-flex;
  width: 16px;
  flex: none;
  justify-content: center;
  color: var(--primary);
}

.smart-select-menu-hint {
  margin: 0;
  padding: 6px 9px 7px;
  color: var(--muted-foreground);
  font-size: var(--font-content-meta);
  line-height: 1.45;
}
</style>
