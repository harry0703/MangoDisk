import { describe, expect, it } from 'vitest';
import { createI18n } from 'vue-i18n';
import zhCN from '@/locales/zh-CN.json';
import zhTW from '@/locales/zh-TW.json';
import enUS from '@/locales/en-US.json';
import jaJP from '@/locales/ja-JP.json';
import type { SystemMaintenanceExecutionItemResult } from '@/lib/models/system-maintenance';
import { maintenanceFailureFeedback } from './system-maintenance-feedback';

const result: SystemMaintenanceExecutionItemResult = {
  taskId: 'windows.maintenance.update-components',
  status: 'failed',
  mutationState: 'mayHaveChanged',
  verified: false,
  requiresRestart: false,
  failureReason: 'serviceDisabled',
};

describe.each([
  ['zh-CN', zhCN],
  ['zh-TW', zhTW],
  ['en-US', enUS],
  ['ja-JP', jaJP],
] as const)('maintenance failure feedback (%s)', (locale, messages) => {
  const { t } = createI18n({ legacy: false, locale, messages: { [locale]: messages } }).global;
  it('shows only the specific reason after partial changes', () => {
    for (const reason of [
      'serviceDisabled',
      'serviceUnavailable',
      'dependencyUnavailable',
      'serviceBusy',
      'timedOut',
      'permissionDenied',
    ] as const) {
      const feedback = maintenanceFailureFeedback({ ...result, failureReason: reason }, 'Test', t);
      expect(feedback).toEqual({ message: t(`systemMaintenance.feedback.failures.${reason}`, { name: 'Test' }) });
      expect(feedback.message).not.toContain('systemMaintenance.');
    }
  });
  it('distinguishes verification failure from repair failure', () => {
    for (const reason of ['verificationFailed', 'verificationPermissionDenied'] as const) {
      const feedback = maintenanceFailureFeedback({ ...result, failureReason: reason }, 'Test', t);
      expect(feedback.message).toBe(t(`systemMaintenance.feedback.failures.${reason}`, { name: 'Test' }));
    }
  });
  it('does not invent a reason when only a generic failure is known', () => {
    const feedback = maintenanceFailureFeedback({ ...result, failureReason: null }, 'Test', t);
    expect(feedback.message).toBe(t('systemMaintenance.feedback.mayHaveChanged', { name: 'Test' }));
  });
});
