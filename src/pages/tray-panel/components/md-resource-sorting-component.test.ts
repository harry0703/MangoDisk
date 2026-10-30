// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createI18n } from 'vue-i18n';
import { expect, it, vi } from 'vitest';
import List from './md-application-resource-list.vue';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import { useTrayPanelStore } from '@/stores/tray-panel-store';
import type { ApplicationCpu, ProcessCpuSummary } from '@/lib/models/system-resources/system-resources';
import en from '@/locales/en-US.json';
vi.mock('@/lib/services/settings/preference-storage-service', () => ({
  PreferenceStorageService: { loadResourceSort: vi.fn(async () => null), saveResourceSort: vi.fn(async () => {}) },
}));
vi.mock('@/lib/services/settings/operating-system-service', () => ({
  OperatingSystemService: { isWindows: () => true, isLinux: () => false },
}));
const app = (id: string, usedPercent: number): ApplicationCpu => ({
  id,
  name: id,
  usedPercent,
  pid: 10,
  processCount: 1,
  iconPath: null,
  isBundle: false,
  canQuit: false,
  canForceQuit: false,
});
const summary = (applications: ApplicationCpu[]): ProcessCpuSummary => ({
  applications,
  usageScale: 'totalCapacity',
  readableProcessCount: applications.length,
  omittedProcessCount: 0,
});
function mountList(applications: ApplicationCpu[], metric: 'cpu' | 'memory' = 'cpu') {
  const pinia = createPinia();
  const page = mount(List, {
    props: {
      metric,
      summary:
        metric === 'cpu'
          ? summary(applications)
          : {
              usageKind: 'physicalFootprint',
              applications: applications.map(({ usedPercent, ...identity }) => ({
                ...identity,
                usedBytes: usedPercent * 1024,
                readableProcessCount: 1,
              })),
              readableProcessCount: applications.length,
              omittedProcessCount: 0,
            },
    },
    global: {
      plugins: [pinia, createI18n({ legacy: false, locale: 'en', messages: { en } })],
      stubs: { MdIcon: true, MdNativeFileIcon: true },
    },
  });
  return { page, store: useTrayPanelStore(pinia) };
}
it('holds rows through rank changes and missing samples, then resumes after collapse', async () => {
  const { page } = mountList([app('a', 20), app('b', 10)]);
  await flushPromises();
  await page.findAll('.application-row')[1]!.trigger('click');
  const row = page.get('[data-application-id="b"]').element;
  await page.setProps({ summary: summary([app('b', 90), app('a', 1)]) });
  expect(page.findAll('.application-name').map(r => r.text())).toEqual(['a', 'b']);
  expect(page.get('[data-application-id="b"]').element).toBe(row);
  expect(page.get('[data-application-id="b"] strong').text()).toBe('90.0%');
  await page.setProps({ summary: summary([app('a', 1)]) });
  expect(page.get('[data-application-id="b"] strong').text()).toBe('—');
  expect(page.text()).toContain(en.monitoring.applicationUnavailable);
  await page.get('[data-application-id="b"] .application-row').trigger('click');
  await page.setProps({ summary: summary([app('c', 99), app('a', 1)]) });
  expect(page.findAll('.application-name').map(r => r.text())).toEqual(['c', 'a']);
  page.unmount();
});
it('sorts explicitly while expanded, persists independently and bounds rendered rows', async () => {
  const { page, store } = mountList(Array.from({ length: 500 }, (_, n) => app(`App ${n}`, n)));
  await flushPromises();
  expect(page.findAll('.application-row').length).toBeLessThan(30);
  await page.findAll('.application-row')[0]!.trigger('click');
  await page.findAll('.sort-columns button')[0]!.trigger('click');
  expect(store.sortPreferences.cpu).toEqual({ column: 'name', direction: 'ascending' });
  expect(store.sortPreferences.memory.column).toBe('usage');
  expect(page.find('[aria-expanded="true"]').exists()).toBe(true);
  expect(PreferenceStorageService.saveResourceSort).toHaveBeenCalled();
  await page.setProps({ active: false });
  expect(page.find('[aria-expanded="true"]').exists()).toBe(false);
  page.unmount();
});

it('does not let a late preference read reorder open details or overwrite a user choice', async () => {
  let resolve!: (value: unknown) => void;
  vi.mocked(PreferenceStorageService.loadResourceSort).mockImplementationOnce(
    () =>
      new Promise(r => {
        resolve = r;
      })
  );
  const { page, store } = mountList([app('z', 20), app('a', 1)]);
  await page.findAll('.application-row')[0]!.trigger('click');
  resolve({ schemaVersion: 1, cpu: { column: 'name', direction: 'ascending' } });
  await flushPromises();
  expect(page.findAll('.application-name').map(r => r.text())).toEqual(['z', 'a']);
  await page.findAll('.sort-columns button')[0]!.trigger('click');
  expect(store.sortPreferences.cpu.direction).toBe('descending');
  expect(page.get('[aria-expanded="true"]').text()).toContain('z');
  page.unmount();
});

it.each(['cpu', 'memory'] as const)(
  'restores %s rows after hiding at the bottom without a scroll event',
  async metric => {
    // Native hidden WebViews can change scrollTop without delivering a scroll event.
    vi.spyOn(HTMLElement.prototype, 'scrollTo').mockImplementation(function (
      this: HTMLElement,
      options?: ScrollToOptions | number
    ) {
      if (typeof options === 'object') this.scrollTop = options.top ?? 0;
    });
    const { page } = mountList(
      Array.from({ length: 50 }, (_, n) => app(`App ${n}`, n)),
      metric
    );
    await flushPromises();
    const viewport = page.get('.list-viewport');
    Object.defineProperty(viewport.element, 'scrollHeight', { configurable: true, value: 2000 });
    Object.defineProperty(viewport.element, 'clientHeight', { configurable: true, value: 400 });
    (viewport.element as HTMLElement).scrollTop = 1600;
    await viewport.trigger('scroll');
    expect(page.findAll('.application-name').some(row => row.text() === 'App 49')).toBe(false);
    await page.setProps({ active: false });
    await page.setProps({ active: true });
    expect((viewport.element as HTMLElement).scrollTop).toBe(0);
    expect(page.findAll('.application-name').some(row => row.text() === 'App 49')).toBe(true);
    page.unmount();
    vi.restoreAllMocks();
  }
);
