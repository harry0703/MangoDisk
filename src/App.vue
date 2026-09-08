<script setup lang="ts">
import { computed } from 'vue';

import { Toaster } from '@/components/ui/sonner';
import { TooltipProvider } from '@/components/ui/tooltip';
import MdAppShell from '@/layouts/md-app-shell.vue';
import { TOOLTIP_OPEN_DELAY_MS } from '@/lib/models/ui';
import type { AppSettings } from '@/lib/models/settings';
import { useAppStore } from '@/stores/app-store';

const appStore = useAppStore();
const toastTheme = computed<AppSettings['theme']>(() => appStore.settings.theme);
</script>

<template>
  <TooltipProvider
    :delay-duration="TOOLTIP_OPEN_DELAY_MS"
    :disable-hoverable-content="true"
    :ignore-non-keyboard-focus="true"
  >
    <MdAppShell />
  </TooltipProvider>
  <!-- Keep notification placement stable across pages, dialogs and AI workspaces. -->
  <Toaster :theme="toastTheme" position="bottom-right" :gap="10" :visible-toasts="4" expand rich-colors close-button />
</template>
