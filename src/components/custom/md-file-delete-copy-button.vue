<script setup lang="ts">
import { ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import type { PermanentDeleteFailure } from '@/lib/models/filesystem/permanent-delete';
import { deleteFailureDiagnostic } from '@/lib/utils/filesystem/permanent-delete-result';
import { ClipboardService } from '@/lib/services/shell/clipboard-service';

const props = defineProps<{ failures: PermanentDeleteFailure[] }>();
const { t } = useI18n({ useScope: 'global' });
const copyState = ref<'idle' | 'copied' | 'failed'>('idle');
watch(
  () => props.failures,
  () => {
    copyState.value = 'idle';
  }
);
async function copyDetails() {
  const failures = props.failures;
  try {
    await ClipboardService.writeText(deleteFailureDiagnostic(failures));
    if (props.failures === failures) copyState.value = 'copied';
  } catch {
    if (props.failures === failures) copyState.value = 'failed';
  }
}
</script>

<template>
  <div class="flex min-w-0 flex-wrap items-center gap-2">
    <Button variant="ghost" size="sm" @click="copyDetails">{{ t('fileDeletion.copyDetails') }}</Button>
    <span role="status" class="text-xs text-muted-foreground">{{
      copyState === 'idle' ? '' : t(`fileDeletion.${copyState}`)
    }}</span>
  </div>
</template>
