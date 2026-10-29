<script setup lang="ts">
import { cachedMetricValue } from '@/lib/utils/system-resources/metric-reading';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import MdConfirmDialog from '@/components/custom/md-confirm-dialog.vue';
import { ResidentService } from '@/lib/services/resident/resident-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { useTrayPanelStore } from '@/stores/tray-panel-store';
import { useI18n } from 'vue-i18n';
import type { ResourceReadings } from '@/lib/models/system-resources/system-resources';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdResourceCard from './md-resource-card.vue';
import MdResourceTrend from './md-resource-trend.vue';
import { METRIC_STATUS_KEYS } from '@/lib/models/system-resources/system-resources';
import { useResidentSettingsStore } from '@/stores/resident-settings-store';
import { Select, SelectContent, SelectItem } from '@/components/ui/select';
import { SelectTrigger } from 'reka-ui';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import MdResourceFacts from './md-resource-facts.vue';
import MdNetworkAddresses from './md-network-addresses.vue';

const props = defineProps<{ reading: ResourceReadings; active: boolean }>();
const { t, n, locale } = useI18n({ useScope: 'global' });
const network = computed(() => cachedMetricValue(props.reading.network, props.reading.observedAtMs));
const settings = useResidentSettingsStore();
const deviceMenuOpen = ref(false);
let disposed = false;
let unsubscribe: (() => void) | null = null;
let preferencesRefreshPending = false;
const subscriptionFailed = ref(false);
const connectingPreferences = ref(false);
function refreshPreferences() {
  if (disposed || !preferencesRefreshPending || settings.loading || settings.saving) return;
  // A native notification can precede an older read/save response. Consume it
  // once the transaction settles; a failed refresh must not start a retry loop.
  preferencesRefreshPending = false;
  void settings.load();
}
function requestPreferencesRefresh() {
  preferencesRefreshPending = true;
  refreshPreferences();
}
watch([() => settings.loading, () => settings.saving], refreshPreferences, { flush: 'post' });
async function reloadPreferences() {
  if (disposed || connectingPreferences.value) return;
  connectingPreferences.value = true;
  try {
    if (!unsubscribe) {
      const stop = await ResidentService.onPreferencesChanged(requestPreferencesRefresh);
      if (disposed) stop();
      else unsubscribe = stop;
      subscriptionFailed.value = false;
    }
  } catch (error) {
    subscriptionFailed.value = true;
    LoggerService.warn('monitoring', 'network_preferences_subscription_failed', { error });
  } finally {
    connectingPreferences.value = false;
  }
  if (!disposed) requestPreferencesRefresh();
}
onMounted(reloadPreferences);
onBeforeUnmount(() => {
  disposed = true;
  unsubscribe?.();
});
const interfaces = computed(() => props.reading.interfaces);
const selectedInterface = computed(() => settings.draft?.networkInterface ?? 'automatic');
const selectedInterfaceLabel = computed(() => {
  if (!settings.draft) return t(settings.error ? 'systemStatus.catalogueFailed' : 'systemStatus.loading');
  const id = settings.draft.networkInterface;
  if (!id) return t('systemStatus.automatic');
  const item = interfaces.value.find(item => item.id === id);
  if (!item) return t('systemStatus.savedDisconnected');
  return `${item.name}${item.connected ? '' : ` · ${t('systemStatus.disconnected')}`}`;
});
const missingInterface = computed(() => {
  const id = settings.draft?.networkInterface;
  return id && !interfaces.value.some(item => item.id === id) ? id : null;
});
const rates = computed(() =>
  (['download', 'upload'] as const).map(direction => {
    const value = network.value;
    const bytes = value
      ? direction === 'download'
        ? value.receivedBytesPerSecond
        : value.transmittedBytesPerSecond
      : null;
    const formatted = bytes === null ? '—' : ByteSizeService.bytes(bytes);
    const separator = formatted.lastIndexOf(' ');
    return {
      direction,
      value: separator < 0 ? formatted : formatted.slice(0, separator),
      unit: separator < 0 ? '' : `${formatted.slice(separator + 1)}/s`,
    };
  })
);

function speed(bits: number | null | undefined): string {
  if (bits === null || bits === undefined || !Number.isFinite(bits) || bits <= 0) return '—';
  const scale = bits >= 1e9 ? 1e9 : bits >= 1e6 ? 1e6 : 1e3;
  return `${n(Number((bits / scale).toFixed(1)))} ${scale === 1e9 ? 'Gbps' : scale === 1e6 ? 'Mbps' : 'Kbps'}`;
}
const connection = computed(() => {
  const value = network.value;
  if (!value) return '—';
  if (value.interface.kind === 'wifi' && value.receiveLinkBps === null) return speed(value.transmitLinkBps);
  return value.receiveLinkBps === value.transmitLinkBps
    ? speed(value.receiveLinkBps)
    : `↓ ${speed(value.receiveLinkBps)} · ↑ ${speed(value.transmitLinkBps)}`;
});
const cumulative = computed(() => network.value?.cumulative);
const trafficHint = computed(() => {
  const started = cumulative.value?.startedAtMs;
  if (!started) return t('networkDetails.cumulativeUnavailable');
  const since = started
    ? new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }).format(started)
    : '—';
  return t('networkDetails.cumulativeHint', { since });
});
const trafficFacts = computed(() => [
  {
    key: 'received',
    label: t('networkDetails.totalReceived'),
    total: cumulative.value?.receivedBytes,
    session: network.value?.receivedBytesSinceStart,
  },
  {
    key: 'transmitted',
    label: t('networkDetails.totalTransmitted'),
    total: cumulative.value?.transmittedBytes,
    session: network.value?.transmittedBytesSinceStart,
  },
]);
const resetTarget = ref<{ id: string; name: string; startedAtMs: number } | null>(null);
const resetting = ref(false);
const resetError = ref(false);
function requestReset() {
  resetError.value = false;
  const value = network.value;
  if (value?.cumulative)
    resetTarget.value = {
      id: value.interface.id,
      name: value.interface.name,
      startedAtMs: value.cumulative.startedAtMs,
    };
}
async function resetTraffic() {
  const target = resetTarget.value;
  if (!target || resetting.value) return;
  resetting.value = true;
  resetError.value = false;
  try {
    const reading = await ResidentService.resetNetworkTraffic(target.id, target.startedAtMs);
    useTrayPanelStore().accept(reading);
    resetTarget.value = null;
  } catch {
    resetError.value = true;
    LoggerService.warn('monitoring', 'network_traffic_reset_failed');
  } finally {
    resetting.value = false;
  }
}
watch(
  () => props.active,
  active => {
    if (!active) {
      deviceMenuOpen.value = false;
      if (!resetting.value) resetTarget.value = null;
    }
  }
);
const interfaceFacts = computed(() => {
  const value = network.value?.interface;
  const rows = [
    {
      key: 'mac',
      label: t('networkDetails.hardwareAddress'),
      value: value?.details?.hardwareAddress ?? '—',
      unit: '',
      copy: true,
    },
    {
      key: 'ipv4',
      label: 'IPv4',
      value: value?.details?.addresses.filter(address => !address.includes(':')).join(', ') || '—',
      unit: '',
      copy: true,
      truncate: true,
    },
    {
      key: 'ipv6',
      label: 'IPv6',
      value: value?.details?.addresses.filter(address => address.includes(':')).join(', ') || '—',
      unit: '',
      copy: true,
      truncate: true,
    },
  ];
  const wifi = value?.details?.wifi;
  if (wifi) {
    rows.push({
      key: 'standard',
      label: t('networkDetails.standard'),
      value: wifi.standard ?? '—',
      unit: '',
      copy: false,
    });
    const channel = [
      wifi.bandGhz === null ? '' : `${n(wifi.bandGhz)} GHz`,
      wifi.channelWidthMhz === null ? '' : `${wifi.channelWidthMhz} MHz`,
    ]
      .filter(Boolean)
      .join(', ');
    rows.push({
      key: 'channel',
      label: t('networkDetails.channel'),
      value: wifi.channel === null ? '—' : `${wifi.channel}${channel ? ` (${channel})` : ''}`,
      unit: '',
      copy: false,
    });
  }
  return rows.filter(row => row.key !== 'ipv6' || row.value !== '—');
});
</script>

<template>
  <MdResourceCard class="detail-summary network-summary" data-metric="network" :aria-label="t('systemStatus.network')">
    <header class="network-heading">
      <Select
        v-model:open="deviceMenuOpen"
        :model-value="selectedInterface"
        :disabled="!settings.draft || settings.loading || settings.saving"
        @update:model-value="settings.change({ networkInterface: $event === 'automatic' ? null : String($event) })"
      >
        <SelectTrigger
          class="network-selector"
          :aria-label="`${t('systemStatus.interface')} · ${selectedInterfaceLabel}`"
        >
          <span>{{ t('systemStatus.network') }}</span>
          <MdIcon :name="ICON_NAMES.chevronDown" :size="12" aria-hidden="true" />
        </SelectTrigger>
        <SelectContent class="z-60 max-w-[calc(100vw-2rem)]" align="start">
          <SelectItem value="automatic">{{ t('systemStatus.automatic') }}</SelectItem>
          <SelectItem v-for="item in interfaces" :key="item.id" :value="item.id"
            >{{ item.name }}{{ item.connected ? '' : ` · ${t('systemStatus.disconnected')}` }}</SelectItem
          >
          <SelectItem v-if="missingInterface" :value="missingInterface">{{
            t('systemStatus.savedDisconnected')
          }}</SelectItem>
        </SelectContent>
      </Select>
    </header>
    <div class="network-rates">
      <div v-for="rate in rates" :key="rate.direction" class="rate-block" :class="rate.direction">
        <div class="rate-number">
          <strong>{{ rate.value }}</strong
          ><small>{{ rate.unit }}</small>
        </div>
        <span class="rate-label"
          ><b aria-hidden="true">{{ rate.direction === 'download' ? '↓' : '↑' }}</b
          >{{ t(rate.direction === 'download' ? 'systemStatus.download' : 'systemStatus.upload') }}</span
        >
      </div>
    </div>
    <MdResourceTrend
      :key="reading.network.value?.interface.id"
      class="network-history"
      metric="network"
      :active="active"
      :history="reading.networkHistory"
      :observed-at-ms="reading.observedAtMs"
      :label="t('systemStatus.lastMinute')"
    >
      <template #scale="{ ceiling }">{{ `${ByteSizeService.bytes(ceiling)}/s` }}</template>
    </MdResourceTrend>
    <footer class="network-summary-meta">
      <span class="network-identity"
        ><i v-if="network" class="connection-dot" :class="{ inactive: reading.network.status !== 'ready' }" />{{
          network?.interface.name ?? t(METRIC_STATUS_KEYS[reading.network.status])
        }}<span
          v-if="network && reading.network.status === 'ready'"
          class="connection-label"
          role="status"
          :aria-label="t('networkDetails.status')"
          >· {{ t('networkDetails.connected') }}</span
        ></span
      >
      <span :role="reading.network.status !== 'ready' ? 'status' : undefined">{{
        t(reading.network.status === 'ready' ? 'systemStatus.lastMinute' : METRIC_STATUS_KEYS[reading.network.status])
      }}</span>
    </footer>
    <p v-if="settings.error || subscriptionFailed" class="network-selection-error" role="alert">
      {{
        t(
          subscriptionFailed
            ? 'monitoring.unavailable'
            : settings.preferences
              ? 'systemStatus.saveFailed'
              : 'systemStatus.catalogueFailed'
        )
      }}
      <button
        type="button"
        class="network-selection-retry"
        :disabled="settings.loading || settings.saving || connectingPreferences"
        @click="reloadPreferences"
      >
        {{ t('monitoring.refresh') }}
      </button>
    </p>
  </MdResourceCard>
  <section class="network-details network-traffic" :aria-label="t('networkDetails.title')">
    <header>
      <MdTooltip :active="active" :text="trafficHint" multiline>
        <h2 tabindex="0">{{ t('networkDetails.title') }}</h2>
      </MdTooltip>
      <MdTooltip :active="active" :text="t('networkDetails.resetHint')" multiline>
        <button type="button" class="traffic-reset" :disabled="!cumulative || resetting" @click="requestReset">
          {{ t('networkDetails.reset') }}
        </button>
      </MdTooltip>
    </header>
    <dl class="traffic-facts">
      <div v-for="fact in trafficFacts" :key="fact.key">
        <dt>{{ fact.label }}</dt>
        <dd>{{ fact.total === undefined ? '—' : ByteSizeService.bytes(Number(fact.total)) }}</dd>
        <MdTooltip :active="active" :text="t('networkDetails.sessionHint')" multiline>
          <small tabindex="0">{{
            t('networkDetails.sessionValue', {
              value: fact.session === undefined ? '—' : ByteSizeService.bytes(fact.session),
            })
          }}</small>
        </MdTooltip>
      </div>
    </dl>
  </section>
  <section class="network-details network-interface" :aria-label="t('networkDetails.localTitle')">
    <header>
      <h2>{{ t('networkDetails.localTitle') }}</h2>
      <MdTooltip
        :active="active"
        multiline
        :text="
          t(
            network?.interface.kind === 'wifi' && network.receiveLinkBps === null
              ? 'networkDetails.wifiLinkHint'
              : 'networkDetails.linkHint'
          )
        "
        ><span tabindex="0">{{ connection }}</span></MdTooltip
      >
    </header>
    <MdResourceFacts :active="active" :facts="interfaceFacts" layout="rows" />
  </section>
  <MdNetworkAddresses :active="active" />
  <MdConfirmDialog
    v-if="resetTarget"
    presentation="panel"
    :open="true"
    :title="t('networkDetails.resetTitle')"
    :description="t('networkDetails.resetDescription', { name: resetTarget.name })"
    :cancel-label="t('common.cancel')"
    :confirm-label="t('networkDetails.reset')"
    :busy="resetting"
    @update:open="
      open => {
        if (!open) resetTarget = null;
      }
    "
    @confirm="resetTraffic"
  >
    <template v-if="resetError" #default>
      <p role="alert" class="text-xs text-destructive-text">{{ t('networkDetails.resetFailed') }}</p>
    </template>
  </MdConfirmDialog>
</template>

<style scoped>
@reference "@assets/main.css";
.network-details {
  padding: 6px 0;
  display: flex;
  flex-direction: column;
  flex: none;
  gap: 6px;
}
.network-traffic {
  padding-block: 4px;
  gap: 6px;
}
.traffic-facts {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(110px, 100%), 1fr));
  gap: 8px;
}
.traffic-facts > div {
  @apply rounded-lg bg-muted/40;
  padding: 6px;
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: baseline;
  gap: 2px 6px;
}
.traffic-facts small {
  grid-column: 1 / -1;
}
.traffic-facts dt,
.traffic-facts small {
  @apply text-muted-foreground;
  display: block;
  font-size: 10px;
  overflow-wrap: anywhere;
}
.traffic-facts dd {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  overflow-wrap: anywhere;
}
.traffic-reset {
  @apply text-muted-foreground hover:text-foreground;
  font-size: 11px;
  padding: 2px 4px;
  flex: none;
  cursor: pointer;
}
.traffic-reset:disabled {
  opacity: 0.5;
  cursor: default;
}
.network-interface :deep(.rows .resource-fact) {
  padding-block: 2px;
  gap: 4px 10px;
}
.network-interface {
  @apply border-t border-border;
}
header {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
h2 {
  font-size: 12px;
  font-weight: 600;
}
header > span {
  @apply text-muted-foreground;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}
.network-summary {
  --resource-summary-height: 160px;
  --resource-trend-height: 52px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 10px;
}

.network-heading {
  flex-wrap: nowrap;
  gap: 12px;
}
.network-selector {
  @apply text-foreground rounded-md;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-height: 24px;
  min-width: 0;
  max-width: 100%;
  text-align: left;
  overflow-wrap: anywhere;
  font-size: 12px;
  cursor: pointer;
  background: transparent;
}
.network-selector:hover:not(:disabled),
.network-selector[data-state='open'] {
  @apply bg-accent text-accent-foreground;
}
.network-selector:disabled {
  cursor: default;
  opacity: 0.55;
}
.network-selector:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
.network-rates {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  padding-block: 0;
  gap: 16px;
}
.rate-block {
  min-width: 0;
}
.rate-number {
  display: flex;
  align-items: baseline;
  gap: 5px;
  font-variant-numeric: tabular-nums;
}
.rate-number strong {
  font-size: clamp(24px, 7vw, 30px);
  line-height: 1.15;
  font-weight: 600;
  letter-spacing: -0.035em;
}
.rate-number small {
  @apply text-muted-foreground;
  font-size: 12px;
  white-space: nowrap;
}
.rate-label {
  @apply text-muted-foreground;
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  margin-top: 2px;
}
.rate-label b {
  font-size: 14px;
}
.download .rate-label b {
  color: var(--status-download);
}
.upload .rate-label b {
  color: var(--status-upload);
}
.network-history {
  flex: none;
  height: 52px;
  margin-top: 2px;
  border-radius: 6px;
  background: var(--muted);
}
.network-summary-meta {
  @apply text-muted-foreground;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  font-size: 10px;
}
.network-identity {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  overflow-wrap: anywhere;
}
.connection-label {
  white-space: nowrap;
}
.connection-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--success);
  flex: none;
}
.connection-dot.inactive {
  background: var(--muted-foreground);
}
.network-selection-error {
  @apply text-destructive-text;
  font-size: 11px;
}
.network-selection-retry {
  margin-left: 6px;
  text-decoration: underline;
  cursor: pointer;
}
.network-selection-retry:disabled {
  opacity: 0.5;
  cursor: default;
}
</style>
