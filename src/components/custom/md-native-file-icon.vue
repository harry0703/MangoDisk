<script setup lang="ts">
import { computed, ref, watch } from 'vue';

import MdFileTypeIcon from '@/components/custom/md-file-type-icon.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import type { FileIconMode } from '@/lib/models/file-icon';
import { ICON_NAMES } from '@/lib/models/ui';
import { FileIconService } from '@/lib/services/file-icon-service';
import * as FileTypeUtils from '@/lib/utils/file-type';

const props = withDefaults(
  defineProps<{
    path: string;
    name: string;
    directory?: boolean;
    directoryMode?: FileIconMode;
    compact?: boolean;
  }>(),
  {
    directory: false,
    directoryMode: 'automatic',
    compact: false,
  }
);

const dataUrl = ref<string | null>(null);
const descriptor = computed(() => FileTypeUtils.descriptor(props.name));
const usesSemanticFileIcon = computed(() => !props.directory && descriptor.value.iconSource === 'semantic');
let requestSequence = 0;

watch(
  () => [props.path, props.name, props.directory, props.directoryMode] as const,
  async ([path, , directory, directoryMode]) => {
    const sequence = ++requestSequence;
    // Some recognized formats resolve to indistinguishable native document
    // icons. The descriptor keeps that presentation policy out of this adapter.
    if (usesSemanticFileIcon.value) {
      dataUrl.value = null;
      return;
    }
    const request = {
      path,
      kind: directory ? ('directory' as const) : ('file' as const),
      mode: directory ? directoryMode : ('automatic' as const),
    };
    const cached = FileIconService.peek(request, true);
    dataUrl.value = cached ?? null;
    const resolved = await FileIconService.resolve(request);
    // Rows can be reused while an asynchronous native batch is running.
    // Ignore stale responses so an old path never paints over the new row.
    // A failed refresh should not replace a known icon with a generic one.
    if (sequence === requestSequence) dataUrl.value = resolved ?? cached ?? null;
  },
  { immediate: true }
);
</script>

<template>
  <span v-if="dataUrl" class="native-file-icon" :class="{ compact }" aria-hidden="true">
    <img :src="dataUrl" alt="" draggable="false" />
  </span>
  <span v-else-if="directory" class="directory-fallback" :class="{ compact }" aria-hidden="true">
    <MdIcon :name="ICON_NAMES.linuxFolder" :size="compact ? 23 : 27" />
  </span>
  <MdFileTypeIcon v-else :name="name" :compact="compact" />
</template>

<style scoped>
@reference "@assets/main.css";

.native-file-icon,
.directory-fallback {
  display: grid;
  width: 34px;
  height: 34px;
  flex: none;
  place-items: center;
}

.native-file-icon.compact,
.directory-fallback.compact {
  width: 30px;
  height: 30px;
}

.native-file-icon img {
  width: 100%;
  height: 100%;
  object-fit: contain;
  user-select: none;
}

.directory-fallback {
  background: transparent;
}
</style>
