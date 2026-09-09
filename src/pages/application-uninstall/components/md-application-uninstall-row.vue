<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';

import { Button } from '@/components/ui/button';
import MdApplicationIcon from '@/components/custom/md-application-icon.vue';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdStatusBadge from '@/components/custom/md-status-badge.vue';
import MdResultCheckbox from '@/components/custom/md-result-checkbox.vue';
import MdResultTableHierarchy from '@/components/custom/md-result-table-hierarchy.vue';
import MdResultTableRow from '@/components/custom/md-result-table-row.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import type { ApplicationUninstallCandidate, ApplicationUninstallComponentSummary } from '@/lib/models/application';
import { ICON_NAMES, type IconName } from '@/lib/models/ui';
import { ByteSizeService } from '@/lib/services/byte-size-service';
import * as FormatUtils from '@/lib/utils/format';
import * as PathUtils from '@/lib/utils/path';

import {
  applicationCanStartUninstall,
  applicationStatusKey,
  applicationIsSystemItem,
} from '../application-uninstall-catalog';
import { applicationSizeHintKey, applicationUnavailableTitleKey } from '../application-uninstall-presentation';
import { defaultApplicationComponentIds } from '../application-uninstall-selection';
import MdApplicationUninstallDetailRow from './md-application-uninstall-detail-row.vue';

const props = defineProps<{
  candidate: ApplicationUninstallCandidate;
  iconSrc?: string;
  selected: boolean;
  selectedComponentIds: string[];
  expanded: boolean;
  busy: boolean;
  uninstallEnabled: boolean;
}>();
const emit = defineEmits<{
  toggleSelection: [];
  toggleComponent: [component: ApplicationUninstallComponentSummary];
  toggleExpanded: [];
  open: [path: string];
  uninstall: [];
  iconError: [];
  openWindowsSettings: [];
  removeRecord: [];
}>();
const { locale, t } = useI18n({ useScope: 'global' });
const canRemoveRecord = computed(
  () =>
    props.candidate.platform === 'windowsRegistry' &&
    (props.candidate.recordState === 'orphanedRegistration' || props.candidate.capability === 'viewOnly')
);

const showUnavailableEntry = computed(
  () => props.candidate.capability === 'viewOnly' && props.candidate.recordState !== 'orphanedRegistration'
);

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

function displayedSizeHint(): string {
  return t(applicationSizeHintKey(props.candidate.installerKind));
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
          'has-two-actions': Boolean(candidate.applicationPath) && (canUninstallCandidate() || canRemoveRecord),
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
            <span class="flex min-w-0 items-center gap-2">
              <strong class="md-result-primary">{{ candidate.name }}</strong>
              <MdStatusBadge v-if="applicationIsSystemItem(candidate)" size="compact">
                {{
                  t(
                    candidate.systemKind === 'sharedRuntime' || candidate.systemKind === 'windowsSharedPackage'
                      ? 'applicationUninstall.sharedComponent'
                      : 'applicationUninstall.systemApplication'
                  )
                }}
              </MdStatusBadge>
            </span>
            <small>
              {{ candidate.publisher || candidate.primaryIdentifier }}
              <template v-if="candidate.version">
                · {{ t('applicationUninstall.version', { version: candidate.version }) }}
              </template>
            </small>
          </span>
        </button>
        <span
          v-if="candidate.applicationPath || canUninstallCandidate() || canRemoveRecord"
          class="application-actions"
        >
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
            v-if="canRemoveRecord"
            variant="ghost"
            destructive
            :disabled="busy"
            :label="t('applicationUninstall.removeRecord')"
            @click.stop="emit('removeRecord')"
          >
            <MdIcon :name="ICON_NAMES.trash" :size="16" />
          </MdIconAction>
          <MdIconAction
            v-else-if="canUninstallCandidate()"
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
        <strong class="application-size md-result-primary" :title="displayedSizeHint()">
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
      <p
        v-if="candidate.platform === 'macosBundle' && candidate.capability === 'requiresElevation'"
        class="association-warning"
      >
        <MdIcon :name="ICON_NAMES.info" :size="14" />
        {{ t('applicationUninstall.requiresElevationDescriptionMacos') }}
      </p>
      <MdResultTableHierarchy
        v-if="candidate.components.length || candidate.possibleRelatedPaths.length || showUnavailableEntry"
      >
        <MdApplicationUninstallDetailRow
          v-if="showUnavailableEntry"
          class="application-record-actions"
          :icon="ICON_NAMES.application"
          :title="t(applicationUnavailableTitleKey(candidate.uninstallDiagnostic))"
        >
          <template #actions>
            <Button
              v-if="candidate.platform === 'windowsRegistry'"
              class="application-settings-link"
              variant="ghost"
              size="sm"
              :disabled="busy"
              @click="emit('openWindowsSettings')"
            >
              {{ t('applicationUninstall.openWindowsInstalledApps') }}
              <MdIcon :name="ICON_NAMES.external" :size="14" />
            </Button>
          </template>
        </MdApplicationUninstallDetailRow>
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
          <strong class="component-size md-result-primary" :title="displayedSizeHint()">
            {{ displayedComponentSize(component) }}
          </strong>
        </MdResultTableRow>
        <MdApplicationUninstallDetailRow
          v-for="path in candidate.possibleRelatedPaths"
          :key="path"
          class="possible-related-location"
          :icon="ICON_NAMES.folder"
          :title="t('applicationUninstall.possibleRelatedLocations')"
          :description="PathUtils.display(path)"
          :description-title="path"
        >
          <template #actions>
            <MdIconAction
              variant="ghost"
              :label="t('applicationUninstall.showLocation')"
              :aria-label="t('applicationUninstall.showPossibleRelatedLocation', { application: candidate.name })"
              @click="emit('open', path)"
            >
              <MdIcon :name="ICON_NAMES.folder" :size="16" />
            </MdIconAction>
          </template>
        </MdApplicationUninstallDetailRow>
        <template v-if="candidate.possibleRelatedPaths.length" #footer>
          <p class="related-location-note">
            <MdIcon :name="ICON_NAMES.info" :size="12" />
            {{ t('applicationUninstall.possibleRelatedLocationsDescription') }}
          </p>
        </template>
      </MdResultTableHierarchy>
    </div>
  </article>
</template>

<style scoped src="./md-application-uninstall-row.css"></style>
