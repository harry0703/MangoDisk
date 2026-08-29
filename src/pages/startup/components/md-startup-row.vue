<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';

import MdApplicationIcon from '@/components/custom/md-application-icon.vue';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdResultTableHierarchy from '@/components/custom/md-result-table-hierarchy.vue';
import MdResultTableRow from '@/components/custom/md-result-table-row.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import type { StartupArtifact, StartupOwnerGroup } from '@/lib/models/startup';
import { ICON_NAMES } from '@/lib/models/ui';
import { FormatUtils } from '@/lib/utils/format';
import { StartupCommandUtils } from '@/lib/utils/startup-command';

import {
  canManageStartupArtifact,
  isRemovableOrphanStartupArtifact,
  nextStartupDesiredState,
  startupArtifactRevealPath,
  type StartupManageableState,
} from '../startup-view';

const props = defineProps<{
  group: StartupOwnerGroup;
  artifacts: StartupArtifact[];
  iconSrc?: string;
  subtitle: string | null;
  startTiming: string;
  state: StartupManageableState;
  revealPath: string | null;
  isWindows: boolean;
  isMacOs: boolean;
  expanded: boolean;
  busy: boolean;
  changing: boolean;
  copiedActionKey: string | null;
  aiRecommended?: boolean;
  aiRecommendedItemIds?: string[];
}>();
const emit = defineEmits<{
  toggleExpanded: [];
  toggleGroup: [];
  toggleArtifact: [artifact: StartupArtifact];
  reveal: [path: string];
  copy: [request: { actionKey: string; value: string }];
  openSystemSettings: [];
  removeOrphans: [];
}>();
const { locale, t } = useI18n({ useScope: 'global' });

const manageableArtifacts = computed(() => props.artifacts.filter(canManageStartupArtifact));
const groupManageable = computed(() => manageableArtifacts.value.length > 0);
const systemManaged = computed(() => props.artifacts.some(artifact => artifact.controlCapability === 'systemManaged'));
const hasMultipleArtifacts = computed(() => props.artifacts.length > 1);
const removableOrphans = computed(() => props.artifacts.filter(isRemovableOrphanStartupArtifact));

function targetCommand(artifact: StartupArtifact): string {
  return StartupCommandUtils.display(artifact, false);
}

function copyActionKey(artifact: StartupArtifact, field: 'configuration' | 'command'): string {
  return `${artifact.itemId}:${field}`;
}

function isCopied(artifact: StartupArtifact, field: 'configuration' | 'command'): boolean {
  return props.copiedActionKey === copyActionKey(artifact, field);
}

function localizedDiagnostics(artifact: StartupArtifact): string {
  return artifact.diagnostics.map(value => t(`startup.diagnostics.${value}`)).join(t('startup.valueSeparator'));
}
</script>

<template>
  <article class="startup-result-row">
    <MdResultTableRow class="startup-row-line" :data-expanded="expanded">
      <div class="startup-main" @click="emit('toggleExpanded')">
        <button
          class="startup-disclosure"
          type="button"
          :aria-expanded="expanded"
          :aria-label="t('startup.viewDetails', { name: group.name })"
          @click.stop="emit('toggleExpanded')"
        >
          <MdApplicationIcon :src="iconSrc" :platform="isWindows ? 'windowsRegistry' : 'macosBundle'" :size="40" />
          <span class="startup-identity">
            <span class="startup-name-line">
              <strong class="md-result-primary">{{ group.name }}</strong>
              <span v-if="aiRecommended" class="ai-badge" :title="t('largeFiles.aiAdvisorBadge')">
                <MdIcon :name="ICON_NAMES.smartSelect" :size="11" />
                <span>{{ t('largeFiles.aiAdvisorBadge') }}</span>
              </span>
            </span>
            <small v-if="subtitle || group.version">
              <template v-if="subtitle">{{ subtitle }}</template>
              <template v-if="group.version">
                <template v-if="subtitle"> · </template>
                {{ t('startup.detail.versionValue', { version: group.version }) }}</template
              >
            </small>
          </span>
        </button>

        <span
          v-if="revealPath || removableOrphans.length"
          class="startup-actions"
          :class="{ 'has-cleanup': removableOrphans.length }"
        >
          <button
            v-if="removableOrphans.length"
            class="startup-cleanup-action"
            type="button"
            :disabled="busy"
            @click.stop="emit('removeOrphans')"
          >
            <MdIcon :name="ICON_NAMES.trash" :size="14" />
            {{ t('startup.cleanup.action') }}
          </button>
          <MdIconAction
            v-if="revealPath"
            class="startup-location-action"
            variant="ghost"
            :label="t('startup.showLocation')"
            :aria-label="t('startup.showNamedLocation', { name: group.name })"
            @click.stop="emit('reveal', revealPath!)"
          >
            <MdIcon :name="ICON_NAMES.folder" :size="16" />
          </MdIconAction>
        </span>

        <span class="startup-item-count">{{ t('startup.itemCount', { count: artifacts.length }) }}</span>

        <button
          v-if="groupManageable"
          class="startup-switch"
          type="button"
          role="switch"
          :aria-label="
            t(state === 'disabled' ? 'startup.change.enableNamedGroup' : 'startup.change.disableNamedGroup', {
              name: group.name,
            })
          "
          :aria-checked="state === 'enabled'"
          :aria-busy="changing"
          :data-state="state"
          :disabled="busy || state === 'mixed'"
          @click.stop="emit('toggleGroup')"
        >
          <span class="switch-thumb">
            <span v-if="changing" class="switch-spinner md-operational-motion" aria-hidden="true" />
          </span>
        </button>
        <span v-else class="startup-state" :data-state="state">
          {{ t(`startup.configuredStates.${state}`) }}
        </span>

        <MdIconAction
          appearance="unstyled"
          class="startup-expand"
          :label="t('startup.viewDetails', { name: group.name })"
          :aria-expanded="expanded"
          @click.stop="emit('toggleExpanded')"
        >
          <MdIcon class="startup-chevron" :class="{ expanded }" :name="ICON_NAMES.chevronDown" :size="17" />
        </MdIconAction>
      </div>
    </MdResultTableRow>

    <div v-if="expanded" class="startup-details">
      <div v-if="!groupManageable && !removableOrphans.length" class="startup-management-note">
        <span>
          <MdIcon :name="ICON_NAMES.info" :size="15" />
          {{ isMacOs && systemManaged ? t('startup.detail.systemManaged') : t('startup.detail.viewOnly') }}
        </span>
        <Button
          v-if="isMacOs && systemManaged"
          variant="outline"
          size="sm"
          type="button"
          @click="emit('openSystemSettings')"
        >
          <MdIcon :name="ICON_NAMES.external" :size="14" />
          {{ t('startup.detail.openLoginItemsSettings') }}
        </Button>
      </div>

      <dl v-if="hasMultipleArtifacts" class="startup-detail-list startup-group-detail-list">
        <div class="startup-detail-row">
          <dt>{{ t('startup.detail.timing') }}</dt>
          <dd>{{ startTiming }}</dd>
        </div>
      </dl>

      <component
        :is="hasMultipleArtifacts ? MdResultTableHierarchy : 'div'"
        class="startup-artifact-list"
        :class="{ 'is-grouped': hasMultipleArtifacts }"
      >
        <article v-for="artifact in artifacts" :key="artifact.itemId" class="startup-native-item">
          <MdResultTableRow v-if="hasMultipleArtifacts" class="startup-native-row">
            <span class="startup-native-icon">
              <MdIcon :name="ICON_NAMES.startup" :size="16" />
            </span>
            <span class="startup-native-identity">
              <strong class="md-result-primary">{{ artifact.displayName }}</strong>
              <span
                v-if="aiRecommendedItemIds?.includes(artifact.itemId)"
                class="ai-badge"
                :title="t('largeFiles.aiAdvisorBadge')"
              >
                <MdIcon :name="ICON_NAMES.smartSelect" :size="11" />
                <span>{{ t('largeFiles.aiAdvisorBadge') }}</span>
              </span>
            </span>
            <span v-if="startupArtifactRevealPath(artifact)" class="startup-native-actions">
              <MdIconAction
                variant="ghost"
                :label="t('startup.showLocation')"
                :aria-label="t('startup.showNamedLocation', { name: artifact.displayName })"
                @click="emit('reveal', startupArtifactRevealPath(artifact)!)"
              >
                <MdIcon :name="ICON_NAMES.folder" :size="15" />
              </MdIconAction>
            </span>
            <button
              v-if="canManageStartupArtifact(artifact)"
              class="startup-switch"
              type="button"
              role="switch"
              :aria-label="
                t(
                  nextStartupDesiredState(artifact.configuredState) === 'enabled'
                    ? 'startup.change.enableNamedGroup'
                    : 'startup.change.disableNamedGroup',
                  { name: artifact.displayName }
                )
              "
              :aria-checked="artifact.configuredState === 'enabled'"
              :aria-busy="changing"
              :data-state="artifact.configuredState"
              :disabled="busy"
              @click="emit('toggleArtifact', artifact)"
            >
              <span class="switch-thumb">
                <span v-if="changing" class="switch-spinner md-operational-motion" aria-hidden="true" />
              </span>
            </button>
            <span v-else class="startup-native-state">
              {{ t(`startup.configuredStates.${artifact.configuredState}`) }}
            </span>
          </MdResultTableRow>

          <dl class="startup-detail-list">
            <div v-if="!hasMultipleArtifacts" class="startup-detail-row">
              <dt>{{ t('startup.detail.timing') }}</dt>
              <dd>{{ startTiming }}</dd>
            </div>
            <div class="startup-detail-row">
              <dt>{{ t('startup.detail.source') }}</dt>
              <dd>
                {{ t(`startup.sourceKinds.${artifact.sourceKind}`) }} · {{ t(`startup.scopes.${artifact.scope}`) }}
              </dd>
            </div>
            <div v-if="artifact.configurationPath" class="startup-detail-row">
              <dt>{{ t('startup.detail.configuration') }}</dt>
              <dd class="startup-target-value">
                <span class="startup-target-text" :title="artifact.configurationPath">
                  {{ artifact.configurationPath }}
                </span>
                <span class="startup-target-actions">
                  <MdIconAction
                    variant="ghost"
                    :label="t('startup.showLocation')"
                    :aria-label="t('startup.showNamedLocation', { name: artifact.displayName })"
                    @click="emit('reveal', artifact.configurationPath)"
                  >
                    <MdIcon :name="ICON_NAMES.folder" :size="13" />
                  </MdIconAction>
                  <MdIconAction
                    variant="ghost"
                    :label="
                      t(isCopied(artifact, 'configuration') ? 'startup.copiedToClipboard' : 'startup.copyToClipboard')
                    "
                    :aria-label="t('startup.copyConfiguration', { name: artifact.displayName })"
                    @click="
                      emit('copy', {
                        actionKey: copyActionKey(artifact, 'configuration'),
                        value: artifact.configurationPath!,
                      })
                    "
                  >
                    <MdIcon
                      :name="isCopied(artifact, 'configuration') ? ICON_NAMES.check : ICON_NAMES.copy"
                      :size="13"
                    />
                  </MdIconAction>
                </span>
              </dd>
            </div>
            <div class="startup-detail-row">
              <dt>{{ t('startup.detail.command') }}</dt>
              <dd class="startup-target-value">
                <span class="startup-target-text" :title="targetCommand(artifact) || undefined">
                  {{ targetCommand(artifact) || '—' }}
                </span>
                <span v-if="targetCommand(artifact)" class="startup-target-actions">
                  <MdIconAction
                    v-if="!hasMultipleArtifacts && startupArtifactRevealPath(artifact) && !artifact.configurationPath"
                    variant="ghost"
                    :label="t('startup.showLocation')"
                    :aria-label="t('startup.showNamedLocation', { name: artifact.displayName })"
                    @click="emit('reveal', startupArtifactRevealPath(artifact)!)"
                  >
                    <MdIcon :name="ICON_NAMES.folder" :size="13" />
                  </MdIconAction>
                  <MdIconAction
                    variant="ghost"
                    :label="t(isCopied(artifact, 'command') ? 'startup.copiedToClipboard' : 'startup.copyToClipboard')"
                    :aria-label="t('startup.copyCommand', { name: artifact.displayName })"
                    @click="
                      emit('copy', {
                        actionKey: copyActionKey(artifact, 'command'),
                        value: targetCommand(artifact),
                      })
                    "
                  >
                    <MdIcon :name="isCopied(artifact, 'command') ? ICON_NAMES.check : ICON_NAMES.copy" :size="13" />
                  </MdIconAction>
                </span>
              </dd>
            </div>
            <div v-if="artifact.runtimeState !== 'unknown'" class="startup-detail-row">
              <dt>{{ t('startup.detail.runtime') }}</dt>
              <dd>{{ t(`startup.runtimeStates.${artifact.runtimeState}`) }}</dd>
            </div>
            <div v-if="artifact.trust !== 'unknown'" class="startup-detail-row">
              <dt>{{ t('startup.detail.trust') }}</dt>
              <dd>{{ t(`startup.trustStates.${artifact.trust}`) }}</dd>
            </div>
            <div v-if="artifact.modifiedAtMs" class="startup-detail-row">
              <dt>{{ t('startup.detail.modified') }}</dt>
              <dd>{{ FormatUtils.dateTime(artifact.modifiedAtMs, locale) }}</dd>
            </div>
            <div v-if="artifact.diagnostics.length" class="startup-detail-row is-warning">
              <dt>{{ t('startup.detail.diagnostics') }}</dt>
              <dd>{{ localizedDiagnostics(artifact) }}</dd>
            </div>
          </dl>
        </article>
      </component>
    </div>
  </article>
</template>

<style scoped src="./md-startup-row.css"></style>
