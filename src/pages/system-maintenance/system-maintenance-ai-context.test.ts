import { expect, it } from 'vitest';
import type { SystemMaintenanceItem } from '@/lib/models/system-maintenance';
import { systemMaintenanceAiContext } from './system-maintenance-ai-context';
import fixtures from '../../../tests/fixtures/ai-context-v2.json';

it('preserves availability, permission and estimate without diagnosing a fault', () => {
  const fixture = fixtures[4]!;
  const item = { ...fixture.subject } as unknown as SystemMaintenanceItem;
  expect(systemMaintenanceAiContext(item, fixture.title, fixture.description, 'macos')).toEqual(fixture);
  expect(
    systemMaintenanceAiContext(
      { ...item, status: 'unavailable', diagnostic: 'toolUnavailable' },
      fixture.title,
      fixture.description,
      'windows'
    ).subject
  ).toMatchObject({ status: 'unavailable', diagnostic: 'toolUnavailable' });
});
