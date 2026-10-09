import type { TrendPoint } from '@/lib/models/system-resources';

export const TREND_WINDOW_MS = 60_000;

/** A monotonic display clock, independent of native sample delivery cadence. */
export class ResourceTrendTimeline {
  origin = 0;
  points: TrendPoint[] = [];
  private clockOrigin = 0;
  private lastObserved = 0;
  private initialized = false;
  private delay = 0;
  private interval = 0;
  private playhead = 0;
  private lastFrameClock = 0;

  time(clock: number): number {
    return this.origin + Math.max(0, clock - this.clockOrigin);
  }

  offset(clock: number): number {
    this.advance(clock);
    return ((this.origin - this.playhead) / TREND_WINDOW_MS) * 100;
  }

  private advance(clock: number) {
    const elapsed = Math.max(0, clock - this.lastFrameClock);
    this.lastFrameClock = clock;
    const now = this.time(clock);
    const last = this.points.at(-1)?.sampledAtMs;
    const fresh = last !== undefined && now - last <= (this.interval >= 3000 ? 10000 : 5000);
    // A delayed VM/IPC delivery must not let the viewport outrun completed
    // samples. Catch up gradually after it arrives, never snap the whole strip.
    // Real outages still scroll out after freshness expires; no values are made up.
    const target = Math.min(now - this.delay, fresh ? last : Infinity);
    this.playhead = Math.max(this.playhead, Math.min(target, this.playhead + elapsed * 1.1));
  }

  position(timestamp: number): number {
    return 100 + ((timestamp - this.origin) / TREND_WINDOW_MS) * 100;
  }

  reset() {
    this.initialized = false;
    this.points = [];
  }

  accept(incoming: TrendPoint[], observed: number, clock: number, interval: number) {
    // Draw a completed segment outside the right edge before revealing it. The
    // short delivery allowance absorbs ordinary IPC jitter without extrapolating
    // values; numeric readings remain live while the graph trails one sample.
    this.delay = interval + 250;
    this.interval = interval;
    if (this.initialized) this.advance(clock);
    // Normal IPC jitter must not restart the scrolling animation. Rebase only on
    // clock discontinuities/resume, or hourly to keep CSS coordinates bounded.
    const discontinuity =
      !this.initialized || observed < this.lastObserved || Math.abs(observed - this.time(clock)) > 5000;
    const rebase = discontinuity || this.time(clock) - this.origin > 3_600_000;
    if (rebase) {
      this.origin = observed;
      this.clockOrigin = clock;
      if (discontinuity) {
        this.points = [];
        const last = incoming.at(-1)?.sampledAtMs;
        const fresh = last !== undefined && observed - last <= (interval >= 3000 ? 10000 : 5000);
        this.playhead = Math.min(observed - this.delay, fresh ? last : Infinity);
        this.lastFrameClock = clock;
      }
    }
    this.initialized = true;
    this.lastObserved = observed;
    if (!incoming.length) {
      // Native baselines are cleared when devices change or sampling restarts.
      this.points = [];
      return;
    }
    const merged = new Map(this.points.map(point => [point.sampledAtMs, point]));
    incoming.forEach(point => merged.set(point.sampledAtMs, point));
    const cutoff = this.playhead - TREND_WINDOW_MS - interval * 2;
    // Connected segments can span two intervals. Retain their offscreen endpoint
    // for the buffered viewport too, so pruning cannot erase a visible fragment.
    const retained = [...merged.values()]
      .filter(point => point.sampledAtMs > cutoff)
      .sort((left, right) => left.sampledAtMs - right.sampledAtMs)
      .slice(-128);
    // Every native snapshot includes all histories, even if only another metric
    // changed. Preserve the array identity to avoid rebuilding unchanged paths.
    if (
      rebase ||
      retained.length !== this.points.length ||
      retained.some((point, index) => {
        const previous = this.points[index]!;
        return (
          point.sampledAtMs !== previous.sampledAtMs ||
          point.primary !== previous.primary ||
          point.secondary !== previous.secondary
        );
      })
    ) {
      this.points = retained;
    }
  }
}
