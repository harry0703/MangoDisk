// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { beforeEach, expect, it, vi } from 'vitest';
import type { AppUpdateNotice } from '@/lib/models/app-update';
import Notice from './md-update-notice.vue';
import { BackgroundUpdateService } from '@/lib/services/background-update-service';
import { ResidentService } from '@/lib/services/resident-service';
import en from '@/locales/en-US.json';
import zh from '@/locales/zh-CN.json';
import tw from '@/locales/zh-TW.json';
import ja from '@/locales/ja-JP.json';
import ko from '@/locales/ko-KR.json';
vi.mock('@/lib/services/background-update-service', () => ({ BackgroundUpdateService: { watch: vi.fn() } }));
vi.mock('@/lib/services/resident-service', () => ({ ResidentService: { openMain: vi.fn() } }));
vi.mock('@/lib/services/logger-service', () => ({ LoggerService: { warn: vi.fn(), info: vi.fn() } }));
let publish: (notice: AppUpdateNotice) => void;
const stop = vi.fn();
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(BackgroundUpdateService.watch).mockImplementation(async handler => {
    publish = handler;
    return stop;
  });
  vi.mocked(ResidentService.openMain).mockResolvedValue(undefined);
});
it.each([en, zh, tw, ja, ko])(
  'shows only an available update and opens the existing about destination',
  async messages => {
    const wrapper = mount(Notice, {
      global: {
        plugins: [createI18n({ legacy: false, locale: 'test', messages: { test: messages } })],
        stubs: { MdIcon: true },
      },
    });
    await flushPromises();
    expect(wrapper.find('button').exists()).toBe(false);
    publish({ schemaVersion: 1, checked: true, revision: 1, version: '1.2.0' });
    await flushPromises();
    expect(wrapper.get('button').text()).toBe(messages.updates.noticeAvailable);
    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(ResidentService.openMain).toHaveBeenCalledWith('about');
    publish({ schemaVersion: 1, checked: true, revision: 2, version: null });
    await flushPromises();
    expect(wrapper.find('button').exists()).toBe(false);
    wrapper.unmount();
    expect(stop).toHaveBeenCalledOnce();
  }
);
