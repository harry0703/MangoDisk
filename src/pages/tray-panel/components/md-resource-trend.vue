<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue';
import MdIconTrend from '@/components/icons/md-icon-trend.vue';
import type { TrendPoint } from '@/lib/models/system-resources';
import { ResourceTrendTimeline } from './resource-trend-timeline';
import { ResourceTrendScale } from './resource-trend-scale';

const props = withDefaults(
  defineProps<{
    metric: 'cpu' | 'gpu' | 'memory' | 'network' | 'disk' | 'temperature';
    history: TrendPoint[];
    observedAtMs: number;
    label: string;
    active?: boolean;
  }>(),
  { active: true }
);
const timeline = new ResourceTrendTimeline();
const offset = ref(0);
const points = shallowRef<TrendPoint[]>([]);
const ceiling = ref(1);
const scale = new ResourceTrendScale();
const amplitude = ref(1);
const bidirectional = computed(() => props.metric === 'network' || props.metric === 'disk');
const interval = computed(() =>
  props.metric === 'temperature' ? 4000 : props.metric === 'memory' ? 3000 : props.metric === 'network' ? 1000 : 2000
);
const series = computed(() => {
  const channels = !bidirectional.value
    ? [{ secondary: false, color: 'var(--primary)' }]
    : [
        { secondary: false, color: 'var(--status-download)' },
        { secondary: true, color: 'var(--status-upload)' },
      ];
  return channels.map(channel => {
    const segments: { x: number; y: number }[][] = [];
    let previous: number | null = null;
    for (const point of points.value) {
      const value = channel.secondary ? point.secondary : point.primary;
      if (value === null || !Number.isFinite(value)) {
        previous = null;
        continue;
      }
      // Leave missing samples blank instead of drawing a misleading bridge over
      // disconnected periods. The retained edge sample scrolls out with its line.
      if (!segments.length || previous === null || point.sampledAtMs - previous > interval.value * 2) segments.push([]);
      segments.at(-1)!.push({
        x: timeline.position(point.sampledAtMs),
        y: bidirectional.value
          ? 50 + (channel.secondary ? -1 : 1) * Math.max(0, Math.min(1, value / ceiling.value)) * 47
          : 98 - Math.max(0, Math.min(1, value / ceiling.value)) * 96,
      });
      previous = point.sampledAtMs;
    }
    const paths = segments.map(segment => {
      const first = segment[0]!;
      const last = segment.at(-1)!;
      const line = `M${segment.map(point => `${point.x},${point.y}`).join(' L')}`;
      // A round-capped zero-length segment makes the first real sample visible.
      return {
        line: segment.length === 1 ? `${line} l0.001,0` : line,
        area: `${line} L${last.x},${bidirectional.value ? 50 : 100} L${first.x},${bidirectional.value ? 50 : 100} Z`,
      };
    });
    return {
      color: channel.color,
      line: paths.map(path => path.line).join(' '),
      area: paths.map(path => path.area).join(' '),
    };
  });
});
let frame = 0;
let frameTimer: ReturnType<typeof setTimeout> | null = null;
let lastPaint = -Infinity;
// A minute-wide trend moves fractions of a pixel per frame. Bound SVG updates
// independently of the display's refresh rate without changing sample cadence.
const FRAME_INTERVAL_MS = 1000 / 30;
let mounted = false;
let reducedMotion: MediaQueryList | null = null;

function paint(clock = performance.now()) {
  // Move vector geometry instead of translating a cached bitmap. Fractional CSS
  // layer offsets can blur thin strokes in WebView2 at Windows display scaling.
  offset.value = timeline.offset(clock);
  amplitude.value = bidirectional.value ? scale.amplitude(clock) : 1;
}
function animate() {
  frame = 0;
  if (props.active === false || document.hidden) {
    stop();
    return;
  }
  const clock = performance.now();
  if (clock - lastPaint >= FRAME_INTERVAL_MS) {
    paint(clock);
    lastPaint = clock;
  }
  const last = points.value.at(-1);
  if (!last || timeline.position(last.sampledAtMs) + timeline.offset(performance.now()) < 0) {
    // A disconnected, fully empty viewport needs no continuous frame work.
    frame = 0;
    return;
  }
  // Wait outside the rendering loop instead of waking at 120/144 Hz just
  // to skip a paint. RAF still aligns each SVG update with the next display frame.
  frameTimer = setTimeout(
    () => {
      frameTimer = null;
      frame = requestAnimationFrame(animate);
    },
    Math.max(1, Math.ceil(FRAME_INTERVAL_MS - (performance.now() - lastPaint)))
  );
}
function stop() {
  cancelAnimationFrame(frame);
  if (frameTimer !== null) clearTimeout(frameTimer);
  frameTimer = null;
  frame = 0;
  lastPaint = -Infinity;
}
function start() {
  if (
    mounted &&
    props.active !== false &&
    !document.hidden &&
    !reducedMotion?.matches &&
    !frame &&
    frameTimer === null &&
    points.value.length
  ) {
    frame = requestAnimationFrame(animate);
  }
}
function sync() {
  const clock = performance.now();
  timeline.accept(props.history, props.observedAtMs || props.history.at(-1)?.sampledAtMs || 0, clock, interval.value);
  points.value = timeline.points;
  if (!bidirectional.value) ceiling.value = props.metric === 'temperature' ? 150 : 100;
  else {
    const peak = Math.max(
      1,
      ...points.value.flatMap(point => [point.primary, point.secondary ?? 0]).filter(Number.isFinite)
    );
    scale.update(peak, clock, !mounted || document.hidden || reducedMotion?.matches);
    ceiling.value = scale.ceiling;
    if (!points.value.length) scale.reset();
  }
  // The new geometry and its compensating scale must update in the same Vue
  // render. Subsequent frames adjust one SVG group, retaining non-scaling strokes.
  amplitude.value = bidirectional.value ? scale.amplitude(clock) : 1;
  if (!points.value.length) stop();
  // Positions and a rebased strip transform must reach the same paint together.
  void nextTick(() => {
    if (!mounted) return;
    paint();
    start();
  });
}
function resume() {
  stop();
  if (props.active === false || document.hidden) return;
  timeline.reset();
  scale.reset();
  sync();
}
watch(() => props.active, resume);
watch(() => [props.history, props.observedAtMs, props.metric], sync, { immediate: true });
onMounted(() => {
  mounted = true;
  reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
  if (reducedMotion.matches) scale.finish();
  reducedMotion.addEventListener('change', resume);
  document.addEventListener('visibilitychange', resume);
  paint();
  start();
});
onBeforeUnmount(() => {
  mounted = false;
  stop();
  reducedMotion?.removeEventListener('change', resume);
  document.removeEventListener('visibilitychange', resume);
});
</script>

<template>
  <div class="resource-trend" :class="[metric, { bidirectional }]" role="img" :aria-label="label">
    <div class="trend-strip">
      <MdIconTrend :series="series" :amplitude="amplitude" :offset="offset" />
    </div>
  </div>
</template>

<style scoped>
@reference "@assets/main.css";
.resource-trend {
  @apply border-b border-border;
  height: 24px;
  position: relative;
  overflow: hidden;
  margin-top: 4px;
}
.bidirectional {
  border-bottom: 0;
}
.temperature {
  height: 36px;
}
.bidirectional::before {
  content: '';
  position: absolute;
  top: 50%;
  left: 0;
  right: 0;
  border-top: 1px solid var(--border);
}
.trend-strip {
  position: absolute;
  inset: 0;
}
</style>
