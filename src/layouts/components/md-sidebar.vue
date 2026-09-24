<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import MdIcon from '@/components/icons/md-icon.vue';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { APP_NAME, primaryNavGroupsForPlatform, SECONDARY_NAV_ITEMS } from '@/lib/models/application-shell';
import type { PageId } from '@/lib/models/application-shell';
import { ICON_NAMES } from '@/lib/models/ui';

const { t } = useI18n({ useScope: 'global' });

const props = withDefaults(
  defineProps<{
    currentPage: PageId;
    busyPages: PageId[];
    noticePages: PageId[];
    platform: string;
    expanded?: boolean;
  }>(),
  {
    expanded: false,
  }
);
const emit = defineEmits<{
  navigate: [page: PageId];
  toggle: [];
}>();
const primaryNavGroups = computed(() => primaryNavGroupsForPlatform(props.platform));
const openTooltipPage = ref<PageId | null>(null);

function updateTooltip(page: PageId, open: boolean) {
  if (props.expanded) {
    openTooltipPage.value = null;
    return;
  }
  if (open) {
    openTooltipPage.value = page;
  } else if (openTooltipPage.value === page) {
    openTooltipPage.value = null;
  }
}

function navigate(page: PageId) {
  openTooltipPage.value = null;
  emit('navigate', page);
}

function isBusy(page: PageId): boolean {
  return props.busyPages.includes(page);
}

watch(
  [() => props.expanded, () => props.currentPage],
  () => {
    openTooltipPage.value = null;
  },
  { flush: 'sync' }
);
</script>

<template>
  <aside class="sidebar" :class="{ expanded }">
    <nav class="nav-list" :aria-label="APP_NAME">
      <div
        v-for="group in primaryNavGroups"
        :key="group.id"
        class="nav-group"
        role="group"
        :aria-label="t(group.titleKey)"
      >
        <span class="nav-group-label" aria-hidden="true">{{ t(group.titleKey) }}</span>
        <div class="nav-group-items">
          <Tooltip
            v-for="item in group.items"
            :key="item.id"
            :disabled="expanded"
            :open="!expanded && openTooltipPage === item.id"
            @update:open="updateTooltip(item.id, $event)"
          >
            <TooltipTrigger as-child>
              <button
                type="button"
                :aria-label="t(`navigation.${item.id}`)"
                :aria-current="currentPage === item.id ? 'page' : undefined"
                :aria-busy="isBusy(item.id)"
                class="nav-item"
                :class="{ active: currentPage === item.id }"
                @click="navigate(item.id)"
              >
                <span class="nav-icon" aria-hidden="true">
                  <MdIcon :name="item.icon" />
                  <span v-if="!expanded && isBusy(item.id)" class="nav-icon-status md-operational-motion" />
                </span>
                <span class="nav-label" aria-hidden="true">{{ t(`navigation.${item.id}`) }}</span>
                <span class="nav-accessory">
                  <span
                    v-if="expanded && isBusy(item.id)"
                    class="nav-status md-operational-motion"
                    aria-hidden="true"
                  />
                </span>
              </button>
            </TooltipTrigger>
            <TooltipContent v-if="!expanded" side="right" :side-offset="10">
              {{ t(`navigation.${item.id}`) }}
            </TooltipContent>
          </Tooltip>
        </div>
      </div>
    </nav>

    <div class="sidebar-footer">
      <Tooltip
        v-for="item in SECONDARY_NAV_ITEMS"
        :key="item.id"
        :disabled="expanded"
        :open="!expanded && openTooltipPage === item.id"
        @update:open="updateTooltip(item.id, $event)"
      >
        <TooltipTrigger as-child>
          <button
            type="button"
            :aria-label="t(`navigation.${item.id}`)"
            :aria-current="currentPage === item.id ? 'page' : undefined"
            :aria-busy="isBusy(item.id)"
            class="nav-item"
            :class="{ active: currentPage === item.id }"
            @click="navigate(item.id)"
          >
            <span class="nav-icon" aria-hidden="true">
              <MdIcon :name="item.icon" />
              <span v-if="!expanded && isBusy(item.id)" class="nav-icon-status md-operational-motion" />
            </span>
            <span class="nav-label" aria-hidden="true">{{ t(`navigation.${item.id}`) }}</span>
            <span class="nav-accessory">
              <span v-if="expanded && isBusy(item.id)" class="nav-status md-operational-motion" aria-hidden="true" />
              <span
                v-else-if="!isBusy(item.id) && noticePages.includes(item.id)"
                class="nav-notice"
                :aria-label="t('updates.navigationNotice')"
              />
            </span>
          </button>
        </TooltipTrigger>
        <TooltipContent v-if="!expanded" side="right" :side-offset="10">
          {{ t(`navigation.${item.id}`) }}
        </TooltipContent>
      </Tooltip>

      <div class="sidebar-toggle-block">
        <Tooltip :disabled="expanded">
          <TooltipTrigger as-child>
            <button
              type="button"
              class="nav-item sidebar-toggle"
              :aria-label="t(expanded ? 'common.collapseSidebar' : 'common.expandSidebar')"
              :aria-expanded="expanded"
              @click="emit('toggle')"
            >
              <span class="nav-icon" aria-hidden="true">
                <MdIcon
                  :name="expanded ? ICON_NAMES.sidebarCollapse : ICON_NAMES.sidebarExpand"
                  :size="18"
                  :stroke-width="1.7"
                />
              </span>
              <span class="nav-label" aria-hidden="true">{{ t('common.collapseSidebar') }}</span>
            </button>
          </TooltipTrigger>
          <TooltipContent v-if="!expanded" side="right" :side-offset="10">
            {{ t('common.expandSidebar') }}
          </TooltipContent>
        </Tooltip>
      </div>
    </div>
  </aside>
</template>

<style scoped>
@reference "@assets/main.css";
.sidebar {
  display: flex;
  width: var(--sidebar-width, 256px);
  min-width: var(--sidebar-width, 256px);
  height: 100vh;
  flex-direction: column;
  transition:
    width var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease),
    min-width var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease);
  @apply bg-transparent text-sidebar-foreground;
}
.nav-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding-inline: 8px;
  padding-block: 10px 4px;
  transition:
    gap var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease),
    padding var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease);
}
.nav-group {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 0;
  transition: gap var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease);
}
.nav-group-items {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}
.nav-group-label {
  max-height: 0;
  padding-inline: 12px;
  overflow: hidden;
  opacity: 0;
  color: var(--sidebar-foreground);
  color: color-mix(in oklab, var(--sidebar-foreground) 58%, transparent);
  font-size: 11px;
  font-weight: 600;
  line-height: 20px;
  letter-spacing: 0.04em;
  white-space: nowrap;
  visibility: hidden;
  transform: translateY(-3px);
  transition:
    max-height var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease),
    opacity 100ms ease,
    transform 180ms ease,
    visibility 0s linear var(--sidebar-transition-duration, 240ms);
}
.nav-item {
  position: relative;
  display: flex;
  width: 100%;
  height: var(--layout-sidebar-item-height);
  align-items: center;
  justify-content: flex-start;
  gap: 0;
  border: 0;
  border-radius: 8px;
  padding: 0 14px;
  background: transparent;
  color: inherit;
  font: inherit;
  font-size: 14px;
  cursor: pointer;
  transition:
    background-color 0.16s ease,
    color 0.16s ease,
    box-shadow 0.16s ease,
    gap var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease),
    padding-inline var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease);
}
.sidebar.expanded .nav-list,
.sidebar.expanded .sidebar-footer {
  padding-inline: 10px;
}
.sidebar.expanded .nav-list {
  gap: 12px;
  padding-block: 10px 6px;
}
.sidebar.expanded .nav-group {
  gap: 4px;
}
.sidebar.expanded .nav-group-label {
  max-height: 20px;
  opacity: 1;
  visibility: visible;
  transform: translateY(0);
  transition-delay: 0s, 70ms, 70ms, 0s;
}
.sidebar.expanded .nav-item {
  gap: 12px;
  padding-inline: 12px;
}
.nav-item:hover:not(.active) {
  background: var(--sidebar-accent);
  background: color-mix(in oklab, var(--sidebar-accent) 52%, transparent);
  color: var(--sidebar-foreground);
}
.nav-item:active:not(.active) {
  background: var(--sidebar-accent);
  background: color-mix(in oklab, var(--sidebar-accent) 72%, transparent);
}
.nav-item.active {
  @apply bg-sidebar-accent text-sidebar-accent-foreground;
  font-weight: 600;
}
.nav-item.active::before {
  position: absolute;
  left: 0;
  width: 3px;
  height: 24px;
  border-radius: 999px;
  @apply bg-primary;
  content: '';
}
.nav-item:focus-visible {
  outline: none;
  box-shadow: inset 0 0 0 2px var(--focus-ring-subtle);
  box-shadow: inset 0 0 0 2px color-mix(in oklab, var(--primary) 32%, transparent);
}
.nav-icon {
  position: relative;
  display: grid;
  width: 24px;
  height: 24px;
  flex: none;
  place-items: center;
  color: currentColor;
  font-size: 22px;
  line-height: 1;
}
.nav-icon-status {
  position: absolute;
  inset: -4px;
  border: 1.5px solid var(--border-primary-subtle);
  border: 1.5px solid color-mix(in oklab, var(--primary) 16%, transparent);
  border-top-color: var(--primary);
  border-top-color: color-mix(in oklab, var(--primary) 88%, transparent);
  border-right-color: var(--primary);
  border-right-color: color-mix(in oklab, var(--primary) 46%, transparent);
  border-radius: 50%;
  pointer-events: none;
  animation: nav-spin 0.9s linear infinite;
}
.nav-label {
  max-width: 0;
  min-width: 0;
  overflow: hidden;
  opacity: 0;
  text-overflow: ellipsis;
  white-space: nowrap;
  visibility: hidden;
  transform: translateX(-4px);
  transition:
    max-width var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease),
    opacity 100ms ease,
    transform 180ms ease,
    visibility 0s linear var(--sidebar-transition-duration, 240ms);
}
.sidebar.expanded .nav-label {
  max-width: 156px;
  opacity: 1;
  visibility: visible;
  transform: translateX(0);
  transition-delay: 0s, 60ms, 60ms, 0s;
}
.nav-accessory {
  position: absolute;
  top: 8px;
  right: 8px;
  display: grid;
  width: 12px;
  height: 12px;
  flex: none;
  place-items: center;
}
.sidebar.expanded .nav-accessory {
  position: static;
  margin-left: auto;
}
.nav-status {
  width: 11px;
  height: 11px;
  aspect-ratio: 1;
  border: 1.5px solid var(--border-primary-subtle);
  border: 1.5px solid color-mix(in oklab, var(--primary) 20%, transparent);
  border-top-color: var(--primary);
  border-top-color: color-mix(in oklab, var(--primary) 78%, transparent);
  border-radius: 50%;
  animation: nav-spin 0.75s linear infinite;
}
.nav-notice {
  width: 8px;
  height: 8px;
  flex: none;
  border-radius: 50%;
  @apply bg-destructive ring-2 ring-sidebar;
}
.sidebar-footer {
  display: flex;
  margin-top: auto;
  flex-direction: column;
  gap: 3px;
  padding-inline: 8px;
  padding-block: 8px 14px;
  transition: padding-inline var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease);
}
.sidebar-toggle-block {
  margin-top: 3px;
}
.sidebar-toggle {
  height: 32px;
  gap: 0;
  color: var(--sidebar-foreground);
  color: color-mix(in oklab, var(--sidebar-foreground) 66%, transparent);
  font-size: 12px;
}
.sidebar.expanded .sidebar-toggle {
  gap: 10px;
  padding-inline: 12px;
}
.sidebar-toggle:hover:not(.active) {
  background: var(--sidebar-accent);
  background: color-mix(in oklab, var(--sidebar-accent) 38%, transparent);
  color: var(--sidebar-foreground);
}
@keyframes nav-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
