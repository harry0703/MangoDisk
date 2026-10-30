<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import MdIcon from '@/components/icons/md-icon.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import {
  METRIC_STATUS_KEYS,
  type MetricReading,
  type ResourceReadings,
} from '@/lib/models/system-resources/system-resources';
import type { BatteryDetails } from '@/lib/models/system-resources/power';
import { cachedMetricValue } from '@/lib/utils/system-resources/metric-reading';
import MdResourceTrend from './md-resource-trend.vue';
import MdResourceFacts from './md-resource-facts.vue';
import MdMetricFreshness from './md-metric-freshness.vue';

const props = withDefaults(
  defineProps<{ reading: ResourceReadings; active: boolean; systemPowerSupported?: boolean }>(),
  { systemPowerSupported: true }
);
const { t, locale } = useI18n();
const formatters = computed(() =>
  [0, 1, 2].map(maximumFractionDigits => new Intl.NumberFormat(locale.value, { maximumFractionDigits }))
);
function number(value: number | null | undefined, digits = 1): string {
  return value == null || !Number.isFinite(value) ? '—' : formatters.value[digits]!.format(value);
}
function observed<T>(reading: MetricReading<T>): T | null {
  return cachedMetricValue(reading, props.reading.observedAtMs);
}
const battery = computed(() =>
  props.reading.batteryPresent === false ? null : observed(props.reading.batteryDetails)
);
const system = computed(() => observed(props.reading.systemPower));
const batteryPower = computed(() =>
  props.reading.batteryPresent === false ? null : observed(props.reading.batteryPower)
);
const temperature = computed(() =>
  props.reading.batteryPresent === false ? null : observed(props.reading.batteryTemperature)
);
const STATE_KEYS: Record<BatteryDetails['chargeState'], string> = {
  charging: 'power.states.charging',
  finishing: 'power.states.finishing',
  discharging: 'power.states.discharging',
  full: 'power.states.full',
  paused: 'power.states.paused',
  connected: 'power.states.connected',
  unknown: 'power.states.unknown',
};
const CAPACITY_KEYS = { fullCharge: 'power.fullCharge', design: 'power.design' };
const batteryState = computed(() => battery.value?.chargeState ?? 'unknown');
const batteryPending = computed(() => !battery.value && props.reading.batteryDetails.status === 'loading');
const batteryLabel = computed(() =>
  battery.value
    ? t(STATE_KEYS[batteryState.value])
    : t(
        METRIC_STATUS_KEYS[
          props.reading.batteryDetails.status === 'ready' ? 'stale' : props.reading.batteryDetails.status
        ]
      )
);
const batteryIcon = computed(() => {
  const value = battery.value;
  if (!value) return ICON_NAMES.battery;
  if (['charging', 'finishing'].includes(value.chargeState) && value.externalConnected !== false)
    return ICON_NAMES.batteryCharging;
  if (value.externalConnected && ['full', 'paused', 'connected'].includes(value.chargeState)) return ICON_NAMES.plug;
  if (value.chargeState !== 'discharging' && value.chargeState !== 'full') return ICON_NAMES.battery;
  const percent = value.chargePercent;
  if (percent === null || !Number.isFinite(percent) || percent <= 0 || percent > 100) return ICON_NAMES.battery;
  if (percent <= 25) return ICON_NAMES.batteryLow;
  return percent <= 75 ? ICON_NAMES.batteryMedium : ICON_NAMES.batteryFull;
});
const batterySourceIcon = computed(() => {
  if (!battery.value) return null;
  if (batteryIcon.value === ICON_NAMES.batteryCharging) return ICON_NAMES.zap;
  if (batteryIcon.value === ICON_NAMES.plug) return ICON_NAMES.plug;
  return batteryState.value === 'unknown' ? ICON_NAMES.help : null;
});
const time = computed(() => {
  const state = batteryState.value;
  const charging = state === 'charging' || state === 'finishing';
  if (!charging && state !== 'discharging') return '';
  const seconds = charging ? battery.value?.secondsToFull : battery.value?.secondsToEmpty;
  if (seconds == null) return '';
  const minutes = Math.max(1, Math.ceil(seconds / 60));
  const duration =
    minutes < 60
      ? t('power.minutes', { minutes })
      : t('power.hoursMinutes', { hours: Math.floor(minutes / 60), minutes: minutes % 60 });
  return t(charging ? 'power.untilFull' : 'power.remaining', { duration });
});
const capacityHealth = computed(() => {
  const percent = battery.value?.healthPercent;
  return percent != null && Number.isFinite(percent) && percent >= 0 ? Math.round(percent * 10) / 10 : null;
});
const capacityHealthTone = computed<'success' | 'warning' | 'critical' | undefined>(() => {
  const percent = capacityHealth.value;
  if (percent == null) return undefined;
  if (percent < 80) return 'critical';
  return percent < 90 ? 'warning' : 'success';
});
const batteryFacts = computed(() => [
  {
    key: 'battery-power',
    label: t(
      batteryPower.value?.state === 'charging'
        ? 'power.intoBattery'
        : batteryPower.value?.state === 'discharging'
          ? 'power.fromBattery'
          : 'power.batteryPower'
    ),
    value: number(batteryPower.value?.watts),
    unit: 'W',
    hint: t('power.batteryPowerHint'),
  },
  {
    key: 'temperature',
    label: t('power.temperature'),
    value: number(temperature.value?.celsius, 0),
    unit: '°C',
  },
  {
    key: 'health',
    label: t('power.health'),
    value: number(capacityHealth.value),
    tone: capacityHealthTone.value,
    unit: '%',
    hint: t('power.healthHint'),
  },
  { key: 'cycles', label: t('power.cycles'), value: number(battery.value?.cycleCount, 0), unit: '' },
  { key: 'voltage', label: t('power.batteryVoltage'), value: number(battery.value?.voltageVolts, 2), unit: 'V' },
  {
    key: 'current-ma',
    label: t('power.batteryCurrent'),
    value: number(battery.value?.currentMilliamps, 0),
    unit: 'mA',
  },
]);
const capacityFacts = computed(() => {
  const capacity = battery.value?.capacity;
  return (['fullCharge', 'design'] as const).map(key => ({
    key,
    label: t(CAPACITY_KEYS[key]),
    value: number(capacity?.[key], 0),
    unit: capacity ? (capacity.unit === 'milliwattHours' ? 'mWh' : 'mAh') : '',
  }));
});
const currentCapacityHint = computed(() => {
  const capacity = battery.value?.capacity;
  if (capacity?.current == null || !Number.isFinite(capacity.current) || capacity.current < 0) return '';
  const unit = capacity.unit === 'milliwattHours' ? 'mWh' : 'mAh';
  return `${t('power.current')}: ${number(capacity.current, 0)} ${unit}`;
});
const capacityRatio = computed(() => {
  const { fullCharge, design } = battery.value?.capacity ?? {};
  if (
    fullCharge == null ||
    design == null ||
    !Number.isFinite(fullCharge) ||
    !Number.isFinite(design) ||
    fullCharge < 0 ||
    design <= 0
  )
    return null;
  return Math.min(100, (fullCharge / design) * 100);
});
const batteryLevel = computed(() => {
  const percent = battery.value?.chargePercent;
  return percent != null && Number.isFinite(percent) && percent >= 0 && percent <= 100 ? percent : null;
});
const batteryLevelTone = computed(() => {
  const percent = batteryLevel.value;
  if (percent == null) return 'unknown';
  if (percent < 20) return 'critical';
  return percent <= 40 ? 'low' : 'healthy';
});
const batteryStatusTone = computed(() => {
  if (
    batterySourceIcon.value === ICON_NAMES.zap ||
    (batterySourceIcon.value === ICON_NAMES.plug && batteryState.value === 'full')
  )
    return 'success';
  if (batteryState.value === 'discharging' && batteryLevel.value != null && batteryLevel.value <= 15) return 'critical';
  return 'normal';
});
const adapterFacts = computed(() => [
  { key: 'input', label: t('power.inputPower'), value: number(battery.value?.adapter?.inputWatts), unit: 'W' },
  {
    key: 'reported',
    label: t('power.reportedPower'),
    value: number(battery.value?.adapter?.reportedWatts, 0),
    unit: 'W',
    hint: t('power.adapterHint'),
  },
  {
    key: 'voltage',
    label: t('power.adapterVoltage'),
    value: number(battery.value?.adapter?.voltageVolts, 2),
    unit: 'V',
  },
]);
</script>

<template>
  <div class="power-details">
    <header
      v-if="reading.batteryPresent !== false"
      class="battery-status"
      :data-level-tone="batteryLevelTone"
      :data-status-tone="batteryStatusTone"
      :aria-busy="batteryPending"
    >
      <div class="battery-level">
        <div
          class="battery-shell"
          :class="{ unknown: batteryLevel == null && !batteryPending, loading: batteryPending }"
          aria-hidden="true"
        >
          <div class="battery-fill" :style="{ width: `${batteryLevel ?? 0}%` }" />
          <span v-if="batterySourceIcon" class="battery-source" :data-source="batterySourceIcon">
            <MdIcon
              v-if="batterySourceIcon !== ICON_NAMES.help"
              class="battery-source-outline"
              :name="batterySourceIcon"
              :size="24"
              :stroke-width="5"
            />
            <MdIcon class="battery-source-glyph" :name="batterySourceIcon" :size="24" :stroke-width="2" />
          </span>
        </div>
        <MdTooltip :text="currentCapacityHint" :active="active" :close-on-text-change="false">
          <strong :tabindex="currentCapacityHint ? 0 : undefined"
            >{{ number(batteryLevel, 0) }}<small v-if="batteryLevel != null">%</small></strong
          >
        </MdTooltip>
      </div>
      <div class="battery-state">
        <span class="battery-state-label"><MdIcon :name="batteryIcon" :size="12" />{{ batteryLabel }}</span>
        <MdMetricFreshness
          v-if="battery"
          :reading="reading.batteryDetails"
          :now="reading.observedAtMs"
          :active="active"
        />
        <small v-if="time && reading.batteryDetails.status === 'ready'">{{ time }}</small>
      </div>
    </header>
    <section v-if="systemPowerSupported && reading.systemPower.status !== 'unsupported'" class="system-power">
      <div class="power-heading">
        <div class="power-label">
          <MdTooltip :text="t('power.systemPowerHint')" :active="active" multiline
            ><span tabindex="0">{{ t('power.systemPower') }}</span></MdTooltip
          ><small>{{ t('power.lastMinute') }}</small>
        </div>
        <strong>{{ number(system?.watts) }}<small v-if="system"> W</small></strong>
      </div>
      <MdResourceTrend
        v-if="reading.systemPowerHistory.length"
        metric="power"
        :history="reading.systemPowerHistory"
        :observed-at-ms="reading.observedAtMs"
        :label="t('power.trendLabel')"
        :active="active"
      />
      <small v-if="!system" class="power-unavailable">{{
        t(METRIC_STATUS_KEYS[reading.systemPower.status === 'ready' ? 'stale' : reading.systemPower.status])
      }}</small>
    </section>
    <template v-if="reading.batteryPresent !== false">
      <section class="power-section battery-facts">
        <h2>{{ t('power.details') }}</h2>
        <MdResourceFacts :facts="batteryFacts" :active="active" layout="rows" />
        <div class="battery-capacity" :data-health-tone="capacityHealthTone">
          <dl class="capacity-scale">
            <div v-for="fact in capacityFacts" :key="fact.key" class="resource-fact">
              <dt>{{ fact.label }}</dt>
              <dd>
                {{ fact.value }}<small v-if="fact.unit">{{ fact.unit }}</small>
              </dd>
            </div>
          </dl>
          <div
            v-if="capacityRatio != null"
            class="capacity-track"
            role="meter"
            :aria-label="t('power.health')"
            aria-valuemin="0"
            aria-valuemax="100"
            :aria-valuenow="capacityRatio"
            :aria-valuetext="`${number(capacityRatio)}%`"
          >
            <span :style="{ width: `${capacityRatio}%` }" />
          </div>
        </div>
      </section>
      <section v-if="battery && battery.externalConnected !== null" class="power-section adapter-facts">
        <div class="adapter-heading">
          <h2>{{ t('power.adapter') }}</h2>
          <span class="adapter-connection" :data-connected="battery.externalConnected">{{
            t(battery.externalConnected ? 'power.adapterConnected' : 'power.adapterDisconnected')
          }}</span>
        </div>
        <MdResourceFacts
          v-if="battery.externalConnected && battery.adapter"
          :facts="adapterFacts"
          :active="active"
          layout="rows"
        />
        <p v-else-if="battery.externalConnected" class="adapter-unavailable">{{ t('power.adapterUnavailable') }}</p>
      </section>
    </template>
    <p v-else-if="!system" class="power-unavailable">{{ t('power.noMeasurements') }}</p>
  </div>
</template>

<style scoped>
@reference "@assets/main.css";
.power-details {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 0;
}
.battery-status {
  --battery-color: var(--muted-foreground);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 6px 0 4px;
  flex: none;
}
.battery-status[data-level-tone='healthy'] {
  --battery-color: var(--success);
}
.battery-status[data-level-tone='low'] {
  --battery-color: var(--warning);
}
.battery-status[data-level-tone='critical'] {
  --battery-color: var(--destructive-text);
}
.battery-level {
  display: flex;
  align-items: center;
  gap: 14px;
}
.battery-shell {
  @apply bg-muted;
  position: relative;
  width: 68px;
  height: 32px;
  padding: 3px;
  border: 2px solid var(--muted-foreground);
  border-radius: 9px;
  background-clip: content-box;
}
.battery-shell::after {
  @apply bg-muted-foreground;
  content: '';
  position: absolute;
  width: 3px;
  height: 12px;
  top: 8px;
  right: -6px;
  border-radius: 0 3px 3px 0;
}
.battery-fill {
  height: 100%;
  border-radius: 4px;
  background: var(--battery-color);
}
.battery-source {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
}
.battery-source[data-source='plug'] {
  transform: rotate(90deg);
}
.battery-source :deep(svg) {
  grid-area: 1 / 1;
  fill: currentColor;
}
/* A contrasting keyline keeps power symbols readable across partial battery fills. */
.battery-source-outline {
  color: var(--foreground);
}
.battery-source-glyph {
  color: var(--background);
}
.battery-source[data-source='help'] :deep(svg) {
  color: var(--foreground);
  fill: var(--background);
}
.battery-shell.unknown {
  border-style: dashed;
}
.battery-shell.loading {
  @apply bg-muted;
  opacity: 0.55;
}
.battery-level strong {
  font-size: 32px;
  line-height: 1;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.battery-level small {
  @apply text-muted-foreground;
  font-size: 16px;
  font-weight: 400;
  margin-left: 2px;
}
.battery-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  /* Reserve the estimate/update line so a refresh does not move the facts. */
  min-height: 44px;
  text-align: center;
  font-size: 11px;
  max-width: 100%;
  overflow-wrap: anywhere;
}
.battery-state-label {
  @apply bg-muted/40 text-muted-foreground;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border-radius: 6px;
  padding: 3px 9px;
  font-weight: 500;
}
[data-status-tone='success'] .battery-state-label {
  @apply bg-success/12 text-success-foreground;
}
[data-status-tone='critical'] .battery-state-label {
  @apply bg-destructive/10 text-destructive-text;
}
.battery-state small,
.power-label small,
.power-unavailable {
  @apply text-muted-foreground;
  font-size: 10px;
}
.system-power {
  padding: 0 0 2px;
  flex: none;
  min-width: 0;
}
.power-heading {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
  font-size: 12px;
}
.power-label {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 3px 8px;
}
.power-heading strong {
  flex: none;
  font-size: 18px;
  font-weight: 500;
  line-height: 1.2;
  font-variant-numeric: tabular-nums;
}
.power-heading strong small {
  font-size: 11px;
  font-weight: normal;
}
.system-power :deep(.resource-trend) {
  height: 22px;
  margin-top: 4px;
}
.power-section {
  flex: none;
}
.power-section h2 {
  @apply text-foreground;
  font-size: 12px;
  font-weight: 600;
  margin-bottom: 6px;
}
.power-details .power-section :deep(.resource-facts.rows .resource-fact) {
  padding: 2px 0;
  gap: 2px 12px;
}
.power-details .power-section :deep(.resource-facts.rows dt) {
  font-size: 12px;
  line-height: 18px;
  flex: 1;
  min-width: 0;
}
.power-section :deep(dt > span:first-child) {
  overflow: visible;
  text-overflow: clip;
  white-space: normal;
  overflow-wrap: anywhere;
}
.power-details .power-section :deep(.resource-facts.rows dd) {
  font-size: 12px;
  line-height: 18px;
  font-weight: 500;
}
.power-section :deep(dd small) {
  margin-left: 3px;
  @apply text-muted-foreground;
  font-size: 10px;
}
.battery-capacity {
  --capacity-health-color: var(--muted-foreground);
  @apply border-t border-border;
  margin-top: 6px;
  padding-top: 6px;
}
.battery-capacity[data-health-tone='success'] {
  --capacity-health-color: var(--success);
}
.battery-capacity[data-health-tone='warning'] {
  --capacity-health-color: var(--warning);
}
.battery-capacity[data-health-tone='critical'] {
  --capacity-health-color: var(--destructive-text);
}
.capacity-scale {
  display: flex;
  justify-content: space-between;
  gap: 12px;
}
.capacity-scale .resource-fact {
  min-width: 0;
  flex: 1;
}
.capacity-scale .resource-fact:last-child {
  text-align: right;
}
.capacity-scale dt {
  @apply text-muted-foreground;
  font-size: 10px;
  overflow-wrap: anywhere;
}
.capacity-scale dd {
  margin: 1px 0 0;
  font-variant-numeric: tabular-nums;
  overflow-wrap: anywhere;
}
.capacity-track {
  @apply bg-muted;
  height: 6px;
  border-radius: 3px;
  overflow: hidden;
  margin: 5px 0;
}
.capacity-track span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--capacity-health-color);
}
.adapter-facts {
  @apply border-t border-border;
  padding-top: 8px;
}
.adapter-heading {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}
.adapter-connection,
.adapter-unavailable {
  @apply text-muted-foreground;
  font-size: 11px;
}
.adapter-connection[data-connected='true'] {
  @apply text-success-foreground;
}
.power-unavailable {
  display: block;
  margin-top: 4px;
}
</style>
