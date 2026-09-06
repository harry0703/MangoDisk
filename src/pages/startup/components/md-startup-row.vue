<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';

import MdAiAction from '@/components/custom/md-ai-action.vue';
import MdApplicationIcon from '@/components/custom/md-application-icon.vue';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdStatusBadge from '@/components/custom/md-status-badge.vue';
import MdResultTableHierarchy from '@/components/custom/md-result-table-hierarchy.vue';
import MdResultTableRow from '@/components/custom/md-result-table-row.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import type { StartupArtifact, StartupOwnerGroup } from '@/lib/models/startup';
import type { WindowsStartupTool } from '@/lib/services/windows-startup-tool-service';
import { ICON_NAMES } from '@/lib/models/ui';
import * as FormatUtils from '@/lib/utils/format';
import * as StartupCommandUtils from '@/lib/utils/startup-command';

import {
  canManageStartupArtifact,
  isManualCleanupStartupArtifact,
  manualCleanupToolForStartupArtifacts,
  nextStartupDesiredState,
  startupArtifactRevealPath,
  supportsStartupRemoval,
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
}>();
const emit = defineEmits<{
  toggleExpanded: [];
  toggleGroup: [];
  toggleArtifact: [artifact: StartupArtifact];
  reveal: [path: string];
  copy: [request: { actionKey: string; value: string }];
  openSystemSettings: [];
  openWindowsTool: [tool: WindowsStartupTool];
  removeItems: [];
  explain: [name: string, artifacts: StartupArtifact[]];
}>();
const { locale, t } = useI18n({ useScope: 'global' });

const manageableArtifacts = computed(() => props.artifacts.filter(canManageStartupArtifact));
const groupManageable = computed(() => manageableArtifacts.value.length > 0);
const systemManaged = computed(() => props.artifacts.some(artifact => artifact.controlCapability === 'systemManaged'));
const protectedService = computed(
  () =>
    props.isWindows &&
    props.artifacts.length > 0 &&
    props.artifacts.every(
      artifact => artifact.sourceKind === 'service' && artifact.controlCapability === 'systemManaged'
    )
);
const hasMultipleArtifacts = computed(() => props.artifacts.length > 1);
const sourceKinds = computed(() => [...new Set(props.artifacts.map(artifact => artifact.sourceKind))]);
const removableItems = computed(() => props.artifacts.filter(supportsStartupRemoval));
const manualCleanupArtifacts = computed(() => props.artifacts.filter(isManualCleanupStartupArtifact));
const manualCleanupTool = computed<WindowsStartupTool | null>(() =>
  manualCleanupToolForStartupArtifacts(props.artifacts)
);

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
    <MdResultTableRow class="startup-row-line md-ai-hover-row" :data-expanded="expanded">
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
            <strong class="md-result-primary" :title="group.name">{{ group.name }}</strong>
          </span>
        </button>

        <span
          class="startup-actions has-ai"
          :class="{ 'has-cleanup': removableItems.length, 'has-location': revealPath }"
        >
          <MdAiAction :name="group.name" :disabled="busy" @explain="emit('explain', group.name, artifacts)" />
          <MdIconAction
            v-if="removableItems.length"
            class="startup-cleanup-action"
            variant="ghost"
            destructive
            :label="t('startup.remove.action')"
            :aria-label="t('startup.remove.action')"
            :disabled="busy"
            @click.stop="emit('removeItems')"
          >
            <MdIcon :name="ICON_NAMES.trash" :size="16" />
          </MdIconAction>
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

        <span class="startup-source-slot">
          <MdStatusBadge
            v-if="sourceKinds.length"
            size="compact"
            :title="sourceKinds.length === 1 ? t(`startup.sourceKinds.${sourceKinds[0]}`) : t('startup.mixedSources')"
          >
            {{ sourceKinds.length === 1 ? t(`startup.sourceKinds.${sourceKinds[0]}`) : t('startup.mixedSources') }}
          </MdStatusBadge>
        </span>
        <!-- Reserve the count slot so single and grouped rows keep their controls aligned. -->
        <span class="startup-item-count">
          <template v-if="hasMultipleArtifacts">{{ t('startup.itemCount', { count: artifacts.length }) }}</template>
        </span>

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
      <div
        v-if="manualCleanupArtifacts.length || (!groupManageable && !removableItems.length)"
        class="startup-management-note"
      >
        <span>
          <MdIcon :name="ICON_NAMES.info" :size="15" />
          {{
            manualCleanupArtifacts.length
              ? t('startup.cleanup.manualGuidance')
              : protectedService
                ? t('startup.detail.protectedService')
                : isMacOs && systemManaged
                  ? t('startup.detail.systemManaged')
                  : t('startup.detail.viewOnly')
          }}
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
        <Button
          v-else-if="isWindows && manualCleanupTool"
          variant="outline"
          size="sm"
          type="button"
          @click="emit('openWindowsTool', manualCleanupTool)"
        >
          <MdIcon :name="ICON_NAMES.external" :size="14" />
          {{ t(`startup.cleanup.openTools.${manualCleanupTool}`) }}
        </Button>
      </div>

      <dl
        v-if="subtitle || group.version || hasMultipleArtifacts"
        class="startup-detail-list startup-group-detail-list"
      >
        <div v-if="subtitle" class="startup-detail-row">
          <dt>{{ t('startup.detail.description') }}</dt>
          <dd class="startup-description">{{ subtitle }}</dd>
        </div>
        <div v-if="group.version" class="startup-detail-row">
          <dt>{{ t('startup.detail.version') }}</dt>
          <dd class="startup-description">{{ group.version }}</dd>
        </div>
        <div v-if="hasMultipleArtifacts" class="startup-detail-row">
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
          <MdResultTableRow v-if="hasMultipleArtifacts" class="startup-native-row md-ai-hover-row">
            <span class="startup-native-icon">
              <MdIcon :name="ICON_NAMES.startup" :size="16" />
            </span>
            <span class="startup-native-identity">
              <strong class="md-result-primary">{{ artifact.displayName }}</strong>
              <MdStatusBadge size="compact">{{ t(`startup.sourceKinds.${artifact.sourceKind}`) }}</MdStatusBadge>
            </span>
            <span class="startup-native-actions">
              <MdAiAction
                :name="artifact.displayName"
                :disabled="busy"
                @explain="emit('explain', artifact.displayName, [artifact])"
              />
              <MdIconAction
                v-if="startupArtifactRevealPath(artifact)"
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
              <dd>
                {{
                  artifact.sourceKind === 'service' &&
                  artifact.configuredState === 'disabled' &&
                  artifact.runtimeState === 'running'
                    ? t('startup.serviceStillRunning')
                    : t(`startup.runtimeStates.${artifact.runtimeState}`)
                }}
              </dd>
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
