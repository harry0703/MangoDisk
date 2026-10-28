// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FloatingLayoutService } from './floating-layout-service';
import type { FloatingEntry } from '@/lib/models/resident/floating-monitor';
function entry(id: FloatingEntry['id'], digits: string): FloatingEntry {
  return { id, digits, marker: '', text: id, tooltip: id, tone: 'normal' };
}
afterEach(() => vi.restoreAllMocks());
describe('floating font measurement', () => {
  it('reserves the same complete reading slot for changing readings and measures localized labels', () => {
    const root = document.createElement('main');
    root.innerHTML = '<span class="metric-label" data-v-fixture>CPU</span>';
    const measuredStyles = new Map<HTMLElement, string>();
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      expect(this.hasAttribute('data-v-fixture')).toBe(true);
      const firstStyle = measuredStyles.get(this) ?? this.className;
      measuredStyles.set(this, firstStyle);
      // Reproduce a WebView that retains a connected clone's first font metrics.
      const glyphWidth = firstStyle === 'metric-label' ? 6 : firstStyle === 'fan-values' ? 5 : 7;
      return new DOMRect(0, 0, (this.textContent?.length ?? 0) * glyphWidth, 14);
    });
    const initial = FloatingLayoutService.measure(root, [{ entry: entry('upload', '0'), label: 'Upload' }]);
    const changed = FloatingLayoutService.measure(root, [
      { entry: { ...entry('upload', '999'), marker: '↑M' }, label: 'Upload' },
    ]);
    expect(changed).toEqual(initial);
    expect(initial[0]!.valueWidth).toBe('999MB/s'.length * 7);
    expect(initial[0]!.labelWidth).toBe('Upload'.length * 6);
    expect(
      FloatingLayoutService.measure(root, [{ entry: entry('upload', '9'), label: '上传' }])[0]!.labelWidth
    ).toBeLessThan(initial[0]!.labelWidth);
    expect(
      FloatingLayoutService.measure(root, [{ entry: entry('fanSpeed', '0'), label: 'GPU fan' }])[0]!.valueWidth
    ).toBe('99.9k'.length * 7);
    expect(
      FloatingLayoutService.measure(root, [
        {
          entry: { ...entry('fanSpeed', '1602'), fanRows: [{ id: 1, text: 'F1 1602', tone: 'normal' }] },
          label: 'GPU fan',
        },
      ])[0]!.valueWidth
    ).toBe('F2 30000'.length * 5);
    for (const id of [
      'cpuFrequency',
      'cpuEfficiencyFrequency',
      'cpuPerformanceFrequency',
      'cpuSuperFrequency',
    ] as const) {
      const widths = ['0.1', '3.2', '20.0', '—'].map(
        digits =>
          FloatingLayoutService.measure(root, [{ entry: entry(id, digits), label: 'CPU frequency' }])[0]!.valueWidth
      );
      expect(new Set(widths).size).toBe(1);
      expect(widths[0]).toBe('20.0GHz'.length * 7);
      const idleWidths = ['active', 'idle', 'unavailable'].map(
        state =>
          FloatingLayoutService.measure(root, [
            {
              entry: {
                ...entry(id, state === 'active' ? '2.4' : 'Idle'),
                frequency: { state: state as 'active' | 'idle' | 'unavailable', idleLabel: 'Long idle label' },
              },
              label: 'CPU',
            },
          ])[0]!.valueWidth
      );
      expect(new Set(idleWidths).size).toBe(1);
      expect(idleWidths[0]).toBe('Long idle label'.length * 7);
    }
    for (const id of ['cpuPower', 'gpuPower', 'systemPower', 'batteryPower', 'chargingPower'] as const) {
      const widths = ['0.0', '0.4', '6.7', '3000.0', '—'].map(
        digits => FloatingLayoutService.measure(root, [{ entry: entry(id, digits), label: 'Power' }])[0]!.valueWidth
      );
      expect(new Set(widths).size).toBe(1);
      expect(widths[0]).toBe('3000.0mW'.length * 7);
    }
    expect(root.children).toHaveLength(1);
  });
  it('removes the hidden measurement node when font measurement fails and supports empty selections', () => {
    const root = document.createElement('main');
    expect(FloatingLayoutService.measure(root, [])).toEqual([]);
    root.innerHTML = '<span class="metric-label">CPU</span>';
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => {
      throw new Error('font unavailable');
    });
    expect(() => FloatingLayoutService.measure(root, [{ entry: entry('cpu', '12'), label: 'CPU' }])).toThrow(
      'font unavailable'
    );
    expect(root.children).toHaveLength(1);
  });
});
