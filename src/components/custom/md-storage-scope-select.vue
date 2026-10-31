<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';

import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdNativeFileIcon from '@/components/custom/md-native-file-icon.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Select, SelectContent, SelectItem, SelectTrigger } from '@/components/ui/select';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import type { VolumeInfo } from '@/lib/models/storage/volume';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import { FileManagerService } from '@/lib/services/filesystem/file-manager-service';
import { FolderSelectionService } from '@/lib/services/storage/folder-selection-service';
import { findStandardScanFolderByPath } from '@/lib/utils/storage/standard-scan-folder';
import { type StandardScanFolder } from '@/lib/models/storage/standard-scan-folder';
import * as PathUtils from '@/lib/utils/filesystem/path';

const { t } = useI18n({ useScope: 'global' });

const props = withDefaults(
  defineProps<{
    modelValue: string | string[];
    multiple?: boolean;
    disks: VolumeInfo[];
    recentFolders?: string[];
    standardFolders?: StandardScanFolder[];
    disabled?: boolean;
    allowFolder?: boolean;
    protectionEnabled?: boolean;
    protectedPaths?: string[];
  }>(),
  {
    recentFolders: () => [],
    standardFolders: () => [],
    disabled: false,
    allowFolder: true,
    multiple: false,
    protectionEnabled: false,
    protectedPaths: () => [],
  }
);

const emit = defineEmits<{
  error: [error: unknown];
  'remove-folder': [path: string];
  'update:modelValue': [value: string | string[]];
  'update:protectedPaths': [paths: string[]];
}>();

const CHOOSE_FOLDER_VALUE = '__mangodisk_choose_folder__';
const selectOpen = ref(false);
const hoveredFolderPath = ref('');
const selectedPaths = computed(() =>
  Array.isArray(props.modelValue) ? props.modelValue : props.modelValue ? [props.modelValue] : []
);
const firstPath = computed(() => selectedPaths.value[0] ?? '');
const selectedKeys = computed(() => new Set(selectedPaths.value.map(PathUtils.comparisonKey)));
const protectedKeys = computed(() => new Set(props.protectedPaths.map(PathUtils.comparisonKey)));
const isSelected = (path: string) => selectedKeys.value.has(PathUtils.comparisonKey(path));
const isExplicitlyProtected = (path: string) => protectedKeys.value.has(PathUtils.comparisonKey(path));
const protectingAncestor = (path: string) => {
  const pathKey = PathUtils.comparisonKey(path);
  return props.protectedPaths.find(root => {
    const rootKey = PathUtils.comparisonKey(root);
    return rootKey !== pathKey && PathUtils.isSameOrChildKey(pathKey, rootKey);
  });
};
const isProtected = (path: string) => isExplicitlyProtected(path) || Boolean(protectingAncestor(path));
const selectedProtectedCount = computed(() => selectedPaths.value.filter(path => isProtected(path)).length);
function samePath(left: unknown, right: unknown): boolean {
  return (
    typeof left === 'string' &&
    typeof right === 'string' &&
    PathUtils.comparisonKey(left) === PathUtils.comparisonKey(right)
  );
}
const selectedDisk = computed(() => {
  const selectedKey = PathUtils.comparisonKey(firstPath.value);
  return props.disks.find(disk => PathUtils.comparisonKey(disk.mountPoint) === selectedKey) ?? null;
});
const selectedStandardFolder = computed(() =>
  selectedDisk.value ? null : findStandardScanFolderByPath(props.standardFolders, firstPath.value)
);
const selectedLabel = computed(() => {
  if (selectedPaths.value.length > 1) {
    return selectedProtectedCount.value
      ? t('scanScope.selectedLocationsWithProtected', {
          count: selectedPaths.value.length,
          protected: selectedProtectedCount.value,
        })
      : t('scanScope.selectedLocations', { count: selectedPaths.value.length });
  }
  if (selectedDisk.value) return selectedDisk.value.name;
  if (selectedStandardFolder.value) {
    // Keep the real path for scanning and tooltips, but render the localized name through its stable ID.
    return t(`folderPicker.standardFolders.${selectedStandardFolder.value.id}`);
  }
  return PathUtils.fileName(firstPath.value);
});

function toggleProtection(path: string) {
  if (props.disabled || !props.protectionEnabled || !isSelected(path) || protectingAncestor(path)) return;
  const key = PathUtils.comparisonKey(path);
  const next = isExplicitlyProtected(path)
    ? props.protectedPaths.filter(item => PathUtils.comparisonKey(item) !== key)
    : [...props.protectedPaths, PathUtils.display(path)];
  emit('update:protectedPaths', PathUtils.uniquePaths(next));
}

function protectionActionHint(path: string) {
  if (protectingAncestor(path)) return t('scanScope.protectedByParent');
  return t(isProtected(path) ? 'scanScope.markCleanable' : 'scanScope.markProtected');
}
const standardFolderKeys = computed(
  () => new Set(props.standardFolders.map(folder => PathUtils.comparisonKey(folder.path)))
);
const folderOptions = computed(() => {
  const selectedKey = selectedDisk.value ? '' : PathUtils.comparisonKey(firstPath.value);
  const recentFolders = props.recentFolders.filter(
    path => !standardFolderKeys.value.has(PathUtils.comparisonKey(path))
  );
  if (!props.multiple)
    recentFolders.sort((left, right) => {
      if (PathUtils.comparisonKey(left) === selectedKey) return -1;
      if (PathUtils.comparisonKey(right) === selectedKey) return 1;
      return 0;
    });
  const recentKeys = new Set(recentFolders.map(PathUtils.comparisonKey));
  const folders = recentFolders.map(path => ({
    path,
    label: PathUtils.fileName(path),
    removable: true,
    selected: PathUtils.comparisonKey(path) === selectedKey,
  }));
  for (const path of selectedPaths.value) {
    const key = PathUtils.comparisonKey(path);
    if (
      props.disks.some(disk => PathUtils.comparisonKey(disk.mountPoint) === key) ||
      standardFolderKeys.value.has(key) ||
      recentKeys.has(key)
    )
      continue;
    folders.push({ path, label: PathUtils.fileName(path), removable: true, selected: true });
  }
  return folders;
});
const openFolderOptions = ref<typeof folderOptions.value | null>(null);
const visibleFolderOptions = computed(() => openFolderOptions.value ?? folderOptions.value);

// Keep the open menu stable when deselecting folders outside the bounded history.
// A fresh snapshot on each opening still reflects history changes from other pages.
watch(
  selectOpen,
  open => {
    openFolderOptions.value = open && props.multiple ? folderOptions.value : null;
  },
  { flush: 'sync' }
);

async function updateValue(value: unknown) {
  if (props.disabled) return;
  const paths = Array.isArray(value)
    ? value.filter((path): path is string => typeof path === 'string')
    : typeof value === 'string'
      ? [value]
      : [];
  closeTooltips();
  if (!paths.includes(CHOOSE_FOLDER_VALUE)) {
    emit('update:modelValue', props.multiple ? paths.map(PathUtils.display) : PathUtils.display(paths[0] ?? ''));
    return;
  }

  selectOpen.value = false;
  try {
    // Close the menu and tooltip portals before the native dialog deactivates the WebView.
    await nextTick();
    const selected = await FolderSelectionService.select(props.multiple, t('scanScope.chooseFolder'), firstPath.value);
    if (!selected.length) return;
    const directories = await FolderSelectionService.filterExistingDirectories(selected);
    if (props.multiple) {
      const added = directories.filter(path => !isSelected(path));
      emit('update:modelValue', [...selectedPaths.value, ...added]);
    } else if (directories[0]) {
      emit('update:modelValue', PathUtils.display(directories[0]));
    }
  } catch (error) {
    emit('error', error);
  }
}

async function revealFolder(path: string) {
  if (props.disabled) return;
  closeTooltips();
  try {
    await FileManagerService.reveal(path);
  } catch (error) {
    emit('error', error);
  }
}

function navigateFolderActions(event: KeyboardEvent) {
  if (props.disabled || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  if (!(event.currentTarget instanceof HTMLElement) || !(event.target instanceof HTMLElement)) return;
  const row = event.currentTarget;
  const option = row.querySelector<HTMLElement>('[role="option"]');
  if (!option) return;
  const current = event.target.closest<HTMLElement>('[role="option"], button');
  let target: HTMLElement | undefined;

  // Select owns vertical navigation; its collection excludes the sibling action buttons.
  // Keep horizontal movement within this row and resolve vertical movement from its option.
  if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
    const controls = [
      option,
      ...row.querySelectorAll<HTMLButtonElement>('button:not([aria-disabled="true"]):not(:disabled)'),
    ];
    const index = controls.indexOf(current ?? option);
    target = controls[Math.max(0, Math.min(controls.length - 1, index + (event.key === 'ArrowRight' ? 1 : -1)))];
  } else if (current !== option && ['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) {
    const options = Array.from(
      row.closest('[role="listbox"]')?.querySelectorAll<HTMLElement>('[role="option"]:not([aria-disabled="true"])') ??
        []
    );
    const index = options.indexOf(option);
    if (index < 0) return;
    const next =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? options.length - 1
          : index + (event.key === 'ArrowDown' ? 1 : -1);
    target = options[Math.max(0, Math.min(options.length - 1, next))];
  }
  if (!target) return;
  event.preventDefault();
  event.stopPropagation();
  closeTooltips();
  target.focus();
}

async function removeFolder(path: string, event: MouseEvent) {
  const row = event.target instanceof HTMLElement ? event.target.closest('.scope-history-option') : null;
  const restoreFocus = row?.contains(document.activeElement);
  const options = Array.from(
    row?.closest('[role="listbox"]')?.querySelectorAll<HTMLElement>('[role="option"]:not([aria-disabled="true"])') ?? []
  );
  const option = row?.querySelector<HTMLElement>('[role="option"]');
  const index = option ? options.indexOf(option) : -1;
  const adjacent = index >= 0 ? (options[index + 1] ?? options[index - 1]) : undefined;
  closeTooltips();
  // Explicit removal should take effect immediately, unlike unchecking an option.
  if (openFolderOptions.value) {
    openFolderOptions.value = openFolderOptions.value.filter(folder => !samePath(folder.path, path));
  }
  emit('remove-folder', path);
  // Removing a focused action unmounts its button. Preserve keyboard position in the menu.
  if (restoreFocus) {
    await nextTick();
    if (selectOpen.value) {
      const target = option?.isConnected ? option : adjacent;
      if (target?.isConnected) target.focus();
    }
  }
}

function updateSelectOpen(open: boolean) {
  selectOpen.value = open;
  closeTooltips();
}

function showFolderTooltip(path: string) {
  if (selectOpen.value) hoveredFolderPath.value = path;
}

function hideFolderTooltip(path: string) {
  if (hoveredFolderPath.value === path) hoveredFolderPath.value = '';
}

function closeTooltips() {
  hoveredFolderPath.value = '';
}

// Native folder dialogs and window deactivation can remove a tooltip trigger
// before Reka UI receives pointerleave. Clearing controlled state on blur keeps
// the portal from retaining a tooltip without a valid positioning anchor.
onMounted(() => {
  window.addEventListener('blur', closeTooltips);
});
onBeforeUnmount(() => window.removeEventListener('blur', closeTooltips));
watch(() => props.modelValue, closeTooltips);
</script>

<template>
  <Select
    v-model:open="selectOpen"
    :multiple="multiple"
    :by="samePath"
    :model-value="modelValue"
    :disabled="disabled"
    @update:model-value="updateValue"
    @update:open="updateSelectOpen"
  >
    <SelectTrigger
      class="scope-select h-9 w-full sm:w-44"
      :aria-label="t('scanScope.label')"
      @pointerdown="closeTooltips"
    >
      <span class="flex min-w-0 flex-1 items-center gap-2">
        <MdIcon
          class="scope-trigger-icon"
          :class="{ protected: selectedProtectedCount > 0 }"
          :name="
            selectedProtectedCount
              ? ICON_NAMES.lock
              : selectedPaths.length && (!selectedDisk || selectedPaths.length > 1)
                ? ICON_NAMES.folder
                : ICON_NAMES.hardDrive
          "
          :size="18"
        />
        <span class="min-w-0 flex-1 truncate text-left">
          {{ selectedLabel || t('scanScope.label') }}
        </span>
      </span>
    </SelectTrigger>
    <SelectContent class="md-storage-scope-menu" align="end" :collision-padding="12">
      <div v-if="standardFolders.length" class="scope-section-label">
        {{ t('folderPicker.commonFolders') }}
      </div>
      <!-- Anchor outside the item so the tooltip clears its checkbox and preserves the checked state. -->
      <Tooltip
        v-for="folder in standardFolders"
        :key="folder.id"
        :open="hoveredFolderPath === folder.path"
        disable-hoverable-content
      >
        <TooltipTrigger as-child>
          <div
            class="scope-history-option"
            :class="{ selected: isSelected(folder.path) }"
            @pointerenter="showFolderTooltip(folder.path)"
            @pointerleave="hideFolderTooltip(folder.path)"
            @pointerdown="closeTooltips"
            @keydown="navigateFolderActions"
          >
            <SelectItem
              :value="folder.path"
              :text-value="t(`folderPicker.standardFolders.${folder.id}`)"
              class="scope-option-item"
              :class="{
                'scope-multiple-option': multiple,
                'scope-protection-option': protectionEnabled && isSelected(folder.path),
              }"
            >
              <span class="flex w-full min-w-0 items-center gap-2">
                <MdNativeFileIcon
                  class="scope-native-icon"
                  :path="folder.path"
                  :name="t(`folderPicker.standardFolders.${folder.id}`)"
                  directory
                  directory-mode="generic"
                  compact
                />
                <span class="min-w-0 flex-1 truncate">
                  {{ t(`folderPicker.standardFolders.${folder.id}`) }}
                </span>
              </span>
            </SelectItem>
            <div class="scope-option-actions" @pointerenter="closeTooltips" @keydown.enter.stop @keydown.space.stop>
              <MdTooltip v-if="protectionEnabled && isSelected(folder.path)" :text="protectionActionHint(folder.path)">
                <button
                  class="scope-protection-toggle"
                  :class="{ protected: isProtected(folder.path) }"
                  type="button"
                  :aria-disabled="disabled || Boolean(protectingAncestor(folder.path)) || undefined"
                  :aria-label="protectionActionHint(folder.path)"
                  @pointerdown.stop.prevent
                  @click.stop="toggleProtection(folder.path)"
                >
                  <MdIcon v-if="isProtected(folder.path)" :name="ICON_NAMES.lock" :size="13" />
                  <span>
                    {{
                      t(
                        protectingAncestor(folder.path)
                          ? 'scanScope.inheritedProtected'
                          : isProtected(folder.path)
                            ? 'scanScope.protected'
                            : 'scanScope.cleanable'
                      )
                    }}
                  </span>
                </button>
              </MdTooltip>
              <MdIconAction
                appearance="unstyled"
                class="scope-option-reveal"
                :label="t('scanScope.revealFolder')"
                :disabled="disabled"
                @pointerdown.stop.prevent
                @click.stop="revealFolder(folder.path)"
              >
                <MdIcon :name="ICON_NAMES.folderOpen" :size="16" />
              </MdIconAction>
            </div>
          </div>
        </TooltipTrigger>
        <TooltipContent side="left" :side-offset="8" :collision-padding="12" class="md-storage-scope-path-tooltip">
          {{ folder.path }}
        </TooltipContent>
      </Tooltip>
      <div v-if="standardFolders.length && visibleFolderOptions.length" class="scope-separator" role="separator"></div>
      <Tooltip
        v-for="folder in visibleFolderOptions"
        :key="folder.path"
        :open="hoveredFolderPath === folder.path"
        disable-hoverable-content
      >
        <TooltipTrigger as-child>
          <div
            class="scope-history-option"
            :class="{ selected: isSelected(folder.path) }"
            @pointerenter="showFolderTooltip(folder.path)"
            @pointerleave="hideFolderTooltip(folder.path)"
            @pointerdown="closeTooltips"
            @keydown="navigateFolderActions"
          >
            <SelectItem
              :value="folder.path"
              :text-value="folder.label"
              class="scope-option-item scope-option-removable"
              :class="{
                'scope-multiple-option': multiple,
                'scope-protection-option': protectionEnabled && isSelected(folder.path),
              }"
            >
              <span class="flex w-full min-w-0 items-center gap-2">
                <MdNativeFileIcon
                  class="scope-native-icon"
                  :path="folder.path"
                  :name="folder.label"
                  directory
                  directory-mode="generic"
                  compact
                />
                <span class="min-w-0 flex-1 truncate">{{ folder.label }}</span>
              </span>
            </SelectItem>
            <div class="scope-option-actions" @pointerenter="closeTooltips" @keydown.enter.stop @keydown.space.stop>
              <MdTooltip v-if="protectionEnabled && isSelected(folder.path)" :text="protectionActionHint(folder.path)">
                <button
                  class="scope-protection-toggle"
                  :class="{ protected: isProtected(folder.path) }"
                  type="button"
                  :aria-disabled="disabled || Boolean(protectingAncestor(folder.path)) || undefined"
                  :aria-label="protectionActionHint(folder.path)"
                  @pointerdown.stop.prevent
                  @click.stop="toggleProtection(folder.path)"
                >
                  <MdIcon v-if="isProtected(folder.path)" :name="ICON_NAMES.lock" :size="13" />
                  <span>
                    {{
                      t(
                        protectingAncestor(folder.path)
                          ? 'scanScope.inheritedProtected'
                          : isProtected(folder.path)
                            ? 'scanScope.protected'
                            : 'scanScope.cleanable'
                      )
                    }}
                  </span>
                </button>
              </MdTooltip>
              <MdIconAction
                appearance="unstyled"
                class="scope-option-reveal"
                :label="t('scanScope.revealFolder')"
                :disabled="disabled"
                @pointerdown.stop.prevent
                @click.stop="revealFolder(folder.path)"
              >
                <MdIcon :name="ICON_NAMES.folderOpen" :size="16" />
              </MdIconAction>
              <MdIconAction
                v-if="folder.removable"
                appearance="unstyled"
                class="scope-history-remove"
                :label="t('scanScope.removeFolder', { name: folder.label })"
                @pointerdown.stop.prevent
                @click.stop="removeFolder(folder.path, $event)"
              >
                <MdIcon :name="ICON_NAMES.close" :size="14" />
              </MdIconAction>
            </div>
          </div>
        </TooltipTrigger>
        <TooltipContent side="left" :side-offset="8" :collision-padding="12" class="md-storage-scope-path-tooltip">
          {{ folder.path }}
        </TooltipContent>
      </Tooltip>
      <div v-if="standardFolders.length || visibleFolderOptions.length" class="scope-separator" role="separator"></div>
      <div
        v-for="disk in disks"
        :key="disk.mountPoint"
        class="scope-history-option"
        :class="{ selected: isSelected(disk.mountPoint) }"
        @keydown="navigateFolderActions"
      >
        <SelectItem
          :value="disk.mountPoint"
          class="scope-option-item"
          :class="{
            'scope-multiple-option': multiple,
            'scope-protection-option': protectionEnabled && isSelected(disk.mountPoint),
          }"
        >
          <span class="flex min-w-0 items-center gap-2">
            <MdIcon class="flex-none text-muted-foreground" :name="ICON_NAMES.hardDrive" :size="16" />
            <span class="truncate">{{ disk.name }}</span>
          </span>
        </SelectItem>
        <div
          v-if="protectionEnabled && isSelected(disk.mountPoint)"
          class="scope-option-actions"
          @keydown.enter.stop
          @keydown.space.stop
        >
          <MdTooltip :text="protectionActionHint(disk.mountPoint)">
            <button
              class="scope-protection-toggle"
              :class="{ protected: isProtected(disk.mountPoint) }"
              type="button"
              :aria-disabled="disabled || Boolean(protectingAncestor(disk.mountPoint)) || undefined"
              :aria-label="protectionActionHint(disk.mountPoint)"
              @pointerdown.stop.prevent
              @click.stop="toggleProtection(disk.mountPoint)"
            >
              <MdIcon v-if="isProtected(disk.mountPoint)" :name="ICON_NAMES.lock" :size="13" />
              <span>
                {{
                  t(
                    protectingAncestor(disk.mountPoint)
                      ? 'scanScope.inheritedProtected'
                      : isProtected(disk.mountPoint)
                        ? 'scanScope.protected'
                        : 'scanScope.cleanable'
                  )
                }}
              </span>
            </button>
          </MdTooltip>
        </div>
      </div>
      <div v-if="allowFolder" class="scope-separator" role="separator"></div>
      <SelectItem v-if="allowFolder" :value="CHOOSE_FOLDER_VALUE">
        <span class="flex min-w-0 items-center gap-2">
          <MdIcon class="flex-none text-muted-foreground" :name="ICON_NAMES.folderPlus" :size="16" />
          <span>{{ t('scanScope.chooseFolder') }}</span>
        </span>
      </SelectItem>
    </SelectContent>
  </Select>
</template>

<style scoped>
@reference "@assets/main.css";
.scope-select {
  min-width: 0;
  @apply border-border/70 bg-card/35 shadow-none hover:border-border hover:bg-card/55;
}
.scope-select[data-state='open'] {
  @apply border-border bg-card/55 ring-0;
}
.scope-select:focus-visible {
  @apply border-ring ring-3 ring-ring/20;
}
.scope-trigger-icon {
  flex: none;
  @apply text-muted-foreground;
}
.scope-trigger-icon.protected {
  color: var(--warning-foreground, var(--foreground));
}
.scope-section-label {
  padding: 0.25rem 0.5rem 0.125rem;
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
}
.scope-separator {
  height: 1px;
  margin: 0.25rem -0.25rem;
  @apply bg-border;
}
.scope-history-option {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  grid-template-areas: 'option';
}
.scope-history-option > * {
  grid-area: option;
}
.scope-history-option :deep(.scope-native-icon.native-file-icon),
.scope-history-option :deep(.scope-native-icon.directory-fallback) {
  width: 20px;
  height: 20px;
  flex-shrink: 0;
}
.scope-option-actions {
  z-index: 1;
  align-self: center;
  justify-self: end;
  margin-right: 0.25rem;
  display: flex;
  align-items: center;
  gap: 0.125rem;
}
.scope-protection-toggle {
  display: inline-flex;
  height: 1.75rem;
  align-items: center;
  gap: 0.25rem;
  border-radius: 0.3rem;
  padding-inline: 0.45rem;
  color: var(--muted-foreground);
  font-size: var(--font-content-meta);
  font-weight: 600;
  @apply hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none;
}
.scope-protection-toggle.protected {
  color: var(--warning-foreground, var(--foreground));
  background: color-mix(in oklab, var(--warning, var(--primary)) 12%, transparent);
}
.scope-protection-toggle[aria-disabled='true'] {
  cursor: default;
  opacity: 0.72;
}
.scope-option-actions :deep(.icon-action) {
  display: grid;
  width: 1.75rem;
  height: 1.75rem;
  cursor: pointer;
  place-items: center;
  border-radius: 0.25rem;
  @apply text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none;
}
.scope-history-option.selected:not(:has(.scope-multiple-option)) .scope-option-actions {
  margin-right: 1.75rem;
}

/* Reserve action space so hover never moves labels or the selection indicator. */
.scope-option-item {
  padding-right: 3.5rem;
}
.scope-option-item.scope-option-removable {
  padding-right: 5.25rem;
}
.scope-option-item.scope-multiple-option {
  padding-right: 2rem;
}
.scope-option-item.scope-multiple-option.scope-protection-option {
  padding-right: 7.25rem;
}
.scope-option-item.scope-option-removable.scope-multiple-option {
  padding-right: 3.75rem;
}
.scope-option-item.scope-option-removable.scope-multiple-option.scope-protection-option {
  padding-right: 9rem;
}
.scope-option-actions :deep(.scope-option-reveal) {
  opacity: 0;
  pointer-events: none;
}
.scope-history-option:hover :deep(.scope-option-reveal),
.scope-history-option:focus-within :deep(.scope-option-reveal) {
  opacity: 1;
  pointer-events: auto;
}

.scope-multiple-option {
  padding-left: 2rem;
}
.scope-multiple-option :deep(> span:first-child) {
  left: 0.5rem;
  right: auto;
  width: 17px;
  height: 17px;
  border: 1.5px solid var(--color-border);
  border-radius: 5px;
}
.scope-multiple-option :deep(> span:first-child svg) {
  width: 12px;
  height: 12px;
  stroke-width: 2.5px;
}
.scope-multiple-option[data-state='checked'] :deep(> span:first-child) {
  @apply border-primary bg-primary text-primary-foreground;
}
</style>

<style>
/* Portal content needs unscoped selectors; keep them specific to this control. */
.md-storage-scope-menu {
  width: max(21rem, var(--reka-select-trigger-width, 0px));
  max-width: min(28rem, var(--reka-select-content-available-width, 28rem), calc(100vw - 1.5rem));
}
.md-storage-scope-menu [data-reka-select-viewport] {
  min-width: 0;
}
/* SelectItemText wraps the label in a flex item whose default minimum width
   otherwise lets long names defeat the label's ellipsis. */
.md-storage-scope-menu [data-slot='select-item'] > span:last-child {
  min-width: 0;
  flex: 1;
}
.md-storage-scope-path-tooltip {
  max-width: min(34rem, var(--reka-tooltip-content-available-width, 34rem), calc(100vw - 1.5rem));
  pointer-events: none;
  overflow-wrap: anywhere;
  text-align: left;
  text-wrap: wrap;
  white-space: normal;
}
</style>
