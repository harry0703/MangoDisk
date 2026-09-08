// @vitest-environment happy-dom
import { mount } from '@vue/test-utils';
import { afterEach, describe, expect, it } from 'vitest';
import { i18n, type SupportedLocale } from '@/i18n';
import type { AiQuota } from '@/lib/models/ai';
import MdAiQuotaStatus from './md-ai-quota-status.vue';

const initialLocale = i18n.global.locale.value;
afterEach(() => {
  i18n.global.locale.value = initialLocale;
});

describe('AI quota copy', () => {
  it.each<SupportedLocale>(['zh-CN', 'zh-TW', 'en-US', 'ja-JP'])(
    'uses neutral fallback copy and server limits in %s',
    async locale => {
      i18n.global.locale.value = locale;
      const wrapper = mount(MdAiQuotaStatus, { global: { plugins: [i18n] } });
      try {
        expect(wrapper.text()).toBe(i18n.global.t('ai.freeDailyAllowance'));
        // Unknown policy must not become a promise of a fixed allowance or wait.
        for (const key of ['ai.freeDailyAllowance', 'ai.errors.freeRateLimited']) {
          expect(i18n.global.t(key)).not.toMatch(/\d/);
        }
        const quota: AiQuota = {
          available: true,
          unavailableReason: null,
          remaining: 13,
          dailyLimit: 20,
          cooldownSeconds: 60,
          nextAllowedAt: '2026-09-07T00:01:00Z',
          resetAt: '2026-09-08T00:00:00Z',
          serverTime: '2026-09-07T00:00:00Z',
          activeRequests: 0,
          maxConcurrentRequests: 2,
          policyVersion: 'test',
          promptVersion: 'test',
        };
        await wrapper.setProps({ quota });
        expect(wrapper.text()).toBe(i18n.global.t('ai.freeRemaining', { remaining: 13, limit: 20 }));
        await wrapper.setProps({ quota: { ...quota, remaining: 3, dailyLimit: 10 } });
        expect(wrapper.text()).toBe(i18n.global.t('ai.freeRemaining', { remaining: 3, limit: 10 }));
        expect(wrapper.text()).not.toContain('20');
        await wrapper.setProps({ quota: { ...quota, remaining: 0, resetAt: 'invalid' } });
        expect(wrapper.text()).toBe(i18n.global.t('ai.freeDailyExhausted'));
        await wrapper.setProps({ quota: { ...quota, unavailableReason: 'AI_SERVICE_DISABLED' } });
        expect(wrapper.text()).toBe(i18n.global.t('ai.freeTemporarilyUnavailable'));
      } finally {
        wrapper.unmount();
      }
    }
  );
});
