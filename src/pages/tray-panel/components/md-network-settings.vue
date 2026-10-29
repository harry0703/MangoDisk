<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdSettingsGroup from '@/components/custom/md-settings-group.vue';
import MdSettingsRow from '@/components/custom/md-settings-row.vue';
import MdSwitch from '@/components/custom/md-switch.vue';
import { Button } from '@/components/ui/button';
import { Dialog, DialogTitle } from '@/components/ui/dialog';
import type { PublicNetworkPreferences } from '@/lib/models/system-resources/network-addresses';
import { useResidentSettingsStore } from '@/stores/resident-settings-store';

const open = defineModel<boolean>('open', { required: true });
const { t } = useI18n();
const settings = useResidentSettingsStore();
const preferences = computed(() => settings.draft?.publicNetwork);
const disabled = computed(() => !preferences.value || settings.loading || settings.saving);
function change(patch: Partial<PublicNetworkPreferences>) {
  if (disabled.value || !preferences.value) return;
  void settings.change({ publicNetwork: { ...preferences.value, ...patch } });
}
</script>

<template>
  <Dialog v-model:open="open">
    <MdDialogContent size="compact" :show-close="false" :aria-describedby="undefined" class="network-settings max-w-80">
      <MdDialogHeader>
        <DialogTitle class="text-sm">{{ t('networkDetails.settings') }}</DialogTitle>
      </MdDialogHeader>
      <div class="network-settings-body" :aria-busy="settings.saving">
        <MdSettingsGroup plain>
          <MdSettingsRow
            :title="t('networkDetails.publicEnabled')"
            :description="t('networkDetails.queryHint')"
            description-id="network-public-hint"
            label-for="network-public-enabled"
            compact
          >
            <MdSwitch
              id="network-public-enabled"
              aria-describedby="network-public-hint"
              :model-value="preferences?.enabled ?? false"
              :disabled="disabled"
              @update:model-value="change({ enabled: $event })"
            />
          </MdSettingsRow>
        </MdSettingsGroup>
        <p v-if="settings.error" class="text-xs text-destructive-text" role="alert">
          {{ t('networkDetails.settingsFailed') }}
          <button
            type="button"
            class="underline"
            :disabled="settings.loading || settings.saving"
            @click="settings.load()"
          >
            {{ t('common.retry') }}
          </button>
        </p>
      </div>
      <MdDialogFooter class="flex-row justify-end">
        <Button size="sm" variant="outline" @click="open = false">{{ t('common.close') }}</Button>
      </MdDialogFooter>
    </MdDialogContent>
  </Dialog>
</template>

<style scoped>
.network-settings-body {
  padding: 0 14px 12px;
}
</style>
<style>
.network-settings {
  --layout-dialog-header-height: 0px;
  --layout-dialog-header-padding: 14px 14px 8px;
  --layout-dialog-footer-height: 44px;
  --layout-dialog-footer-padding: 4px 14px 12px;
}
</style>
