import type { FloatingEntry, FloatingItemMeasurement } from '@/lib/models/resident/floating-monitor';

/** Measure localized labels and bounded samples in the WebView's actual fonts. */
export class FloatingLayoutService {
  static measure(root: HTMLElement, entries: { entry: FloatingEntry; label: string }[]): FloatingItemMeasurement[] {
    if (!entries.length) return [];
    const reference = root.querySelector<HTMLElement>('.metric-label');
    if (!reference) throw new Error('Floating metric font reference unavailable');
    const meters = new Map<string, HTMLElement>();
    const cache = new Map<string, number>();
    const measure = (className: string, samples: string[]): number =>
      Math.max(
        ...samples.map(text => {
          const key = `${className}:${text}`;
          let width = cache.get(key);
          if (width === undefined) {
            let meter = meters.get(className);
            if (!meter) {
              // WebView2 can retain the previous font when a connected clone changes class.
              // Assign each measurement style before insertion and never switch it in place.
              meter = reference.cloneNode(false) as HTMLElement;
              meter.className = className;
              meter.style.cssText =
                'position:fixed;visibility:hidden;pointer-events:none;display:inline-block;width:max-content;max-width:none;white-space:pre;overflow:visible';
              meters.set(className, meter);
              root.append(meter);
            }
            meter.textContent = text;
            width = Math.ceil(meter.getBoundingClientRect().width);
            if (!Number.isFinite(width) || width <= 0) throw new Error('Floating font measurement is empty');
            cache.set(key, width);
          }
          return width;
        })
      );
    try {
      return entries.map(({ entry, label }) => {
        let digits = ['100', '—'];
        let units = ['%'];
        if (entry.id.endsWith('Frequency')) {
          digits = ['20.0', '18.8', '—'];
          units = ['GHz'];
        } else if (entry.id.endsWith('Temperature')) {
          digits = ['-100', '200', '—'];
          units = ['°C'];
        } else if (entry.id.endsWith('Power')) {
          digits = ['3000.0', '888.8', '0.0', '—'];
          units = ['W', 'mW'];
        } else if (entry.id === 'upload' || entry.id === 'download') {
          digits = ['999', '—'];
          units = ['B/s', 'KB/s', 'MB/s', 'GB/s', 'TB/s'];
        } else if (entry.id === 'fanSpeed') {
          digits = ['9999', '8888', '99.9k', '—'];
          units = [''];
        }
        const samples = digits.flatMap(digits => units.map(unit => (digits === '—' ? digits : digits + unit)));
        if (entry.frequency) samples.push(entry.frequency.idleLabel);
        const valueWidth = Math.max(
          measure('metric-value', samples),
          entry.fanRows?.length ? measure('fan-values', ['F2 30000']) : 0
        );
        return {
          id: entry.id,
          gpuDeviceId: entry.gpuDevice?.id ?? null,
          labelWidth: measure('metric-label', [label]),
          valueWidth,
        };
      });
    } finally {
      meters.forEach(meter => meter.remove());
    }
  }
}
