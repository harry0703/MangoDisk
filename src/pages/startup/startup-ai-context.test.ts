import { expect, it } from 'vitest';
import type { StartupArtifact } from '@/lib/models/startup';
import { startupAiContext } from './startup-ai-context';
import fixtures from '../../../tests/fixtures/ai-context-v2.json';

const artifact = {
  ...fixtures[2]!.subject.entries![0],
  displayName: 'Fixture service',
  target: { path: '/private/program', arguments: ['private-token'] },
  configurationPath: '/private/config',
  itemId: 'private-id',
  publisher: null,
  trust: 'unknown',
  summary: 'Fixture description',
  iconPath: '/private/icon',
} as unknown as StartupArtifact;

it('preserves full source metadata without execution arguments or opaque IDs', () => {
  const context = startupAiContext('Fixture service', [artifact], 'Startup item', 'windows');
  expect(context).toEqual(fixtures[2]);
  const serialized = JSON.stringify(context);
  expect(serialized).toContain('/private/program');
  expect(serialized).not.toMatch(/private-token|private-id|private\/icon/);
});

it('does not truncate groups or redact names containing paths', () => {
  const context = startupAiContext(
    '/private/group',
    Array.from({ length: 70 }, () => ({ ...artifact, displayName: '/private/item' })),
    'Startup item',
    'macos'
  );
  expect(context.title).toBe('/private/group');
  if (context.subject.module !== 'startup') throw new Error('Expected startup subject');
  expect(context.subject.entries).toHaveLength(70);
  expect(context.subject.omittedCount).toBe(0);
  expect(context.subject.entries[0]!.name).toBe('/private/item');
});

it('retains bank attribution clues and original software descriptions', () => {
  const context = startupAiContext(
    'InfoDaemon.BOC',
    [
      {
        ...artifact,
        configurationPath: '/Library/LaunchDaemons/com.boc.InfoDaemon.plist',
        target: {
          ...artifact.target,
          path: '/usr/local/lib/CFCA/BOCInfo/InfoDaemon.BOC',
          executableName: 'InfoDaemon.BOC',
        },
        publisher: 'Example Publisher',
        summary: 'Software service description',
        version: '1.2.3',
      },
    ],
    'Startup item',
    'macos'
  );
  if (context.subject.module !== 'startup') throw new Error('Expected startup subject');
  expect(context.subject.entries[0]!.identity).toMatchObject({
    executablePath: '/usr/local/lib/CFCA/BOCInfo/InfoDaemon.BOC',
    configurationPath: '/Library/LaunchDaemons/com.boc.InfoDaemon.plist',
    publisher: 'Example Publisher',
    description: 'Software service description',
    version: '1.2.3',
  });
});
