// @vitest-environment happy-dom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { i18n } from '@/i18n';
import type { StartupArtifact, StartupOwnerGroup } from '@/lib/models/startup';
import MdStartupRow from './md-startup-row.vue';

const service: StartupArtifact = {
  itemId: 'service-fixture',
  sourceId: 'windows.services',
  sourceKind: 'service',
  scope: 'machine',
  triggers: ['boot'],
  displayName: 'Fixture service',
  configurationPath: null,
  target: { kind: 'service', path: null, executableName: null, arguments: [] },
  ownerName: 'Fixture',
  publisher: null,
  summary: null,
  summarySource: 'sourceLabel',
  version: null,
  iconPath: null,
  identityConfidence: 'strong',
  configuredState: 'enabled',
  runtimeState: 'running',
  controlCapability: 'elevationRequired',
  trust: 'unknown',
  modifiedAtMs: null,
  diagnostics: [],
  removalSupported: false,
  removableOrphan: false,
};
const group: StartupOwnerGroup = {
  groupId: 'fixture-group',
  name: 'Fixture',
  publisher: null,
  summary: null,
  summarySource: 'sourceLabel',
  version: null,
  iconPath: null,
  identityConfidence: 'strong',
  itemIds: [service.itemId],
  sourceKinds: ['service'],
  triggers: ['boot'],
  scopes: ['machine'],
  configuredState: 'allEnabled',
  controlState: 'requiresElevation',
  systemItem: false,
};
function row(artifacts: StartupArtifact[] = [service], expanded = false) {
  return mount(MdStartupRow, {
    props: {
      group,
      artifacts,
      subtitle: null,
      startTiming: 'At startup',
      state: 'enabled',
      revealPath: null,
      isWindows: true,
      isMacOs: false,
      expanded,
      busy: false,
      changing: false,
      copiedActionKey: null,
    },
    global: {
      plugins: [i18n],
      stubs: { MdApplicationIcon: true, MdIcon: true, MdIconAction: { template: '<button><slot /></button>' } },
    },
  });
}
describe('startup row source badges and service controls', () => {
  it('explains a collapsed group without toggling, removing or expanding it', async () => {
    const wrapper = row();
    await wrapper.get('.md-ai-action').trigger('click');
    expect(wrapper.emitted('explain')?.[0]).toEqual([group.name, [service]]);
    expect(wrapper.emitted('toggleGroup')).toBeUndefined();
    expect(wrapper.emitted('removeItems')).toBeUndefined();
    expect(wrapper.emitted('toggleExpanded')).toBeUndefined();
    wrapper.unmount();
  });

  it('explains protected services without offering a switch or a system-tool detour', () => {
    const wrapper = row([{ ...service, controlCapability: 'systemManaged' }], true);
    expect(wrapper.find('[role="switch"]').exists()).toBe(false);
    expect(wrapper.get('.startup-management-note').text()).toContain(i18n.global.t('startup.detail.protectedService'));
    expect(wrapper.get('.startup-management-note').find('button').exists()).toBe(false);
    expect(wrapper.get('.startup-state').text()).toBe(i18n.global.t('startup.configuredStates.enabled'));
    wrapper.unmount();
  });
  it('shows one compact source badge and a switch without a system-tool detour', async () => {
    const wrapper = row();
    expect(wrapper.findAll('.md-status-badge')).toHaveLength(1);
    expect(wrapper.get('.md-status-badge').text()).toBe(i18n.global.t('startup.sourceKinds.service'));
    await wrapper.get('[role="switch"]').trigger('click');
    expect(wrapper.emitted('toggleGroup')).toEqual([[]]);
    expect(wrapper.emitted('toggleExpanded')).toBeUndefined();
    expect(wrapper.text()).not.toContain(i18n.global.t('startup.detail.viewOnly'));
    wrapper.unmount();
  });
  it('keeps mixed source groups compact and identifies each expanded member', async () => {
    const artifacts = [service, { ...service, itemId: 'task', sourceKind: 'scheduledTask' as const }];
    const wrapper = row(artifacts);
    expect(wrapper.findAll('.md-status-badge')).toHaveLength(1);
    expect(wrapper.get('.md-status-badge').text()).toBe(i18n.global.t('startup.mixedSources'));
    expect(wrapper.get('.startup-item-count').text()).toBe(i18n.global.t('startup.itemCount', { count: 2 }));
    await wrapper.setProps({ expanded: true });
    expect(wrapper.findAll('.md-status-badge')).toHaveLength(3);
    wrapper.unmount();
  });
  it('keeps metadata in details and the single-item header uncluttered', async () => {
    const wrapper = row();
    await wrapper.setProps({ subtitle: 'A long service description', group: { ...group, version: '1.2.3' } });
    expect(wrapper.text()).not.toContain('A long service description');
    expect(wrapper.text()).not.toContain('1.2.3');
    expect(wrapper.get('.startup-item-count').text()).toBe('');
    expect(wrapper.find('.startup-disclosure .md-status-badge').exists()).toBe(false);
    expect(wrapper.get('.startup-source-slot .md-status-badge').text()).toBe(
      i18n.global.t('startup.sourceKinds.service')
    );
    await wrapper.setProps({ expanded: true });
    expect(wrapper.get('.startup-details').text()).toContain('A long service description');
    expect(wrapper.get('.startup-details').text()).toContain('1.2.3');
    wrapper.unmount();
  });
  it('keeps hover actions separate from expansion and switch events', async () => {
    const wrapper = row([{ ...service, removalSupported: true }]);
    await wrapper.setProps({ revealPath: 'C:\\Fixture\\app.exe' });
    await wrapper.get('.startup-location-action').trigger('click');
    await wrapper.get('.startup-cleanup-action').trigger('click');
    expect(wrapper.emitted('reveal')).toEqual([['C:\\Fixture\\app.exe']]);
    expect(wrapper.emitted('removeItems')).toEqual([[]]);
    expect(wrapper.emitted('toggleExpanded')).toBeUndefined();
    expect(wrapper.emitted('toggleGroup')).toBeUndefined();
    wrapper.unmount();
  });
  it('explains a disabled service that remains running and blocks duplicate clicks', async () => {
    const wrapper = row([{ ...service, configuredState: 'disabled' }], true);
    expect(wrapper.text()).toContain(i18n.global.t('startup.serviceStillRunning'));
    await wrapper.setProps({ busy: true, changing: true });
    expect(wrapper.get('[role="switch"]').attributes('disabled')).toBeDefined();
    expect(wrapper.findAll('.switch-spinner')).toHaveLength(1);
    wrapper.unmount();
  });
});
