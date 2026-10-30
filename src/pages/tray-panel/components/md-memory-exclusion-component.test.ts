// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createI18n } from 'vue-i18n';
import { expect, it, vi } from 'vitest';
import Row from './md-application-resource-row.vue';
import { useMemoryReleaseStore } from '@/stores/memory-release-store';
import { MemoryReleaseService } from '@/lib/services/system-resources/memory-release-service';
import en from '@/locales/en-US.json';
import zhCN from '@/locales/zh-CN.json';
import zhTW from '@/locales/zh-TW.json';
import jaJP from '@/locales/ja-JP.json';
import koKR from '@/locales/ko-KR.json';
vi.mock('@/lib/services/settings/operating-system-service', () => ({
  OperatingSystemService: { isWindows: () => true, isLinux: () => false },
}));
vi.mock('@/lib/services/system-resources/memory-release-service', () => ({ MemoryReleaseService: { save: vi.fn() } }));
vi.mock('@/lib/services/settings/byte-size-service', () => ({ ByteSizeService: { memory: (n: number) => `${n} B` } }));
it('excludes all matching image processes from the ranking without quitting or removing the row', async () => {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useMemoryReleaseStore();
  store.accept({
    schemaVersion: 1,
    revision: 0,
    automatic: false,
    intervalMinutes: 30,
    thresholdPercent: 80,
    skipForeground: true,
    exclusions: [],
  });
  vi.mocked(MemoryReleaseService.save).mockImplementation(async p => ({ ...p, revision: p.revision + 1 }));
  const page = mount(Row, {
    props: {
      application: {
        id: 'code',
        name: 'Code.exe',
        usedBytes: 100,
        readableProcessCount: 12,
        processCount: 12,
        iconPath: 'C:\\Apps\\Code.exe',
        isBundle: false,
        canQuit: true,
        canForceQuit: true,
      },
      expanded: true,
      share: 100,
    },
    global: {
      plugins: [pinia, createI18n({ legacy: false, locale: 'en', messages: { en } })],
      stubs: { MdNativeFileIcon: true, MdIcon: true },
    },
  });
  const action = page.get('button[aria-pressed]');
  await action.trigger('click');
  await flushPromises();
  expect(action.attributes('aria-pressed')).toBe('true');
  expect(page.get('.excluded-badge').text()).toBe('Excluded');
  expect(page.get('.application-name').text()).toBe('Code.exe (12)');
  await action.trigger('click');
  await flushPromises();
  expect(page.find('.excluded-badge').exists()).toBe(false);
  page.unmount();
});

it('expands a CPU application into individual PID readings without app-wide actions', async () => {
  const page = mount(Row, {
    props: {
      application: {
        id: 'code',
        name: 'Code.exe',
        pid: 10,
        usedPercent: 3,
        processCount: 2,
        iconPath: 'C:\\Apps\\Code.exe',
        isBundle: false,
        canQuit: false,
        canForceQuit: false,
        processes: [
          { pid: 10, startedAt: 1, usedPercent: 2 },
          { pid: 11, startedAt: 1, usedPercent: 1 },
        ],
      },
      metric: 'cpu',
      expanded: true,
      share: 100,
    },
    global: {
      plugins: [createPinia(), createI18n({ legacy: false, locale: 'en', messages: { en } })],
      stubs: { MdNativeFileIcon: true, MdIcon: true },
    },
  });
  expect(page.get('.application-name').text()).toContain('Code.exe (2)');
  expect(page.findAll('.cpu-members li').map(row => row.text())).toEqual(['Process ID: 102.0%', 'Process ID: 111.0%']);
  expect(page.find('.quit-application-button').exists()).toBe(false);
  expect(page.find('button[aria-pressed]').exists()).toBe(false);
  await page.setProps({
    application: {
      id: 'system',
      name: 'System',
      pid: 4,
      usedPercent: 1,
      processCount: 1,
      iconPath: null,
      isBundle: false,
      canQuit: false,
      canForceQuit: false,
      locationStatus: 'denied',
    },
  });
  expect(page.text()).toContain(en.monitoring.locationDenied);
  expect(page.find('.reveal-button').exists()).toBe(false);
  page.unmount();
});

it.each([en, zhCN, zhTW, jaJP, koKR])(
  'labels partial memory sums without hiding readable bytes or marking complete groups partial',
  async messages => {
    const page = mount(Row, {
      props: {
        application: {
          id: 'cloud',
          name: '0dcloud',
          usedBytes: 568,
          readableProcessCount: 1,
          processCount: 2,
          iconPath: null,
          isBundle: true,
          canQuit: false,
          canForceQuit: false,
        },
        metric: 'memory',
        expanded: true,
        share: 10,
      },
      global: {
        plugins: [createPinia(), createI18n({ legacy: false, locale: 'en', messages: { en: messages } })],
        stubs: { MdNativeFileIcon: true, MdIcon: true },
      },
    });
    expect(page.get('.application-row strong').text()).toBe('568 B');
    expect(page.get('.excluded-badge').text()).toBe(messages.monitoring.partialData);
    expect(page.get('.application-details').text()).toContain(
      messages.monitoring.partialMemory.replace('{readable}', '1').replace('{total}', '2')
    );
    await page.setProps({ application: { ...page.props('application'), readableProcessCount: 2 } });
    expect(page.find('.excluded-badge').exists()).toBe(false);
    await page.setProps({
      application: { ...page.props('application'), readableProcessCount: 0, usedBytes: null },
      available: false,
    });
    expect(page.get('.application-row strong').text()).toBe('—');
    expect(page.get('.application-details').text()).toContain(messages.monitoring.memoryUnavailable);
    page.unmount();
  }
);
