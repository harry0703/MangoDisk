<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { AiQuota } from '@/lib/models/ai';
import { formatAiQuotaResetAt, isAiServiceUnavailable } from '@/lib/utils/ai-quota';

const props = defineProps<{ quota?: AiQuota | null }>();
const { t, locale } = useI18n({ useScope: 'global' });
const resetTime = computed(() =>
  props.quota?.remaining === 0
    ? formatAiQuotaResetAt(props.quota.resetAt, locale.value, Intl.DateTimeFormat().resolvedOptions().timeZone)
    : null
);
</script>

<template>
  <p role="status" aria-live="polite" aria-atomic="true">
    <template v-if="isAiServiceUnavailable(quota)">{{ t('ai.freeTemporarilyUnavailable') }}</template>
    <template v-else-if="quota?.remaining === 0">{{
      resetTime ? t('ai.freeResetsAt', { time: resetTime }) : t('ai.freeDailyExhausted')
    }}</template>
    <template v-else-if="quota">{{
      t('ai.freeRemaining', { remaining: quota.remaining, limit: quota.dailyLimit })
    }}</template>
    <template v-else>{{ t('ai.freeDailyAllowance') }}</template>
  </p>
</template>
