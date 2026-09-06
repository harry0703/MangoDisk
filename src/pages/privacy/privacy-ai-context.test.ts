import { expect, it } from 'vitest';
import type { PrivacyItem } from '@/lib/models/privacy';
import { privacyAiContext } from './privacy-ai-context';
import fixtures from '../../../tests/fixtures/ai-context-v2.json';

it('projects privacy facts without profiles, tokens, paths or record details', () => {
  const item = {
    ...fixtures[1]!.subject,
    sourceName: 'Browser',
    token: 'private-token',
    sourceId: 'private-source',
    profileName: 'person@example.test',
    profileId: '/Users/person',
    entries: [{ label: 'private-history' }],
  } as unknown as PrivacyItem;
  const context = privacyAiContext(item, 'Cookies', 'allTime', 'windows');
  expect(context).toEqual(fixtures[1]);
  expect(JSON.stringify(context)).not.toMatch(/private|person/);
  expect(privacyAiContext({ ...item, sourceName: '/private/profile' }, 'Cookies', 'today', 'macos').title).toBe(
    '/private/profile · Cookies'
  );
});

it('preserves blocked, review-only and cross-device semantics with zero records', () => {
  const item = {
    ...fixtures[1]!.subject,
    sourceName: 'Browser',
    capability: 'schemaUnsupported',
    recommendation: 'reviewOnly',
    itemCount: 0,
  } as unknown as PrivacyItem;
  expect(privacyAiContext(item, 'Cookies', 'today', 'windows').subject).toMatchObject({
    capability: 'schemaUnsupported',
    recommendation: 'reviewOnly',
    itemCount: 0,
    timeRange: 'today',
    synchronizationMayPropagate: true,
  });
});
