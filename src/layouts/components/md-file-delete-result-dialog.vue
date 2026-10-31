<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdFileDeleteCopyButton from '@/components/custom/md-file-delete-copy-button.vue';
import MdFileDeleteFailures from '@/components/custom/md-file-delete-failures.vue';
import { Button } from '@/components/ui/button';
import { Dialog, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import type { PermanentDeleteBatchResult } from '@/lib/models/filesystem/permanent-delete';
import { deleteFailureHintKey } from '@/lib/utils/filesystem/permanent-delete-result';
import { FormatService } from '@/lib/services/presentation/format-service';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';

const props = defineProps<{ result: PermanentDeleteBatchResult | null }>();
const hintKey = computed(() => deleteFailureHintKey(props.result?.failed ?? []));
const open = defineModel<boolean>('open', { required: true });
const { t } = useI18n({ useScope: 'global' });
function focusClose(event: Event) {
  const close = (event.target as HTMLElement).querySelector<HTMLButtonElement>('[data-file-delete-close]');
  if (close) {
    event.preventDefault();
    close.focus();
  }
}
</script>

<template>
  <Dialog v-model:open="open">
    <MdDialogContent class="flex min-h-0 flex-col" size="standard" @open-auto-focus="focusClose">
      <MdDialogHeader>
        <DialogTitle>{{
          t(result?.removedPaths.length ? 'fileDeletion.partialTitle' : 'fileDeletion.failedTitle')
        }}</DialogTitle>
        <DialogDescription>{{ t(hintKey) }}</DialogDescription>
      </MdDialogHeader>
      <div class="min-h-0 overflow-y-auto px-5 pb-5 scrollbar-stable">
        <p v-if="result?.removedPaths.length" class="mb-4 text-sm text-muted-foreground">
          {{
            t('fileDeletion.removedSummary', {
              removed: FormatService.integer(result.removedPaths.length),
              size: ByteSizeService.bytes(result.releasedBytes),
            })
          }}
        </p>
        <MdFileDeleteFailures :failures="result?.failed ?? []" :show-copy-button="false" />
      </div>
      <MdDialogFooter align="between">
        <MdFileDeleteCopyButton :failures="result?.failed ?? []" />
        <Button data-file-delete-close variant="outline" @click="open = false">{{ t('common.close') }}</Button>
      </MdDialogFooter>
    </MdDialogContent>
  </Dialog>
</template>
