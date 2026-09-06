// @vitest-environment happy-dom

import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';

import MdResultItemContent from './md-result-item-content.vue';
import MdResultTableRow from './md-result-table-row.vue';

const iconStub = { template: '<span class="icon-stub" />' };

describe('result item content component', () => {
  it('keeps inline actions before the value and separate from the disclosure button', async () => {
    const wrapper = mount(MdResultItemContent, {
      props: { title: 'Cache', value: '741 MB', disclosureLabel: 'Cache', expandable: true },
      slots: { actions: '<button class="ai-fixture">AI</button>' },
      global: { stubs: { MdIcon: iconStub } },
    });
    expect(wrapper.find('button button').exists()).toBe(false);
    expect(
      wrapper.get('.result-item-actions').element.nextElementSibling?.classList.contains('result-item-value')
    ).toBe(true);
    await wrapper.get('.ai-fixture').trigger('click');
    expect(wrapper.emitted('toggle')).toBeUndefined();
    await wrapper.get('.result-item-disclosure').trigger('click');
    expect(wrapper.emitted('toggle')).toHaveLength(1);
    await wrapper.setProps({ disclosureDisabled: true });
    expect(wrapper.get('.result-item-disclosure').attributes('disabled')).toBeDefined();
  });
  it('opts result rows into the shared cleanup item geometry', () => {
    const wrapper = mount(MdResultTableRow, {
      props: { layout: 'item' },
      slots: { default: '<span>Result</span>' },
    });

    expect(wrapper.get('.result-table-row').classes()).toContain('result-table-row-item');
  });

  it('uses the shared title, badge, description, value, and disclosure layout', () => {
    const wrapper = mount(MdResultItemContent, {
      props: {
        title: 'Browsing history',
        description: 'Default · Low impact',
        badge: 'Recommended',
        badgeTone: 'accent',
        value: '576',
        valueDetail: '5.24 MB',
        expandable: true,
        expanded: true,
      },
      slots: { icon: '<span class="fixture-icon" />' },
      global: { stubs: { MdIcon: iconStub } },
    });

    expect(wrapper.get('.result-item-title').text()).toContain('Browsing history');
    expect(wrapper.get('.result-item-badge').classes()).toContain('accent');
    expect(wrapper.get('.result-item-description').text()).toBe('Default · Low impact');
    expect(wrapper.get('.result-item-value').text()).toContain('576');
    expect(wrapper.get('.result-item-value').text()).toContain('5.24 MB');
    expect(wrapper.get('.result-item-expand .icon-stub').classes()).toContain('expanded');
  });
});
