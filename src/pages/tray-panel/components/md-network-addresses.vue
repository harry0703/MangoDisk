<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdCopyButton from '@/components/custom/md-copy-button.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import { useNetworkAddressStore } from '@/stores/network-address-store';
import { ADDRESS_FAMILIES, NETWORK_CONTEXT_INTERVAL_MS } from '@/lib/models/system-resources/network-addresses';
import { networkCountryLabel } from '@/lib/utils/system-resources/network-country';
import { useResidentSettingsStore } from '@/stores/resident-settings-store';
import MdNetworkSettings from './md-network-settings.vue';

const props = defineProps<{ active: boolean }>();
const { t, locale } = useI18n();
const store = useNetworkAddressStore();
const settings = useResidentSettingsStore();
const settingsOpen = ref(false);
const enabled = computed(
  () => settings.preferences?.publicNetwork.enabled === true && settings.draft?.publicNetwork.enabled === true
);
const rows = computed(() =>
  ADDRESS_FAMILIES.map(family => {
    const probe = store.probes[family];
    const address = store.validating ? null : store.addresses[family];
    const country = networkCountryLabel(address?.countryCode ?? null, locale.value);
    const outdated = !!address && (store.results[family].stale || store.contextFailed);
    return {
      family,
      address,
      loading: probe.loading || store.validating,
      outdated,
      hint: [country, outdated ? t('networkDetails.publicLastKnown') : null, t('networkDetails.publicHint')]
        .filter(Boolean)
        .join('\n'),
    };
  }).filter(row => row.family === 'ipv4' || (enabled.value && row.address !== null))
);
async function refresh() {
  if (!props.active || !enabled.value) return;
  await store.refresh();
}
function online() {
  if (props.active && enabled.value) void store.checkContext();
}
watch(
  [() => props.active, enabled],
  ([active, allowed], _previous, onCleanup) => {
    if (!active) settingsOpen.value = false;
    if (!allowed) store.clear();
    if (store.active !== (active && allowed)) void store.setActive(active && allowed);
    if (!active || !allowed) return;
    const timer = setInterval(() => void store.checkContext(), NETWORK_CONTEXT_INTERVAL_MS);
    window.addEventListener('online', online);
    onCleanup(() => {
      clearInterval(timer);
      window.removeEventListener('online', online);
    });
  },
  { immediate: true }
);
onBeforeUnmount(() => void store.setActive(false));
</script>

<template>
  <section class="network-addresses" :aria-label="t('networkDetails.publicTitle')">
    <header>
      <MdTooltip :active="active" :text="t('networkDetails.publicHint')" multiline>
        <h2 tabindex="0">{{ t('networkDetails.publicTitle') }}</h2>
      </MdTooltip>
      <div class="address-actions">
        <MdTooltip v-if="enabled" :active="active" :text="t('networkDetails.publicRefresh')">
          <button
            type="button"
            class="address-action"
            :disabled="store.busy"
            :aria-label="t('networkDetails.publicRefresh')"
            @click="refresh"
          >
            <MdIcon
              :name="ICON_NAMES.refresh"
              :size="14"
              :class="{ 'animate-spin motion-reduce:animate-none': store.busy }"
            />
          </button>
        </MdTooltip>
        <MdTooltip :active="active" :text="t('networkDetails.settings')">
          <button
            type="button"
            class="address-action"
            :aria-label="t('networkDetails.settings')"
            @click="settingsOpen = true"
          >
            <MdIcon :name="ICON_NAMES.settings" :size="14" />
          </button>
        </MdTooltip>
      </div>
    </header>
    <dl v-if="settings.preferences && settings.draft" :aria-busy="enabled && store.busy">
      <div v-for="row in rows" :key="row.family" class="address-row" :data-family="row.family">
        <dt>{{ row.family === 'ipv4' ? 'IPv4' : 'IPv6' }}</dt>
        <dd>
          <span v-if="!enabled" class="address-state" role="status">{{ t('networkDetails.publicDisabled') }}</span>
          <template v-else-if="row.address">
            <span v-if="row.outdated" class="address-state">{{ t('networkDetails.publicStale') }}</span>
            <MdCopyButton
              :key="row.address.ip"
              :active="active"
              :text="row.address.ip"
              :label="`${t('networkDetails.publicTitle')} ${row.family === 'ipv4' ? 'IPv4' : 'IPv6'}`"
              :hint="row.hint"
              show-text
              truncate
            />
          </template>
          <span v-else class="address-state" role="status">{{
            t(row.loading ? 'networkDetails.publicLoading' : 'networkDetails.publicUnavailable')
          }}</span>
        </dd>
      </div>
    </dl>
    <p v-if="enabled && store.contextFailed" class="context-error" role="status">
      {{ t('networkDetails.publicContextFailed') }}
    </p>
    <MdNetworkSettings v-model:open="settingsOpen" />
  </section>
</template>

<style scoped>
@reference "@assets/main.css";
.network-addresses {
  @apply border-t border-border;
  padding: 4px 0 2px;
}
header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 2px;
}
h2 {
  font-size: 12px;
  font-weight: 600;
}
.address-actions {
  display: flex;
  align-items: center;
  gap: 2px;
}
.address-action {
  @apply text-muted-foreground hover:bg-accent hover:text-accent-foreground rounded-md;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  cursor: pointer;
}
.address-action:disabled {
  opacity: 0.55;
  cursor: default;
}
.address-action:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
.address-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 4px 10px;
  padding: 2px 0;
  flex-wrap: nowrap;
}
dt {
  @apply text-muted-foreground;
  font-size: 11px;
}
dd {
  display: flex;
  flex: 1;
  flex-wrap: nowrap;
  align-items: baseline;
  justify-content: flex-end;
  gap: 5px;
  margin-left: auto;
  max-width: 100%;
  min-width: 0;
  font-size: 12px;
  font-weight: 500;
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.address-state,
.context-error {
  @apply text-muted-foreground;
  font-size: 10px;
  font-weight: 400;
}
.address-state {
  flex: none;
  white-space: nowrap;
}
.context-error {
  margin-top: 4px;
}
</style>
