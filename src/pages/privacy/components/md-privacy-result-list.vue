<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';

import MdAiAction from '@/components/custom/md-ai-action.vue';
import MdApplicationIcon from '@/components/custom/md-application-icon.vue';
import MdResultCategoryItem from '@/components/custom/md-result-category-item.vue';
import MdResultMasterDetail from '@/components/custom/md-result-master-detail.vue';
import MdResultCheckbox from '@/components/custom/md-result-checkbox.vue';
import MdResultDetailHeader from '@/components/custom/md-result-detail-header.vue';
import MdResultItemContent from '@/components/custom/md-result-item-content.vue';
import MdResultTable from '@/components/custom/md-result-table.vue';
import MdResultTableHierarchy from '@/components/custom/md-result-table-hierarchy.vue';
import MdResultTableRow from '@/components/custom/md-result-table-row.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import type { PrivacyCategory, PrivacyDataKind, PrivacyItem } from '@/lib/models/privacy';
import { ICON_NAMES, type IconName } from '@/lib/models/ui';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import * as FormatUtils from '@/lib/utils/format';
import { isPrivacyItemActionable } from '@/lib/utils/privacy-selection';

import {
  buildPrivacyResultCategories,
  buildPrivacyResultSourceGroups,
  type PrivacyResultCategory,
  type PrivacyResultProfileGroup,
  type PrivacyResultSourceGroup,
} from '../privacy-result-categories';

const props = defineProps<{
  busy: boolean;
  items: PrivacyItem[];
  selectedTokens: string[];
  sourceIconUrls?: Readonly<Record<string, string>>;
  permissionLabel?: string;
}>();
const emit = defineEmits<{
  'update:selectedTokens': [tokens: string[]];
  'show-details': [item: PrivacyItem];
  explain: [item: PrivacyItem];
}>();

const { t } = useI18n({ useScope: 'global' });
const activeCategoryId = ref<PrivacyCategory>('browserActivity');
const expandedSourceIds = ref(new Set<string>());
const sourceExpansionInitialized = ref(false);
const failedIconSourceIds = ref(new Set<string>());
const detailList = ref<InstanceType<typeof MdResultTable> | null>(null);
const selected = computed(() => new Set(props.selectedTokens));
const categories = computed(() => buildPrivacyResultCategories(props.items, props.selectedTokens));
const activeCategory = computed(
  () => categories.value.find(category => category.id === activeCategoryId.value) ?? null
);
const sourceGroups = computed(() =>
  activeCategory.value?.id === 'systemActivity'
    ? []
    : buildPrivacyResultSourceGroups(activeCategory.value?.items ?? [], props.selectedTokens)
);
const systemKinds = new Set<PrivacyDataKind>([
  'currentClipboard',
  'clipboardHistory',
  'recentItems',
  'recentApplications',
  'applicationUsageHistory',
  'networkConnectionHistory',
  'folderViewHistory',
  'printerHistory',
  'shellHistory',
  'jumpLists',
  'runDialogHistory',
  'fileDialogHistory',
  'systemSearchHistory',
  'explorerPathHistory',
]);

const categoryIcons: Record<PrivacyCategory, IconName> = {
  browserActivity: ICON_NAMES.globe,
  browserAccountState: ICON_NAMES.shield,
  applicationActivity: ICON_NAMES.application,
  systemActivity: ICON_NAMES.clock,
};
const sourceIcons: Readonly<Record<string, IconName>> = {
  chrome: ICON_NAMES.brandChrome,
  edge: ICON_NAMES.brandEdge,
  opera: ICON_NAMES.brandOpera,
  qq_browser: ICON_NAMES.brandQq,
  firefox: ICON_NAMES.brandFirefox,
  safari: ICON_NAMES.brandApple,
};

const kindIcons: Record<PrivacyDataKind, IconName> = {
  browsingHistory: ICON_NAMES.history,
  downloadHistory: ICON_NAMES.download,
  cookies: ICON_NAMES.shield,
  siteStorage: ICON_NAMES.database,
  sitePermissions: ICON_NAMES.shield,
  sessions: ICON_NAMES.application,
  browserCache: ICON_NAMES.database,
  searchHistory: ICON_NAMES.search,
  websiteIcons: ICON_NAMES.globe,
  frequentlyVisitedSites: ICON_NAMES.history,
  addressBarShortcuts: ICON_NAMES.search,
  savedPasswords: ICON_NAMES.shield,
  autofillData: ICON_NAMES.application,
  currentClipboard: ICON_NAMES.copy,
  clipboardHistory: ICON_NAMES.history,
  recentItems: ICON_NAMES.clock,
  recentApplications: ICON_NAMES.application,
  applicationUsageHistory: ICON_NAMES.history,
  networkConnectionHistory: ICON_NAMES.globe,
  folderViewHistory: ICON_NAMES.folder,
  printerHistory: ICON_NAMES.file,
  shellHistory: ICON_NAMES.cleanupDeveloperTools,
  jumpLists: ICON_NAMES.list,
  runDialogHistory: ICON_NAMES.application,
  fileDialogHistory: ICON_NAMES.folder,
  systemSearchHistory: ICON_NAMES.search,
  explorerPathHistory: ICON_NAMES.folder,
  applicationCache: ICON_NAMES.database,
  applicationLogs: ICON_NAMES.list,
  applicationSessions: ICON_NAMES.application,
  editorLocalHistory: ICON_NAMES.history,
  recentDocuments: ICON_NAMES.file,
  recentProjects: ICON_NAMES.folder,
  recentConnections: ICON_NAMES.globe,
  playbackHistory: ICON_NAMES.history,
  recentPaths: ICON_NAMES.folder,
  recentSearches: ICON_NAMES.search,
};

function selectable(item: PrivacyItem): boolean {
  return isPrivacyItemActionable(item);
}

function isSystemKind(kind: PrivacyDataKind): boolean {
  return systemKinds.has(kind);
}

function sourceIcon(sourceId: string): IconName {
  return (
    sourceIcons[sourceId] ??
    (activeCategoryId.value === 'applicationActivity' ? ICON_NAMES.application : ICON_NAMES.globe)
  );
}

function sourceIconUrl(sourceId: string): string | undefined {
  if (failedIconSourceIds.value.has(sourceId)) return undefined;
  return props.sourceIconUrls?.[sourceId];
}

function handleSourceIconError(sourceId: string) {
  const next = new Set(failedIconSourceIds.value);
  next.add(sourceId);
  failedIconSourceIds.value = next;
}

watch(
  () => props.sourceIconUrls,
  () => {
    // Retry after icon resolution changes so a transient decode failure does not hide the native icon permanently.
    failedIconSourceIds.value = new Set();
  }
);

function sourceProfileSummary(group: PrivacyResultSourceGroup): string {
  return group.hasProfiles
    ? t('privacy.profileCount', { count: FormatUtils.integer(group.profiles.length) }, group.profiles.length)
    : t('common.itemCount', { count: FormatUtils.integer(group.items.length) }, group.items.length);
}

function toggleCategory(category: PrivacyResultCategory, checked: boolean) {
  const next = new Set(props.selectedTokens);
  for (const item of category.items.filter(selectable)) {
    if (checked) next.add(item.token);
    else next.delete(item.token);
  }
  emit('update:selectedTokens', [...next]);
}

function toggleSource(group: PrivacyResultSourceGroup, checked: boolean) {
  const next = new Set(props.selectedTokens);
  for (const item of group.items.filter(selectable)) {
    if (checked) next.add(item.token);
    else next.delete(item.token);
  }
  emit('update:selectedTokens', [...next]);
}

function toggleProfile(profile: PrivacyResultProfileGroup, checked: boolean) {
  const next = new Set(props.selectedTokens);
  for (const item of profile.items.filter(selectable)) {
    if (checked) next.add(item.token);
    else next.delete(item.token);
  }
  emit('update:selectedTokens', [...next]);
}

function toggleSourceExpanded(sourceId: string) {
  const next = new Set(expandedSourceIds.value);
  if (next.has(sourceId)) next.delete(sourceId);
  else next.add(sourceId);
  expandedSourceIds.value = next;
}

function toggleItem(token: string) {
  const next = new Set(props.selectedTokens);
  if (next.has(token)) next.delete(token);
  else next.add(token);
  emit('update:selectedTokens', [...next]);
}

function capabilityTone(item: PrivacyItem): 'accent' | 'neutral' | 'positive' | 'warning' {
  if (isPrivacyItemActionable(item) && item.impact === 'low') return 'positive';
  if (item.capability === 'permissionRequired') return 'warning';
  return 'neutral';
}

function itemBadge(item: PrivacyItem): string | undefined {
  if (item.recommendation === 'reviewOnly') return t('privacy.recommendations.reviewOnly');
  if (['browserRunning', 'applicationRunning'].includes(item.capability) && item.itemCount === 0) {
    return t('privacy.capabilities.scanAfterClose');
  }
  if (item.capability === 'permissionRequired' && props.permissionLabel) return props.permissionLabel;
  // Browser shutdown is an execution prerequisite shown once in the confirmation dialog.
  // Repeating it on every row obscures the cleanup risk, which is the decision users make here.
  if (isPrivacyItemActionable(item)) return item.impact === 'low' ? t('common.safe') : undefined;
  return t(`privacy.capabilities.${item.capability}`);
}

function itemValue(item: PrivacyItem): string {
  return ['browserRunning', 'applicationRunning'].includes(item.capability) && item.itemCount === 0
    ? t('privacy.pendingScan')
    : FormatUtils.integer(item.itemCount);
}

function itemDescription(item: PrivacyItem, includeSource: boolean): string {
  const parts: string[] = [];
  if (!isSystemKind(item.kind) && includeSource) {
    const owner = [item.sourceName, item.profileName].filter(Boolean).join(' · ');
    if (owner) parts.push(owner);
  }
  // Low risk is already communicated by the badge. Keep the second line only for a distinct
  // consequence such as signing out, workflow disruption, or personal-data removal.
  if (item.impact !== 'low') parts.push(t(`privacy.impacts.${item.impact}`));
  return parts.join(' · ');
}

watch(
  () => categories.value.map(category => category.id),
  categoryIds => {
    if (!categoryIds.includes(activeCategoryId.value)) activeCategoryId.value = categoryIds[0] ?? 'browserActivity';
  },
  { immediate: true }
);

watch(
  sourceGroups,
  groups => {
    if (sourceExpansionInitialized.value || !groups.length) return;
    sourceExpansionInitialized.value = true;
    const firstSourceId = groups[0]?.id;
    if (firstSourceId) expandedSourceIds.value = new Set([firstSourceId]);
  },
  { immediate: true }
);

watch(activeCategoryId, async () => {
  await nextTick();
  detailList.value?.scrollTo({ top: 0 });
});
</script>

<template>
  <MdResultMasterDetail
    class="embedded"
    :empty="!categories.length"
    :navigation-label="t('privacy.categoryNavigation')"
  >
    <template v-if="categories.length" #navigation>
      <MdResultCategoryItem
        v-for="category in categories"
        :key="category.id"
        :active="activeCategoryId === category.id"
        :title="t(`privacy.categories.${category.id}`)"
        :description="`${t('common.itemCount', { count: FormatUtils.integer(category.itemCount) }, category.itemCount)} · ${t(
          'privacy.traceCount',
          { count: FormatUtils.integer(category.traceCount) },
          category.traceCount
        )}`"
        :icon-name="categoryIcons[category.id]"
        :selected-summary="category.selection === 'none' ? undefined : FormatUtils.integer(category.selectedTraceCount)"
        :selected-aria-label="
          category.selection === 'none'
            ? undefined
            : t(
                'privacy.categorySelected',
                { count: FormatUtils.integer(category.selectedTraceCount) },
                category.selectedTraceCount
              )
        "
        @select="activeCategoryId = category.id"
      />
    </template>

    <section v-if="activeCategory" class="privacy-details">
      <MdResultDetailHeader
        :title="t(`privacy.categories.${activeCategory.id}`)"
        :description="t(`privacy.categoryDescriptions.${activeCategory.id}`)"
        :selection="activeCategory.selection"
        :select-label="t('privacy.selectAllInCategory')"
        :disabled="busy || !activeCategory.items.some(selectable)"
        @update:selected="toggleCategory(activeCategory, $event)"
      >
        <template #metric>
          <small>{{ t('privacy.selectedSummary') }}</small>
          <strong>{{ FormatUtils.integer(activeCategory.selectedTraceCount) }}</strong>
          <i>/ {{ FormatUtils.integer(activeCategory.traceCount) }}</i>
        </template>
      </MdResultDetailHeader>

      <MdResultTable ref="detailList" class="detail-list">
        <div v-if="sourceGroups.length" class="source-list">
          <section v-for="group in sourceGroups" :key="group.id" class="source-section">
            <MdResultTableRow
              layout="item"
              class="source-header"
              :data-selected="group.selection !== 'none'"
              :data-expanded="expandedSourceIds.has(group.id)"
            >
              <MdResultCheckbox
                :checked="group.selection === 'all'"
                :indeterminate="group.selection === 'partial'"
                :disabled="busy || !group.items.some(selectable)"
                :aria-label="t('privacy.toggleSource', { source: group.sourceName })"
                @update:checked="toggleSource(group, $event)"
              />
              <button
                class="source-disclosure"
                type="button"
                :aria-expanded="expandedSourceIds.has(group.id)"
                :aria-label="t('privacy.toggleSourceDetails', { source: group.sourceName })"
                @click="toggleSourceExpanded(group.id)"
              >
                <MdResultItemContent
                  :title="group.sourceName"
                  :description="sourceProfileSummary(group)"
                  :badge="group.permissionRequired ? permissionLabel : undefined"
                  :badge-tone="group.permissionRequired ? 'warning' : 'neutral'"
                  :value="t('privacy.traceCount', { count: FormatUtils.integer(group.traceCount) }, group.traceCount)"
                  :expandable="true"
                  :expanded="expandedSourceIds.has(group.id)"
                >
                  <template #icon>
                    <MdApplicationIcon
                      v-if="sourceIconUrl(group.id)"
                      :src="sourceIconUrl(group.id)"
                      :size="30"
                      @error="handleSourceIconError(group.id)"
                    />
                    <MdIcon v-else :name="sourceIcon(group.id)" :size="20" />
                  </template>
                </MdResultItemContent>
              </button>
            </MdResultTableRow>

            <MdResultTableHierarchy v-if="expandedSourceIds.has(group.id)">
              <section
                v-for="profile in group.hasProfiles ? group.profiles : []"
                :key="profile.id"
                class="profile-section"
              >
                <MdResultTableRow layout="item" class="profile-header" :data-selected="profile.selection !== 'none'">
                  <MdResultCheckbox
                    :checked="profile.selection === 'all'"
                    :indeterminate="profile.selection === 'partial'"
                    :disabled="busy || !profile.items.some(selectable)"
                    :aria-label="t('privacy.toggleProfile', { profile: profile.profileName })"
                    @update:checked="toggleProfile(profile, $event)"
                  />
                  <MdResultItemContent
                    :title="profile.profileName"
                    :value="
                      t('privacy.traceCount', { count: FormatUtils.integer(profile.traceCount) }, profile.traceCount)
                    "
                  >
                    <template #icon><MdIcon :name="ICON_NAMES.userProfile" :size="20" /></template>
                  </MdResultItemContent>
                </MdResultTableRow>

                <MdResultTableHierarchy class="profile-items">
                  <MdResultTableRow
                    v-for="item in profile.items"
                    :key="item.token"
                    layout="item"
                    class="privacy-item md-ai-hover-row"
                    :data-selected="selected.has(item.token)"
                  >
                    <MdResultCheckbox
                      :checked="selected.has(item.token)"
                      :disabled="busy || !selectable(item)"
                      :aria-label="t('privacy.toggleItem', { item: t(`privacy.kinds.${item.kind}`) })"
                      @update:checked="toggleItem(item.token)"
                    />
                    <div class="privacy-item-details">
                      <button
                        class="privacy-item-disclosure"
                        type="button"
                        :disabled="item.itemCount === 0"
                        :aria-label="t('privacy.details.open', { item: t(`privacy.kinds.${item.kind}`) })"
                        @click="emit('show-details', item)"
                      />
                      <MdResultItemContent
                        :title="t(`privacy.kinds.${item.kind}`)"
                        :description="itemDescription(item, false)"
                        :badge="itemBadge(item)"
                        :badge-tone="capabilityTone(item)"
                        :value="itemValue(item)"
                        :value-detail="item.itemCount > 0 ? ByteSizeService.bytes(item.estimatedBytes) : undefined"
                      >
                        <template #icon><MdIcon :name="kindIcons[item.kind]" :size="20" /></template>
                        <template #actions
                          ><MdAiAction
                            :name="t(`privacy.kinds.${item.kind}`)"
                            :disabled="busy"
                            @explain="emit('explain', item)"
                        /></template>
                      </MdResultItemContent>
                      <MdIcon class="privacy-detail-chevron" :name="ICON_NAMES.chevronRight" :size="16" />
                    </div>
                  </MdResultTableRow>
                </MdResultTableHierarchy>
              </section>

              <MdResultTableRow
                v-for="item in group.hasProfiles ? [] : group.items"
                :key="item.token"
                layout="item"
                class="privacy-item md-ai-hover-row"
                :data-selected="selected.has(item.token)"
              >
                <MdResultCheckbox
                  :checked="selected.has(item.token)"
                  :disabled="busy || !selectable(item)"
                  :aria-label="t('privacy.toggleItem', { item: t(`privacy.kinds.${item.kind}`) })"
                  @update:checked="toggleItem(item.token)"
                />
                <div class="privacy-item-details">
                  <button
                    class="privacy-item-disclosure"
                    type="button"
                    :disabled="item.itemCount === 0"
                    :aria-label="t('privacy.details.open', { item: t(`privacy.kinds.${item.kind}`) })"
                    @click="emit('show-details', item)"
                  />
                  <MdResultItemContent
                    :title="t(`privacy.kinds.${item.kind}`)"
                    :description="itemDescription(item, false)"
                    :badge="itemBadge(item)"
                    :badge-tone="capabilityTone(item)"
                    :value="itemValue(item)"
                    :value-detail="item.itemCount > 0 ? ByteSizeService.bytes(item.estimatedBytes) : undefined"
                  >
                    <template #icon><MdIcon :name="kindIcons[item.kind]" :size="20" /></template>
                    <template #actions
                      ><MdAiAction
                        :name="t(`privacy.kinds.${item.kind}`)"
                        :disabled="busy"
                        @explain="emit('explain', item)"
                    /></template>
                  </MdResultItemContent>
                  <MdIcon class="privacy-detail-chevron" :name="ICON_NAMES.chevronRight" :size="16" />
                </div>
              </MdResultTableRow>
            </MdResultTableHierarchy>
          </section>
        </div>

        <div v-else class="detail-list-content">
          <MdResultTableRow
            v-for="item in activeCategory.items"
            :key="item.token"
            layout="item"
            class="privacy-item md-ai-hover-row"
            :data-selected="selected.has(item.token)"
          >
            <MdResultCheckbox
              :checked="selected.has(item.token)"
              :disabled="busy || !selectable(item)"
              :aria-label="t('privacy.toggleItem', { item: t(`privacy.kinds.${item.kind}`) })"
              @update:checked="toggleItem(item.token)"
            />
            <div class="privacy-item-details">
              <button
                class="privacy-item-disclosure"
                type="button"
                :disabled="item.itemCount === 0"
                :aria-label="t('privacy.details.open', { item: t(`privacy.kinds.${item.kind}`) })"
                @click="emit('show-details', item)"
              />
              <MdResultItemContent
                :title="t(`privacy.kinds.${item.kind}`)"
                :description="itemDescription(item, true)"
                :badge="itemBadge(item)"
                :badge-tone="capabilityTone(item)"
                :value="itemValue(item)"
                :value-detail="item.itemCount > 0 ? ByteSizeService.bytes(item.estimatedBytes) : undefined"
              >
                <template #icon><MdIcon :name="kindIcons[item.kind]" :size="20" /></template>
                <template #actions
                  ><MdAiAction
                    :name="t(`privacy.kinds.${item.kind}`)"
                    :disabled="busy"
                    @explain="emit('explain', item)"
                /></template>
              </MdResultItemContent>
              <MdIcon class="privacy-detail-chevron" :name="ICON_NAMES.chevronRight" :size="16" />
            </div>
          </MdResultTableRow>
        </div>
      </MdResultTable>
    </section>

    <div v-else class="privacy-details empty">
      <span class="empty-icon"><MdIcon :name="ICON_NAMES.check" :size="22" /></span>
      <strong>{{ t('privacy.emptyResultsTitle') }}</strong>
      <small>{{ t('privacy.emptyResultsDescription') }}</small>
    </div>
  </MdResultMasterDetail>
</template>

<style scoped src="./md-privacy-result-list.css"></style>
