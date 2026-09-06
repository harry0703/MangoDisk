import { expect, it } from 'vitest';
import type { SystemSettingItem } from '@/lib/models/system-settings';
import { systemOptimizationAiContext } from './system-optimization-ai-context';
import fixtures from '../../../tests/fixtures/ai-context-v2.json';

it('distinguishes unapplied targets from actual state and original-value recovery', () => {
  const fixture = fixtures[3]!;
  const item = {
    ...fixture.subject,
    restoreAvailable: fixture.subject.hasRecordedOriginalValue,
    settingId: 'private-id',
  } as unknown as SystemSettingItem;
  expect(systemOptimizationAiContext(item, fixture.title, fixture.description, 'windows', 'optimized')).toEqual(
    fixture
  );
  expect(
    systemOptimizationAiContext(
      { ...item, status: 'optimized', restoreAvailable: true },
      fixture.title,
      fixture.description,
      'windows',
      null
    ).subject
  ).toMatchObject({ status: 'optimized', pendingTarget: null, hasRecordedOriginalValue: true });
});
