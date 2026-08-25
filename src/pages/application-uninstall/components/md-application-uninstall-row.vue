<script setup lang="ts">
import { useI18n } from 'vue-i18n';

import MdApplicationIcon from '@/components/custom/md-application-icon.vue';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdResultCheckbox from '@/components/custom/md-result-checkbox.vue';
import MdResultTableHierarchy from '@/components/custom/md-result-table-hierarchy.vue';
import MdResultTableRow from '@/components/custom/md-result-table-row.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import type { ApplicationUninstallCandidate, ApplicationUninstallComponentSummary } from '@/lib/models/application';
import { ICON_NAMES, type IconName } from '@/lib/models/ui';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import { FormatUtils } from '@/lib/utils/format';
import { PathUtils } from '@/lib/utils/path';

import { applicationCanStartUninstall, applicationStatusKey } from '../application-uninstall-catalog';
import { defaultApplicationComponentIds } from '../application-uninstall-selection';

const props = defineProps<{
  candidate: ApplicationUninstallCandidate;
  iconSrc?: string;
  selected: boolean;
  selectedComponentIds: string[];
  expanded: boolean;
  busy: boolean;
  uninstallEnabled: boolean;
  aiRecommended?: boolean;
}>();
const emit = defineEmits<{
  toggleSelection: [];
  toggleComponent: [component: ApplicationUninstallComponentSummary];
  toggleExpanded: [];
  open: [path: string];
  uninstall: [];
  iconError: [];
}>();
const { locale, t } = useI18n({ useScope: 'global' });

function componentSelected(componentId: string): boolean {
  return props.selected && props.selectedComponentIds.includes(componentId);
}

function canUninstallCandidate(): boolean {
  return (
    props.uninstallEnabled &&
    applicationCanStartUninstall(props.candidate) &&
    defaultApplicationComponentIds(props.candidate).length > 0
  );
}

function candidateDateText(): string {
  const timestamp =
    props.candidate.platform === 'windowsRegistry' ? props.candidate.installedAtMs : props.candidate.lastUsedAtMs;
  if (timestamp !== null) return FormatUtils.dateTime(timestamp, locale.value);
  return t(
    props.candidate.platform === 'windowsRegistry'
      ? 'applicationUninstall.installDateUnavailable'
      : 'applicationUninstall.lastUsedUnavailable'
  );
}

function componentIcon(component: ApplicationUninstallComponentSummary): IconName {
  switch (component.kind) {
    case 'applicationBinary':
    case 'nativeInstaller':
      return ICON_NAMES.application;
    case 'cache':
      return ICON_NAMES.database;
    case 'applicationSupport':
    case 'sandboxContainer':
      return ICON_NAMES.package;
    case 'preferences':
      return ICON_NAMES.fileSettings;
    case 'logs':
      return ICON_NAMES.fileText;
    case 'savedState':
      return ICON_NAMES.history;
    case 'webData':
      return ICON_NAMES.globe;
  }
}

function componentLabel(component: ApplicationUninstallComponentSummary): string {
  const nativeKinds = {
    windowsMsi: 'windowsMsiPackage',
    windowsAppx: 'windowsAppPackage',
    windowsScoop: 'windowsScoopPackage',
    windowsChocolatey: 'windowsChocolateyPackage',
    windowsRegistered: 'windowsRegisteredUninstaller',
  } as const;
  const kind =
    component.kind === 'nativeInstaller' && props.candidate.installerKind
      ? nativeKinds[props.candidate.installerKind]
      : component.kind;
  return t(`applicationUninstall.componentKinds.${kind}`);
}

function componentDescription(component: ApplicationUninstallComponentSummary): string {
  if (component.kind === 'nativeInstaller' && props.candidate.executionMode) {
    return t(`applicationUninstall.executionModes.${props.candidate.executionMode}`);
  }
  return t(`applicationUninstall.componentRisks.${component.risk}`);
}

function displayedApplicationSize(): string {
  if (!props.candidate.totalBytes) return t('applicationUninstall.applicationSizeUnavailable');
  return ByteSizeService.bytes(props.candidate.totalBytes);
}

function displayedComponentSize(component: ApplicationUninstallComponentSummary): string {
  return ByteSizeService.bytes(component.bytes);
}
</script>

<template>
  <article class="application-row">
    <MdResultTableRow class="application-row-line" :data-selected="selected" :data-expanded="expanded">
      <MdResultCheckbox
        class="application-check"
        :checked="selected"
        :disabled="
          busy || !applicationCanStartUninstall(candidate) || !defaultApplicationComponentIds(candidate).length
        "
        :aria-label="t('applicationUninstall.selectApplication', { name: candidate.name })"
        @update:checked="emit('toggleSelection')"
      />
      <div
        class="application-main"
        :class="{
          'has-two-actions': Boolean(candidate.applicationPath) && canUninstallCandidate(),
        }"
        @click="emit('toggleExpanded')"
      >
        <button
          class="application-disclosure"
          type="button"
          :aria-expanded="expanded"
          @click.stop="emit('toggleExpanded')"
        >
          <MdApplicationIcon :src="iconSrc" :platform="candidate.platform" @error="emit('iconError')" />
          <span class="application-identity">
            <span class="application-name-line">
              <strong class="md-result-primary">{{ candidate.name }}</strong>
              <span v-if="aiRecommended" class="ai-badge" :title="t('largeFiles.aiAdvisorBadge')">
                <MdIcon :name="ICON_NAMES.smartSelect" :size="11" />
                <span>{{ t('largeFiles.aiAdvisorBadge') }}</span>
              </span>
            </span>
            <small>
              {{ candidate.publisher || candidate.primaryIdentifier }}
              <template v-if="candidate.version">
                · {{ t('applicationUninstall.version', { version: candidate.version }) }}
              </template>
            </small>
          </span>
        </button>
        <span v-if="candidate.applicationPath || canUninstallCandidate()" class="application-actions">
          <MdIconAction
            v-if="candidate.applicationPath"
            variant="ghost"
            :label="t('applicationUninstall.showLocation')"
            :aria-label="
              t('applicationUninstall.showApplicationLocation', {
                application: candidate.name,
              })
            "
            @click.stop="emit('open', candidate.applicationPath)"
          >
            <MdIcon :name="ICON_NAMES.folder" :size="16" />
          </MdIconAction>
          <MdIconAction
            v-if="canUninstallCandidate()"
            variant="ghost"
            destructive
            :disabled="busy"
            :label="t('applicationUninstall.uninstallApplication', { application: candidate.name })"
            @click.stop="emit('uninstall')"
          >
            <MdIcon :name="ICON_NAMES.uninstall" :size="16" />
          </MdIconAction>
        </span>
        <span class="application-status" :class="candidate.capability">
          {{ t(`applicationUninstall.${applicationStatusKey(candidate)}`) }}
        </span>
        <strong
          class="application-size md-result-primary"
          :title="
            candidate.installerKind === 'windowsAppx' ? t('applicationUninstall.windowsAppPackageSizeHint') : undefined
          "
        >
          {{ displayedApplicationSize() }}
        </strong>
        <span class="application-date">{{ candidateDateText() }}</span>
        <MdIconAction
          class="application-expand"
          variant="ghost"
          :label="
            t(
              expanded
                ? 'applicationUninstall.collapseApplicationDetails'
                : 'applicationUninstall.expandApplicationDetails',
              { application: candidate.name }
            )
          "
          :aria-expanded="expanded"
          @click.stop="emit('toggleExpanded')"
        >
          <MdIcon class="application-chevron" :class="{ expanded }" :name="ICON_NAMES.chevronDown" :size="17" />
        </MdIconAction>
      </div>
    </MdResultTableRow>

    <div v-if="expanded" class="application-details">
      <p v-if="candidate.recordState === 'orphanedRegistration'" class="association-warning">
        <MdIcon :name="ICON_NAMES.info" :size="14" />
        {{ t('applicationUninstall.orphanedRegistrationDescription') }}
      </p>
      <p
        v-else-if="candidate.platform === 'macosBundle' && candidate.capability === 'requiresElevation'"
        class="association-warning"
      >
        <MdIcon :name="ICON_NAMES.info" :size="14" />
        {{ t('applicationUninstall.requiresElevationDescriptionMacos') }}
      </p>
      <p v-else-if="candidate.capability === 'viewOnly'" class="association-warning">
        <MdIcon :name="ICON_NAMES.info" :size="14" />
        {{ t('applicationUninstall.uninstallEntryUnavailableDescription') }}
      </p>
      <MdResultTableHierarchy v-if="candidate.components.length">
        <MdResultTableRow
          v-for="component in candidate.components"
          :key="component.componentId"
          class="component-row"
          :data-selected="componentSelected(component.componentId)"
        >
          <MdResultCheckbox
            :checked="componentSelected(component.componentId)"
            :disabled="busy || !applicationCanStartUninstall(candidate) || component.risk === 'required'"
            :aria-label="
              t('applicationUninstall.selectComponent', {
                component: componentLabel(component),
                application: candidate.name,
              })
            "
            @update:checked="emit('toggleComponent', component)"
          />
          <span class="component-icon">
            <MdIcon :name="componentIcon(component)" :size="17" />
          </span>
          <span class="component-primary">
            <span class="component-main">
              <strong class="md-result-primary">{{ componentLabel(component) }}</strong>
              <small v-if="component.path" :title="component.path">{{ PathUtils.display(component.path) }}</small>
              <small v-else>{{ componentDescription(component) }}</small>
            </span>
            <span v-if="component.path" class="component-actions">
              <MdIconAction
                variant="ghost"
                :label="t('applicationUninstall.showLocation')"
                :aria-label="
                  t('applicationUninstall.showComponentLocation', {
                    component: componentLabel(component),
                  })
                "
                @click="emit('open', component.path)"
              >
                <MdIcon :name="ICON_NAMES.folder" :size="16" />
              </MdIconAction>
            </span>
          </span>
          <span class="component-risk" :class="component.risk">
            {{ t(`applicationUninstall.componentRisks.${component.risk}`) }}
          </span>
          <strong
            class="component-size md-result-primary"
            :title="
              component.kind === 'nativeInstaller' && candidate.installerKind === 'windowsAppx'
                ? t('applicationUninstall.windowsAppPackageSizeHint')
                : undefined
            "
          >
            {{ displayedComponentSize(component) }}
          </strong>
        </MdResultTableRow>
      </MdResultTableHierarchy>
      <div v-if="candidate.possibleRelatedPaths.length" class="possible-related-list">
        <strong>{{ t('applicationUninstall.possibleRelatedLocations') }}</strong>
        <p>{{ t('applicationUninstall.possibleRelatedLocationsDescription') }}</p>
        <div v-for="path in candidate.possibleRelatedPaths" :key="path" class="possible-related-location">
          <small :title="path">{{ PathUtils.display(path) }}</small>
          <MdIconAction
            variant="ghost"
            :label="t('applicationUninstall.showLocation')"
            :aria-label="
              t('applicationUninstall.showPossibleRelatedLocation', {
                application: candidate.name,
              })
            "
            @click="emit('open', path)"
          >
            <MdIcon :name="ICON_NAMES.folder" :size="16" />
          </MdIconAction>
        </div>
      </div>
    </div>
  </article>
</template>

<style scoped src="./md-application-uninstall-row.css"></style>
