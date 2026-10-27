<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { I18nT, useI18n } from 'vue-i18n';
import { toast } from 'vue-sonner';
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from 'reka-ui';
import { Dialog, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import MdCheckbox from '@/components/custom/md-checkbox.vue';
import MdConfirmDialog from '@/components/custom/md-confirm-dialog.vue';
import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdSpinner from '@/components/custom/md-spinner.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { Button } from '@/components/ui/button';
import { useWindowsMonitoringStore } from '@/stores/windows-monitoring-store';
import { useResidentSettingsStore } from '@/stores/resident-settings-store';
import { LinkService } from '@/lib/services/shell/link-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import type {
  WindowsDependencyIssue,
  WindowsMonitoringOutcome,
} from '@/lib/models/system-resources/windows-monitoring';

const outcomeLabels: Record<Exclude<WindowsMonitoringOutcome, 'ready' | 'removed' | 'userCancelled'>, string> = {
  unsupportedHardware: 'windowsMonitoring.outcomes.unsupportedHardware',
  busy: 'windowsMonitoring.outcomes.busy',
  verificationFailed: 'windowsMonitoring.outcomes.verificationFailed',
  certificateUntrusted: 'windowsMonitoring.outcomes.certificateUntrusted',
  installationFailed: 'windowsMonitoring.outcomes.installationFailed',
  serviceUnavailable: 'windowsMonitoring.outcomes.serviceUnavailable',
  restartRequired: 'windowsMonitoring.outcomes.restartRequired',
  driverRemovalFailed: 'windowsMonitoring.outcomes.driverRemovalFailed',
  removalFailed: 'windowsMonitoring.outcomes.removalFailed',
};

const dependencyLabels: Record<WindowsDependencyIssue['reason'], string> = {
  missing: 'windowsMonitoring.dependency.missing',
  incompatible: 'windowsMonitoring.dependency.incompatible',
  certificateUntrusted: 'windowsMonitoring.dependency.certificateUntrusted',
  verificationFailed: 'windowsMonitoring.dependency.verificationFailed',
  unavailable: 'windowsMonitoring.dependency.unavailable',
};

const props = defineProps<{ open: boolean; selectDisplayItem?: boolean }>();
const emit = defineEmits<{ 'update:open': [open: boolean] }>();
const { t } = useI18n({ useScope: 'global' });
const monitoring = useWindowsMonitoringStore();
const settings = useResidentSettingsStore();
const pawnIoWebsite = 'https://pawnio.eu/';
const removeConfirmation = ref(false);
const removeDriver = ref(false);
const driverRemovalPending = ref(false);
const enabled = computed(() => settings.preferences?.windowsCpuTemperatureEnabled ?? false);
const busy = computed(() => monitoring.busy || settings.saving);
const installed = computed(() => monitoring.observation?.serviceInstalled === true);
const supported = computed(() => monitoring.observation?.state !== 'unsupportedHardware');
const collecting = computed(() => monitoring.observation?.state === 'ready' && enabled.value);
const diagnostic = computed(() => {
  const outcome = monitoring.outcome;
  if ((outcome === 'certificateUntrusted' || outcome === 'verificationFailed') && monitoring.nativeCode)
    return {
      label: outcomeLabels[outcome],
      code: monitoring.nativeCode,
      certificateUntrusted: outcome === 'certificateUntrusted',
      failed: true,
    };
  const issue = monitoring.observation?.dependencyIssue;
  return issue
    ? {
        label: dependencyLabels[issue.reason],
        code: issue.nativeCode,
        certificateUntrusted: issue.reason === 'certificateUntrusted',
        failed: issue.reason === 'certificateUntrusted' || issue.reason === 'verificationFailed',
      }
    : null;
});
const needsRepair = computed(
  () => installed.value && ['unavailable', 'incompatible'].includes(monitoring.observation?.state ?? '')
);
const primaryLabel = computed(() =>
  !installed.value
    ? 'windowsMonitoring.install'
    : needsRepair.value
      ? 'windowsMonitoring.repair'
      : 'windowsMonitoring.enable'
);
const description = computed(() => {
  const state = monitoring.observation?.state;
  if (state === 'unsupportedHardware') return 'windowsMonitoring.unsupportedHint';
  if (state === 'unavailable' || state === 'incompatible') return 'windowsMonitoring.repairHint';
  if (collecting.value) return 'windowsMonitoring.collectionHint';
  return installed.value ? 'windowsMonitoring.enableHint' : 'windowsMonitoring.installBody';
});
const needsEnable = computed(
  () =>
    !installed.value ||
    needsRepair.value ||
    !enabled.value ||
    (props.selectDisplayItem && !settings.preferences?.metrics.some(row => row.id === 'cpuTemperature' && row.enabled))
);
watch(
  () => monitoring.outcome,
  outcome => {
    if (!props.open || !outcome || outcome === 'ready' || outcome === 'removed' || outcome === 'userCancelled') return;
    if (
      [
        'verificationFailed',
        'certificateUntrusted',
        'installationFailed',
        'serviceUnavailable',
        'driverRemovalFailed',
        'removalFailed',
      ].includes(outcome)
    ) {
      toast.error(t(outcomeLabels[outcome]));
    } else {
      toast.warning(t(outcomeLabels[outcome]));
    }
  }
);
watch(
  () => settings.error,
  error => {
    if (props.open && error) toast.error(t('systemStatus.saveFailed'));
  }
);
watch(
  () => props.open,
  open => {
    if (open) {
      removeConfirmation.value = false;
      removeDriver.value = false;
      monitoring.outcome = null;
      monitoring.nativeCode = null;
      void monitoring.load();
    }
  }
);
function close(open: boolean) {
  if (!busy.value) emit('update:open', open);
}
async function openPawnIoWebsite() {
  try {
    await LinkService.open(pawnIoWebsite);
  } catch (error) {
    toast.error(t('windowsMonitoring.linkFailed'));
    LoggerService.warn('monitoring', 'windows_monitoring_vendor_link_failed', { url: pawnIoWebsite, error });
  }
}
async function enable() {
  if (busy.value || monitoring.loading || !settings.draft) return;
  await monitoring.load();
  if (!props.open || monitoring.loading || monitoring.busy || !settings.draft) return;
  if (monitoring.observation?.state !== 'ready' && (await monitoring.setup()) !== 'ready') return;
  await settings.change({
    windowsCpuTemperatureEnabled: true,
    ...(props.selectDisplayItem
      ? {
          metrics: settings.draft.metrics.map(row => ({
            ...row,
            enabled: row.id === 'cpuTemperature' ? true : row.enabled,
          })),
        }
      : {}),
  });
}
async function remove() {
  if (busy.value || !settings.draft) return;
  const outcome = await monitoring.setup(true, removeDriver.value);
  if (outcome === 'removed') driverRemovalPending.value = false;
  else if (removeDriver.value && ['driverRemovalFailed', 'restartRequired'].includes(outcome)) {
    driverRemovalPending.value = true;
  }
  if (
    (['removed', 'driverRemovalFailed', 'restartRequired'].includes(outcome) ||
      (['removalFailed', 'certificateUntrusted', 'verificationFailed'].includes(outcome) &&
        monitoring.observation?.state === 'notInstalled')) &&
    !monitoring.observation?.serviceInstalled
  )
    await settings.change({
      windowsCpuTemperatureEnabled: false,
      metrics: settings.draft.metrics.map(row => ({
        ...row,
        enabled: row.id === 'cpuTemperature' ? false : row.enabled,
      })),
    });
  removeConfirmation.value = false;
}
async function repair() {
  if (busy.value) return;
  await monitoring.setup();
}
</script>

<template>
  <Dialog :open="open" @update:open="close">
    <MdDialogContent
      size="compact"
      :aria-busy="busy || monitoring.loading"
      :show-close="!busy"
      @interact-outside.prevent
      @escape-key-down="busy && $event.preventDefault()"
    >
      <MdDialogHeader>
        <DialogTitle>{{ t('windowsMonitoring.title') }}</DialogTitle>
      </MdDialogHeader>
      <div class="space-y-5 overflow-y-auto px-5 pb-5 pt-2 text-content-body">
        <div class="space-y-3">
          <p v-if="monitoring.observation" role="status">
            <span
              class="inline-flex items-center rounded bg-muted px-1.5 py-0.5 text-content-meta font-medium leading-4"
              :class="
                monitoring.needsAttention
                  ? 'text-destructive-text'
                  : collecting
                    ? 'text-primary-text'
                    : 'text-muted-foreground'
              "
            >
              {{ t(monitoring.statusLabel(enabled) ?? 'windowsMonitoring.states.notEnabled') }}
            </span>
          </p>
          <DialogDescription class="text-[length:var(--font-content-body)] text-foreground">{{
            t(description)
          }}</DialogDescription>
          <p v-if="collecting" class="text-content-body text-muted-foreground">
            {{ t('windowsMonitoring.displayHint') }}
          </p>
          <ul
            v-else-if="!installed && supported"
            class="list-disc space-y-2 pl-5 text-content-body text-muted-foreground"
          >
            <li>{{ t('windowsMonitoring.installHint') }}</li>
            <I18nT keypath="windowsMonitoring.driverHint" tag="li" scope="global">
              <template #driver>
                <a
                  :href="pawnIoWebsite"
                  target="_blank"
                  rel="noopener noreferrer"
                  class="inline-flex items-center gap-1 rounded-sm text-primary-text underline underline-offset-2 hover:no-underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  :aria-label="t('windowsMonitoring.pawnIoWebsite')"
                  @click.prevent="openPawnIoWebsite"
                  >PawnIO<MdIcon name="external" :size="12" aria-hidden="true"
                /></a>
              </template>
            </I18nT>
          </ul>
        </div>
        <div v-if="diagnostic" class="space-y-2 rounded-lg border border-border p-3" role="status">
          <p :class="diagnostic.failed ? 'text-destructive-text' : 'text-foreground'">
            {{ t(diagnostic.label) }}
          </p>
          <p v-if="diagnostic.code" class="text-content-secondary">
            {{ t('windowsMonitoring.errorCode') }} <code class="select-text">{{ diagnostic.code }}</code>
          </p>
          <p v-if="diagnostic.certificateUntrusted" class="text-content-secondary text-muted-foreground">
            {{ t('windowsMonitoring.certificateHint') }}
          </p>
        </div>
        <div class="flex items-center gap-1 text-content-meta text-muted-foreground">
          <span>{{ t('windowsMonitoring.supportSummary') }}</span>
          <MdTooltip :text="t('windowsMonitoring.supportHint')">
            <button type="button" class="md-help-action" :aria-label="t('windowsMonitoring.supportHint')">
              <MdIcon name="info" :size="14" />
            </button>
          </MdTooltip>
        </div>
      </div>
      <MdDialogFooter align="between" class="flex-wrap gap-y-3">
        <DropdownMenuRoot v-if="installed || driverRemovalPending">
          <DropdownMenuTrigger as-child>
            <Button variant="ghost" :disabled="busy">
              <MdSpinner v-if="monitoring.busy && !needsEnable" size="small" />
              {{ t('windowsMonitoring.more') }}
              <MdIcon name="chevronDown" :size="14" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuPortal>
            <DropdownMenuContent
              align="start"
              :side-offset="6"
              class="z-50 min-w-36 max-w-[calc(100vw-32px)] rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-xl"
              @close-auto-focus="removeConfirmation && $event.preventDefault()"
            >
              <DropdownMenuItem
                v-if="installed && !needsRepair"
                class="flex cursor-default flex-col items-start gap-1 rounded-md px-3 py-2 text-content-secondary outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50"
                :disabled="busy || !supported"
                @select="repair"
              >
                <span>{{ t('windowsMonitoring.repair') }}</span>
                <span class="text-content-meta text-muted-foreground">{{
                  t('windowsMonitoring.repairDescription')
                }}</span>
              </DropdownMenuItem>
              <DropdownMenuItem
                class="cursor-default rounded-md px-3 py-2 text-content-secondary text-destructive-text outline-none focus:bg-destructive/10 data-[disabled]:pointer-events-none data-[disabled]:opacity-50"
                :disabled="busy"
                @select="
                  removeDriver = false;
                  removeConfirmation = true;
                "
                >{{ t('windowsMonitoring.remove') }}</DropdownMenuItem
              >
            </DropdownMenuContent>
          </DropdownMenuPortal>
        </DropdownMenuRoot>
        <div class="ml-auto flex flex-wrap justify-end gap-2">
          <Button :variant="needsEnable ? 'outline' : 'default'" :disabled="busy" @click="close(false)">{{
            t(needsEnable ? 'common.cancel' : 'systemStatus.done')
          }}</Button>
          <Button
            v-if="needsEnable"
            :disabled="busy || monitoring.loading || !monitoring.observation || !supported"
            @click="enable"
          >
            <MdSpinner v-if="busy || monitoring.loading" size="small" />
            {{ t(primaryLabel) }}
          </Button>
        </div>
      </MdDialogFooter>
    </MdDialogContent>
  </Dialog>
  <MdConfirmDialog
    v-model:open="removeConfirmation"
    :title="t('windowsMonitoring.remove')"
    :description="t('windowsMonitoring.removeHint')"
    description-class="text-[length:var(--font-content-body)]"
    :cancel-label="t('common.cancel')"
    :confirm-label="t('windowsMonitoring.remove')"
    confirm-variant="destructive"
    :busy="busy"
    @confirm="remove"
  >
    <div class="space-y-2">
      <label for="remove-pawnio-driver" class="flex items-center gap-2 text-content-body">
        <MdCheckbox
          id="remove-pawnio-driver"
          :model-value="removeDriver"
          :disabled="busy"
          @update:model-value="removeDriver = $event === true"
        />
        {{ t('windowsMonitoring.removeDriver') }}
      </label>
      <p class="pl-6 text-content-body leading-relaxed text-muted-foreground">
        {{ t('windowsMonitoring.removeDriverHint') }}
      </p>
    </div>
  </MdConfirmDialog>
</template>
