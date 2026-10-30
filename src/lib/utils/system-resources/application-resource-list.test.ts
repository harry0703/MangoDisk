import { describe, expect, it } from 'vitest';
import {
  defaultResourceSort,
  parseResourceSort,
  sortResourceRows,
  updateResourceRows,
} from '@/lib/utils/system-resources/application-resource-list';
import type { ApplicationCpu } from '@/lib/models/system-resources/system-resources';
function app(id: string, usage: number, name = id): ApplicationCpu {
  return {
    id,
    name,
    usedPercent: usage,
    pid: 1,
    processCount: 1,
    isBundle: false,
    canQuit: false,
    canForceQuit: false,
    iconPath: null,
  };
}
describe('resource list interaction order', () => {
  it('updates measurements while retaining identities, missing rows and member positions', () => {
    const a = {
      ...app('a', 20),
      processes: [
        { pid: 1, startedAt: 1, usedPercent: 12 },
        { pid: 2, startedAt: 1, usedPercent: 8 },
      ],
    };
    const before = updateResourceRows([], [a, app('b', 10)], false);
    const next = [
      {
        ...a,
        usedPercent: 3,
        processes: [
          { pid: 2, startedAt: 1, usedPercent: 3 },
          { pid: 1, startedAt: 2, usedPercent: 0 },
        ],
      },
      app('c', 99),
    ];
    const held = updateResourceRows(before, next, true);
    expect(held.map(r => r.application.id)).toEqual(['a', 'b']);
    expect(held[0]?.application).toEqual(next[0]);
    expect(held[0]?.members.map(p => [p.pid, p.startedAt, p.usedPercent])).toEqual([
      [1, 1, null],
      [2, 1, 3],
    ]);
    expect(held[1]?.available).toBe(false);
    expect(updateResourceRows(held, next, false).map(r => r.application.id)).toEqual(['a', 'c']);
  });
  it('sorts names naturally and resolves equal readings by stable identity', () => {
    const rows = updateResourceRows([], [app('b', 1, 'App 10'), app('a', 1, 'App 2'), app('c', 1, 'App 2')], false);
    expect(sortResourceRows(rows, { column: 'name', direction: 'ascending' }, 'en').map(r => r.application.id)).toEqual(
      ['a', 'c', 'b']
    );
    expect(
      sortResourceRows(rows.reverse(), { column: 'usage', direction: 'descending' }, 'en').map(r => r.application.id)
    ).toEqual(['a', 'c', 'b']);
  });
  it('validates persisted versions and each independently saved metric', () => {
    expect(parseResourceSort({ schemaVersion: 9 })).toEqual(defaultResourceSort());
    expect(
      parseResourceSort({
        schemaVersion: 1,
        cpu: { column: 'name', direction: 'ascending' },
        memory: { column: 'oops' },
      })
    ).toEqual({
      ...defaultResourceSort(),
      cpu: { column: 'name', direction: 'ascending' },
    });
  });
});

it('retains unchanged sample values while applying status changes and real clears', async () => {
  const { retainSampleValue } = await import('@/lib/utils/system-resources/system-resources');
  const previous = { sampledAtMs: 1, status: 'ready' as const, value: { rows: [1] } };
  const incoming = { sampledAtMs: 1, status: 'stale' as const, value: { rows: [1] } };
  const result = retainSampleValue(previous, incoming);
  expect(result.value).toBe(previous.value);
  expect(result.status).toBe('stale');
  expect(retainSampleValue(previous, { ...incoming, sampledAtMs: 2 }).value).toBe(incoming.value);
  expect(retainSampleValue(previous, { ...incoming, value: null }).value).toBeNull();
});

it('keeps unreadable memory last in both usage directions without treating it as zero', () => {
  const memory = (id: string, usedBytes: number | null) => ({
    id,
    name: id,
    usedBytes,
    readableProcessCount: usedBytes === null ? 0 : 1,
    processCount: 1,
    isBundle: false,
    canQuit: false,
    canForceQuit: false,
    iconPath: null,
  });
  const rows = updateResourceRows([], [memory('unknown', null), memory('zero', 0), memory('busy', 100)], false);
  expect(rows[0]?.available).toBe(false);
  for (const direction of ['ascending', 'descending'] as const) {
    const sorted = sortResourceRows(rows, { column: 'usage', direction }, 'en');
    expect(sorted.at(-1)?.application.id).toBe('unknown');
    expect(sorted[0]?.application.id).toBe(direction === 'ascending' ? 'zero' : 'busy');
  }
});
