// @vitest-environment happy-dom

import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';

import { i18n } from '@/i18n';
import type { PrivacyItem } from '@/lib/models/privacy';

import MdPrivacyResultList from './md-privacy-result-list.vue';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'macos' }));

const checkboxStub = {
  props: ['checked', 'indeterminate', 'disabled', 'ariaLabel'],
  emits: ['update:checked'],
  template: `
    <button
      class="result-checkbox-stub"
      :disabled="disabled"
      :aria-label="ariaLabel"
      :data-checked="String(checked)"
      :data-indeterminate="String(indeterminate)"
      @click="$emit('update:checked', !checked)"
    />
  `,
};
const resultTableStub = {
  methods: { scrollTo() {} },
  template: '<div class="result-table-stub"><slot /></div>',
};
const passthroughStub = { template: '<div><slot /></div>' };
const iconStub = {
  props: ['name'],
  template: '<span class="icon-stub" :data-icon="name" />',
};
const applicationIconStub = {
  props: ['src'],
  emits: ['error'],
  template: '<img class="application-icon-stub" :src="src" @error="$emit(\'error\')" />',
};

function item(
  token: string,
  category: PrivacyItem['category'],
  kind: PrivacyItem['kind'],
  itemCount: number,
  overrides: Partial<PrivacyItem> = {}
): PrivacyItem {
  return {
    token,
    sourceId: 'chrome',
    sourceName: 'Fixture Browser',
    profileId: 'chrome:Default',
    profileName: 'Default',
    category,
    kind,
    sensitivity: 'activity',
    impact: category === 'browserAccountState' ? 'signOut' : 'low',
    recommendation: 'recommended',
    capability: 'ready',
    itemCount,
    estimatedBytes: itemCount * 16,
    selectedByDefault: true,
    requiresBrowserClose: category !== 'systemActivity',
    synchronizationMayPropagate: false,
    ...overrides,
  };
}

function mountBrowser(sourceIconUrls: Readonly<Record<string, string>> = {}) {
  return mount(MdPrivacyResultList, {
    props: {
      busy: false,
      items: [
        item('history', 'browserActivity', 'browsingHistory', 5, {
          capability: 'browserRunning',
          synchronizationMayPropagate: true,
        }),
        item('profile-empty-download', 'browserActivity', 'downloadHistory', 0, {
          profileName: 'Work',
          profileId: 'chrome:Work',
          capability: 'empty',
          selectedByDefault: false,
        }),
        item('empty-history', 'browserActivity', 'browsingHistory', 0, {
          sourceId: 'empty-browser',
          sourceName: 'Empty Browser',
          capability: 'empty',
          selectedByDefault: false,
        }),
        item('cookies', 'browserAccountState', 'cookies', 3),
        item('sessions', 'browserActivity', 'sessions', 1, {
          impact: 'low',
          recommendation: 'manual',
          selectedByDefault: false,
        }),
        item('clipboard', 'systemActivity', 'currentClipboard', 1),
      ],
      selectedTokens: ['history'],
      sourceIconUrls,
    },
    global: {
      plugins: [i18n],
      stubs: {
        MdIcon: iconStub,
        MdApplicationIcon: applicationIconStub,
        MdIconAction: passthroughStub,
        MdResultCheckbox: checkboxStub,
        MdResultTable: resultTableStub,
        MdResultTableRow: passthroughStub,
      },
    },
  });
}

describe('privacy result list component', () => {
  it('explains a row without selecting it or opening record details', async () => {
    const wrapper = mountBrowser();
    const row = wrapper
      .findAll('.privacy-item')
      .find(row => row.text().includes(i18n.global.t('privacy.kinds.browsingHistory')));
    await row?.get('.md-ai-action').trigger('click');
    expect(wrapper.emitted('explain')?.[0]?.[0]).toMatchObject({ token: 'history' });
    expect(wrapper.emitted('show-details')).toBeUndefined();
    expect(wrapper.emitted('update:selectedTokens')).toBeUndefined();
    expect(wrapper.find('button button').exists()).toBe(false);
    wrapper.unmount();
  });

  it('keeps browser activity, account state, and system traces in separate navigation categories', async () => {
    const wrapper = mountBrowser();

    expect(wrapper.findAll('.result-master-detail-navigation .result-category-item')).toHaveLength(3);
    expect(wrapper.findAll('.result-master-detail-navigation .result-category-item')[2]?.text()).toContain(
      '1 item · 1 trace'
    );
    expect(wrapper.findAll('.source-section')).toHaveLength(2);
    expect(wrapper.findAll('.profile-header')).toHaveLength(2);
    expect(wrapper.findAll('.privacy-item')).toHaveLength(3);
    expect(wrapper.text()).toContain('Default');
    expect(wrapper.text()).toContain('Work');
    expect(wrapper.get('.privacy-item').text()).toContain('Browsing history');
    expect(wrapper.get('.privacy-item').text()).not.toContain('Default');
    expect(wrapper.text()).not.toContain('Cookies and sign-in state');
    expect(wrapper.text()).toContain('Browser sessions');

    await wrapper.findAll('.result-master-detail-navigation .result-category-item')[1]?.trigger('click');

    expect(wrapper.findAll('.privacy-item')).toHaveLength(1);
    expect(wrapper.text()).toContain('Cookies and sign-in state');

    await wrapper.findAll('.result-master-detail-navigation .result-category-item')[2]?.trigger('click');

    expect(wrapper.findAll('.privacy-item')).toHaveLength(1);
    expect(wrapper.text()).toContain('Current clipboard');
    wrapper.unmount();
  });

  it('uses a user profile icon for each browser profile row', () => {
    const wrapper = mountBrowser();

    expect(wrapper.get('.profile-header .icon-stub').attributes('data-icon')).toBe('circleUserRound');
    wrapper.unmount();
  });

  it('uses a resolved native application icon for a browser group', () => {
    const wrapper = mountBrowser({ chrome: 'data:image/png;base64,fixture' });

    expect(wrapper.get('.application-icon-stub').attributes('src')).toBe('data:image/png;base64,fixture');
    wrapper.unmount();
  });

  it('falls back to the built-in browser icon when a native application icon fails', async () => {
    const wrapper = mountBrowser({ chrome: 'data:image/png;base64,broken' });

    await wrapper.get('.application-icon-stub').trigger('error');

    expect(wrapper.find('.application-icon-stub').exists()).toBe(false);
    expect(wrapper.find('.icon-stub').exists()).toBe(true);
    wrapper.unmount();
  });

  it('keeps an empty scanned item visible inside its browser group', async () => {
    const wrapper = mountBrowser();
    const emptyGroup = wrapper.findAll('.source-section')[1];

    expect(emptyGroup?.text()).toContain('Empty Browser');
    expect(emptyGroup?.text()).toContain('0 traces');
    expect(emptyGroup?.findAll('.privacy-item')).toHaveLength(0);

    await emptyGroup?.get('.source-disclosure').trigger('click');

    expect(emptyGroup?.findAll('.privacy-item')).toHaveLength(1);
    expect(emptyGroup?.text()).toContain('Browsing history');
    expect(emptyGroup?.text()).toContain('No traces');
    expect(emptyGroup?.get('.privacy-item .result-checkbox-stub').attributes('disabled')).toBeDefined();
    wrapper.unmount();
  });

  it('changes only the actionable tokens owned by the selected browser', async () => {
    const wrapper = mountBrowser();

    await wrapper.get('.source-section .source-header > .result-checkbox-stub').trigger('click');

    expect(wrapper.emitted('update:selectedTokens')?.at(-1)).toEqual([['history', 'sessions']]);
    wrapper.unmount();
  });

  it('selects every actionable item in the active category without changing other categories', async () => {
    const wrapper = mountBrowser();
    await wrapper.get('.result-detail-selection .result-checkbox-stub').trigger('click');

    expect(wrapper.emitted('update:selectedTokens')?.at(-1)).toEqual([['history', 'sessions']]);
    wrapper.unmount();
  });

  it('keeps a browser collapsed after selecting the entire category', async () => {
    const wrapper = mountBrowser();

    await wrapper.get('.source-disclosure').trigger('click');
    expect(wrapper.findAll('.profile-header')).toHaveLength(0);

    await wrapper.get('.result-detail-selection .result-checkbox-stub').trigger('click');
    const selectedTokens = wrapper.emitted('update:selectedTokens')?.at(-1)?.[0] as string[];
    await wrapper.setProps({ selectedTokens });

    expect(wrapper.findAll('.profile-header')).toHaveLength(0);
    expect(wrapper.get('.source-disclosure').attributes('aria-expanded')).toBe('false');
    wrapper.unmount();
  });

  it('shows low risk once without repeating generic synchronization guidance', () => {
    const wrapper = mountBrowser();
    const historyRow = wrapper.findAll('.privacy-item').find(row => row.text().includes('Browsing history'));
    const sessionRow = wrapper.findAll('.privacy-item').find(row => row.text().includes('Browser sessions'));

    expect(historyRow?.get('.result-item-badge').text()).toBe('Low risk');
    expect(sessionRow?.get('.result-item-badge').text()).toBe('Low risk');
    expect(historyRow?.text()).not.toContain('Low impact');
    expect(historyRow?.text()).not.toContain('browser sync');
    expect(sessionRow?.find('.result-item-description').exists()).toBe(false);
    expect(wrapper.text()).not.toContain('Cookies and sign-in state');
    expect(wrapper.text()).not.toContain('Close browser to clean');
    wrapper.unmount();
  });

  it('labels a protected macOS application and its trace with the required permission', () => {
    const wrapper = mount(MdPrivacyResultList, {
      props: {
        busy: false,
        items: [
          item('pages-recent', 'applicationActivity', 'recentDocuments', 0, {
            sourceId: 'pages',
            sourceName: 'Pages',
            profileId: null,
            profileName: null,
            capability: 'permissionRequired',
            selectedByDefault: false,
          }),
        ],
        selectedTokens: [],
        permissionLabel: 'Full Disk Access required',
      },
      global: {
        plugins: [i18n],
        stubs: {
          MdIcon: iconStub,
          MdApplicationIcon: applicationIconStub,
          MdIconAction: passthroughStub,
          MdResultCheckbox: checkboxStub,
          MdResultTable: resultTableStub,
          MdResultTableRow: passthroughStub,
        },
      },
    });

    expect(wrapper.get('.source-header .result-item-badge').text()).toBe('Full Disk Access required');
    expect(wrapper.get('.privacy-item .result-item-badge').text()).toBe('Full Disk Access required');
    expect(wrapper.text()).not.toContain('Not supported');
    wrapper.unmount();
  });

  it('opens read-only details without changing the cleanup selection', async () => {
    const wrapper = mountBrowser();
    const historyRow = wrapper.findAll('.privacy-item').find(row => row.text().includes('Browsing history'));

    await historyRow?.get('.privacy-item-disclosure').trigger('click');

    expect(wrapper.emitted('show-details')?.at(-1)?.[0]).toMatchObject({ token: 'history' });
    expect(wrapper.emitted('update:selectedTokens')).toBeUndefined();
    wrapper.unmount();
  });
});
