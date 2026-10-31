<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import MdFileDeleteCopyButton from '@/components/custom/md-file-delete-copy-button.vue';
import MdMiddleEllipsis from '@/components/custom/md-middle-ellipsis.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import type { PermanentDeleteFailure } from '@/lib/models/filesystem/permanent-delete';
import { groupDeleteFailures } from '@/lib/utils/filesystem/permanent-delete-result';
import { FormatService } from '@/lib/services/presentation/format-service';
import * as PathUtils from '@/lib/utils/filesystem/path';

const props = withDefaults(defineProps<{ failures: PermanentDeleteFailure[]; showCopyButton?: boolean }>(), {
  showCopyButton: true,
});
const { t } = useI18n({ useScope: 'global' });
const groups = computed(() => groupDeleteFailures(props.failures));
const filesOpen = ref(false);
const limit = ref(50);
const visibleFiles = computed(() => {
  let remaining = limit.value;
  return groups.value.flatMap(group => {
    const items = group.items.slice(0, remaining);
    remaining -= items.length;
    return items.map(failure => ({ failure, reason: group.reason }));
  });
});
const partialFailures = computed(() => props.failures.filter(failure => failure.partial));
watch(
  () => props.failures,
  () => {
    filesOpen.value = false;
    limit.value = 50;
  }
);
function toggleFiles(event: Event) {
  const target = event.currentTarget;
  if (target instanceof HTMLDetailsElement) filesOpen.value = target.open;
}
</script>

<template>
  <div class="min-w-0 space-y-4">
    <dl class="space-y-2">
      <div v-for="group in groups" :key="group.reason" class="flex items-baseline justify-between gap-4 text-sm">
        <dt>{{ t(`fileDeletion.reasons.${group.reason}`) }}</dt>
        <dd class="shrink-0 text-muted-foreground">
          {{ t('fileDeletion.itemCount', { count: FormatService.integer(group.items.length) }) }}
        </dd>
      </div>
    </dl>
    <p v-if="partialFailures.length" class="text-sm text-destructive-text">
      {{
        t(
          partialFailures.every(failure => failure.remainingRestored)
            ? 'fileDeletion.partialRestored'
            : 'fileDeletion.partial'
        )
      }}
    </p>
    <details :open="filesOpen" class="min-w-0 text-sm" @toggle="toggleFiles">
      <summary
        class="w-fit cursor-pointer rounded-sm py-1 text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        {{ t('fileDeletion.viewFiles') }}
      </summary>
      <ul v-if="filesOpen" class="mt-2 space-y-2">
        <li v-for="{ failure, reason } in visibleFiles" :key="failure.path" class="min-w-0">
          <div class="flex min-w-0 items-baseline gap-3">
            <MdTooltip :text="PathUtils.display(failure.path)">
              <span
                tabindex="0"
                class="block min-w-0 flex-1 rounded-sm select-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <MdMiddleEllipsis :text="PathUtils.fileName(failure.path)" :show-tooltip="false" :tail-length="16" />
              </span>
            </MdTooltip>
            <span class="shrink-0 text-xs text-muted-foreground">{{ t(`fileDeletion.reasons.${reason}`) }}</span>
          </div>
          <p v-if="failure.partial" class="mt-1 text-xs text-destructive-text">
            {{ t(failure.remainingRestored ? 'fileDeletion.partialRestored' : 'fileDeletion.partial') }}
          </p>
        </li>
      </ul>
      <Button v-if="filesOpen && failures.length > limit" class="mt-2" variant="ghost" size="sm" @click="limit += 50">{{
        t('fileDeletion.showMore')
      }}</Button>
    </details>
    <MdFileDeleteCopyButton v-if="showCopyButton" :failures="failures" />
  </div>
</template>
