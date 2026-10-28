// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { createPinia } from 'pinia';
import MdConfirmDialog from '@/components/custom/md-confirm-dialog.vue';
import { ResidentService } from '@/lib/services/resident/resident-service';
import { useTrayPanelStore } from '@/stores/tray-panel-store';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import DiskDetails from './md-disk-details.vue';
import NetworkDetails from './md-network-details.vue';
import { OperatingSystemService } from '@/lib/services/settings/operating-system-service';
import { emptyReadings } from '@/lib/utils/system-resources/system-resources';
import type { ResourceReadings } from '@/lib/models/system-resources/system-resources';
import en from '@/locales/en-US.json';
import zh from '@/locales/zh-CN.json';
import tw from '@/locales/zh-TW.json';
import ja from '@/locales/ja-JP.json';
import ko from '@/locales/ko-KR.json';
import pt from '@/locales/pt-BR.json';
import ru from '@/locales/ru-RU.json';
import tr from '@/locales/tr-TR.json';
import uk from '@/locales/uk-UA.json';

vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));

const clipboard = vi.hoisted(() => ({ writeText: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@/lib/services/shell/clipboard-service', () => ({ ClipboardService: clipboard }));

const wrappers: ReturnType<typeof mount>[] = [];
beforeEach(() => vi.spyOn(OperatingSystemService, 'currentPlatform').mockReturnValue('macos'));
afterEach(() => {
  wrappers.splice(0).forEach(wrapper => wrapper.unmount());
  vi.restoreAllMocks();
});
function render(component: typeof DiskDetails | typeof NetworkDetails, reading: ResourceReadings, messages = en) {
  const wrapper = mount(component, {
    props: { reading, active: true },
    global: {
      plugins: [createPinia(), createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': messages } })],
      stubs: { MdResourceOverview: true },
    },
  });
  wrappers.push(wrapper);
  return wrapper;
}
function disk(): ResourceReadings {
  const reading = emptyReadings();
  reading.observedAtMs = 10000;
  reading.disk = {
    status: 'ready',
    sampledAtMs: 1000,
    value: {
      volume: { id: 'one', name: 'System', system: true, fileSystem: 'APFS', mountPoints: [], devices: [] },
      totalBytes: 100,
      availableBytes: 50,
      usedBytes: 50,
      usedPercent: 50,
    },
  };
  reading.diskHealth = {
    status: 'ready',
    sampledAtMs: 1000,
    value: {
      volumeId: 'one',
      volumeName: 'System',
      source: 'nvmeSmart',
      status: 'healthy',
      percentageUsed: 0,
      totalWrittenBytes: '0',
      totalReadBytes: '1024',
      powerCycles: 12,
      availableSpare: 100,
      criticalWarning: 0,
      powerOnHours: 0,
      mediaErrors: '0',
      temperatureCelsius: null,
      ata: null,
      nvme: null,
    },
  };
  return reading;
}

describe('disk health and network detail presentation', () => {
  it('shows decoded SATA lifetime summaries without rendering raw attributes or changing metadata order', () => {
    const reading = disk();
    Object.assign(reading.diskHealth.value!, {
      source: 'ataSmart',
      percentageUsed: 33,
      totalWrittenBytes: '65205638537216',
      totalReadBytes: '545421335363584',
      powerOnHours: 16466,
      powerCycles: 1143,
      availableSpare: null,
      criticalWarning: null,
      mediaErrors: null,
      ata: {
        model: 'Acer AS500A 128G',
        firmware: 'Q1115A0',
        profile: 'siliconMotionLegacy',
        revision: 1,
        attributes: [],
      },
    });
    const wrapper = render(DiskDetails, reading);
    expect(wrapper.text()).toContain('67%');
    expect(wrapper.text()).toContain('16,466');
    expect(wrapper.text()).toContain('1,143');
    expect(wrapper.findAll('.health-metadata dt').map(row => row.text())).toEqual([
      en.diskHealth.read,
      en.diskHealth.temperature,
      en.diskHealth.powerCycles,
    ]);
    expect(
      wrapper
        .findAllComponents({ name: 'MdTooltip' })
        .some(component => component.props('text') === 'Acer AS500A 128G · Q1115A0')
    ).toBe(false);
    expect(wrapper.find('.volume-identity').attributes('data-slot')).toBeUndefined();
    expect(wrapper.text()).not.toContain('Q1115A0');
  });
  it('anchors the health calculation hint to its field instead of the section title', () => {
    const wrapper = render(DiskDetails, disk());
    expect(wrapper.get('.disk-health h2').attributes('data-slot')).toBeUndefined();
    const field = wrapper.get('.disk-health .resource-fact dt [data-slot="tooltip-trigger"]');
    expect(field.text()).toBe(en.diskHealth.healthPercent);
    expect(
      wrapper
        .findAllComponents({ name: 'MdTooltip' })
        .find(tip => tip.props('text') === en.diskHealth.wearHint)
        ?.text()
    ).toBe(en.diskHealth.healthPercent);
  });
  it('keeps one tooltip anchor while disk health loads and becomes unavailable', async () => {
    const reading = disk();
    const loading: ResourceReadings = {
      ...reading,
      diskHealth: { status: 'loading', sampledAtMs: null, value: null },
    };
    const wrapper = render(DiskDetails, loading);
    const anchor = wrapper.get('.disk-health header [data-slot="tooltip-trigger"]:last-child').element;
    for (const current of [reading, loading, reading]) {
      await wrapper.setProps({ reading: { ...current } });
      expect(wrapper.get('.disk-health header [data-slot="tooltip-trigger"]:last-child').element).toBe(anchor);
    }
  });
  it.each([en, zh, tw, ja, ko, pt, ru, tr, uk])(
    'shows supported zero readings and health in every locale',
    messages => {
      const reading = disk();
      reading.diskDetailTemperature = {
        status: 'ready',
        sampledAtMs: 1000,
        value: { volumeId: 'one', volumeName: 'System', celsius: 35, source: 'nvmeSmart' },
      };
      const wrapper = render(DiskDetails, reading, messages);
      expect(wrapper.text()).toContain(messages.diskHealth.temperature);
      expect(wrapper.text()).toContain(messages.diskHealth.healthy);
      expect(wrapper.text()).toContain(messages.diskHealth.healthPercent);
      expect(wrapper.text()).toContain('100%');
      expect(wrapper.text()).not.toContain(messages.diskHealth.unavailable);
    }
  );
  it('clamps exhausted health to zero without inventing missing SMART details', async () => {
    const reading = disk();
    reading.diskHealth.value!.percentageUsed = 150;
    const wrapper = render(DiskDetails, reading);
    expect(wrapper.text()).toContain('0%');
    reading.diskHealth = {
      ...reading.diskHealth,
      value: {
        ...reading.diskHealth.value!,
        source: 'windowsStorage',
        percentageUsed: null,
        totalWrittenBytes: null,
        powerOnHours: null,
        totalReadBytes: null,
        powerCycles: null,
        availableSpare: null,
        criticalWarning: null,
        mediaErrors: null,
      },
    };
    await wrapper.setProps({ reading: { ...reading } });
    expect(wrapper.text()).not.toContain(en.diskHealth.systemHint);
    expect(wrapper.find('.disk-health').findComponent({ name: 'MdTooltip' }).props('text')).toBe(
      en.diskHealth.systemHint
    );
    expect(wrapper.find('.disk-health > .resource-facts:not(.health-metadata)').exists()).toBe(false);
    expect(wrapper.findAll('.health-metadata dt').map(row => row.text())).toEqual([en.diskHealth.temperature]);
    expect(wrapper.find('.health-metadata dd').text()).toBe('—');
  });
  it('keeps failed health fields with the failure state until refresh succeeds', async () => {
    const reading = disk();
    reading.diskHealth.status = 'failed';
    const wrapper = render(DiskDetails, reading);

    expect(wrapper.get('.health-status').text()).toContain(en.systemStatus.failed);
    expect(wrapper.text()).toContain('100%');
    await wrapper.setProps({ reading: disk() });
    expect(wrapper.get('.health-status').text()).toBe(en.diskHealth.healthy);
  });
  it('keeps expired health fields visible while the same disk refreshes in the background', () => {
    const reading = disk();
    reading.observedAtMs = 601_001;
    reading.diskHealth.status = 'stale';
    const wrapper = render(DiskDetails, reading);
    expect(wrapper.text()).toContain(en.diskHealth.healthPercent);
    expect(wrapper.text()).toContain('100%');
    expect(wrapper.get('.health-status').text()).toBe(en.diskHealth.healthy);
    expect(wrapper.text()).not.toContain(en.diskHealth.unavailable);
  });
  it('never restores expired health for a changed disk or a future observation', async () => {
    const reading = disk();
    reading.observedAtMs = 601_001;
    reading.diskHealth.status = 'stale';
    reading.diskHealth.value!.volumeId = 'other';
    const wrapper = render(DiskDetails, reading);
    expect(wrapper.find('.health-status').exists()).toBe(false);
    reading.diskHealth.value!.volumeId = 'one';
    reading.diskHealth.sampledAtMs = reading.observedAtMs + 1;
    await wrapper.setProps({ reading: { ...reading } });
    expect(wrapper.find('.health-status').exists()).toBe(false);
  });
  it.each(['disconnected', 'unsupported'] as const)('hides an old healthy observation after %s', status => {
    const reading = disk();
    reading.diskHealth.status = status;
    const wrapper = render(DiskDetails, reading);
    expect(wrapper.text()).toContain(en.diskHealth.unavailable);
    expect(wrapper.find('.health-status').exists()).toBe(false);
  });
  it('keeps the temperature slot and metadata order while live readings are unavailable', async () => {
    const reading = disk();
    const wrapper = render(DiskDetails, reading);
    const labels = () => wrapper.findAll('.health-metadata dt').map(row => row.text());
    const before = labels();
    expect(before).toContain(en.diskHealth.temperature);
    reading.diskDetailTemperature = {
      status: 'ready',
      sampledAtMs: 2000,
      value: {
        volumeId: 'one',
        volumeName: 'System',
        celsius: 34,
        source: 'nvmeSmart',
      },
    };
    await wrapper.setProps({ reading: { ...reading } });
    expect(labels()).toEqual(before);
    expect(wrapper.text()).toContain('34°C');
  });
  it('keeps the Windows basic health temperature slot through missing and recovered readings', async () => {
    const reading = disk();
    reading.diskHealth.value = {
      ...reading.diskHealth.value!,
      source: 'windowsStorage',
      percentageUsed: null,
      totalWrittenBytes: null,
      totalReadBytes: null,
      powerCycles: null,
      availableSpare: null,
      criticalWarning: null,
      powerOnHours: null,
      mediaErrors: null,
    };
    reading.diskDetailTemperature = {
      status: 'ready',
      sampledAtMs: 2000,
      value: { volumeId: 'one', volumeName: 'System', celsius: 34, source: 'windowsStorage' },
    };
    const wrapper = render(DiskDetails, reading);
    const labels = () => wrapper.findAll('.health-metadata dt').map(row => row.text());
    const before = labels();
    expect(before).toContain(en.diskHealth.temperature);
    for (const status of ['loading', 'failed', 'unsupported'] as const) {
      await wrapper.setProps({
        reading: { ...reading, diskDetailTemperature: { status, sampledAtMs: null, value: null } },
      });
      expect(labels()).toEqual(before);
      expect(wrapper.find('.health-metadata').text()).not.toContain('34°C');
    }
    await wrapper.setProps({ reading: { ...reading } });
    expect(labels()).toEqual(before);
    expect(wrapper.find('.health-metadata').text()).toContain('34°C');
  });
  it('uses a fresh temperature for the selected disk instead of the cached SMART temperature', async () => {
    const reading = disk();
    reading.diskHealth.value!.temperatureCelsius = 99;
    reading.diskDetailTemperature = {
      status: 'ready',
      sampledAtMs: 2000,
      value: {
        volumeId: 'one',
        volumeName: 'System',
        celsius: 35,
        source: 'nvmeSmart',
      },
    };
    reading.diskTemperature = {
      status: 'ready',
      sampledAtMs: 2000,
      value: { volumeId: 'other', volumeName: 'Archive', celsius: 80, source: 'nvmeSmart' },
    };
    const wrapper = render(DiskDetails, reading);
    expect(wrapper.text()).not.toContain('80°C');
    expect(wrapper.text()).toContain('35°C');
    expect(wrapper.text()).not.toContain('99°C');
    await wrapper.setProps({
      reading: {
        ...reading,
        diskDetailTemperature: {
          ...reading.diskDetailTemperature,
          value: { ...reading.diskDetailTemperature.value!, volumeId: 'other' },
        },
      },
    });
    expect(wrapper.text()).not.toContain('35°C');
  });
  it('does not attribute another physical volume’s health to the current selection', () => {
    const reading = disk();
    reading.diskHealth.value!.volumeId = 'other';
    const wrapper = render(DiskDetails, reading);
    expect(wrapper.find('.health-status').exists()).toBe(false);
    expect(wrapper.find('.disk-health .resource-facts').exists()).toBe(false);
  });
  it('formats link rates as bits and keeps valid zero session traffic distinct from missing data', async () => {
    const reading = emptyReadings();
    reading.observedAtMs = 10000;
    reading.network = {
      status: 'ready',
      sampledAtMs: 1000,
      value: {
        interface: {
          id: 'lan',
          name: 'Ethernet',
          kind: 'ethernet',
          connected: true,
          physical: true,
          details: { addresses: ['192.0.2.1', '2001:db8::1'], hardwareAddress: '02:00:00:00:00:01', wifi: null },
          defaultRouteMetric: 1,
        },
        receivedBytesPerSecond: 0,
        transmittedBytesPerSecond: 0,
        selectionReason: 'defaultRoute',
        cumulative: { receivedBytes: '0', transmittedBytes: '100000', startedAtMs: 1000 },
        receivedBytesSinceStart: 0,
        transmittedBytesSinceStart: 1024,
        receiveLinkBps: 1e9,
        transmitLinkBps: 1e8,
      },
    };
    const wrapper = render(NetworkDetails, reading);
    expect(wrapper.text()).toContain('↓ 1 Gbps · ↑ 100 Mbps');
    expect(wrapper.findAll('dd')[0]!.text()).toContain('0');
    expect(wrapper.text()).toContain('192.0.2.1');
    expect(wrapper.text()).toContain('2001:db8::1');
    expect(wrapper.text()).toContain('02:00:00:00:00:01');
    const mac = wrapper.get('button[aria-label="Copy: Current MAC address"]');
    await mac.trigger('click');
    await flushPromises();
    expect(clipboard.writeText).toHaveBeenLastCalledWith('02:00:00:00:00:01');
    const ip = wrapper.get('button[aria-label="Copy: IPv4"]');
    await ip.trigger('click');
    await flushPromises();
    expect(clipboard.writeText).toHaveBeenLastCalledWith('192.0.2.1');
    expect(wrapper.text()).not.toContain(en.networkDetails.sessionHint);
    expect(
      wrapper
        .findAllComponents({ name: 'MdTooltip' })
        .some(component => component.props('text') === en.networkDetails.sessionHint)
    ).toBe(true);
    const reset = vi.spyOn(ResidentService, 'resetNetworkTraffic');
    let finish!: (value: Awaited<ReturnType<typeof ResidentService.resetNetworkTraffic>>) => void;
    reset.mockImplementationOnce(
      () =>
        new Promise(resolve => {
          finish = resolve;
        })
    );
    await wrapper.get('.traffic-reset').trigger('click');
    const dialog = wrapper.getComponent(MdConfirmDialog);
    expect(dialog.props('description')).toContain('Ethernet');
    dialog.vm.$emit('confirm');
    await flushPromises();
    expect(reset).toHaveBeenCalledExactlyOnceWith('lan', 1000);
    expect(dialog.props('busy')).toBe(true);
    const accepted = { ...reading, revision: 50 };
    finish(accepted);
    await flushPromises();
    expect(useTrayPanelStore().reading.revision).toBe(50);
    expect(wrapper.findComponent(MdConfirmDialog).exists()).toBe(false);
    reset.mockRejectedValueOnce(new Error('write failed'));
    await wrapper.get('.traffic-reset').trigger('click');
    wrapper.getComponent(MdConfirmDialog).vm.$emit('confirm');
    await flushPromises();
    expect(document.querySelector('[role=alert]')?.textContent).toBe(en.networkDetails.resetFailed);
    expect(wrapper.findComponent(MdConfirmDialog).props('busy')).toBe(false);
    reset.mockResolvedValueOnce({ ...accepted, revision: 51 });
    wrapper.getComponent(MdConfirmDialog).vm.$emit('confirm');
    await flushPromises();
    expect(wrapper.findComponent(MdConfirmDialog).exists()).toBe(false);
    expect(document.querySelector('[role=alert]')).toBeNull();
    expect(useTrayPanelStore().reading.revision).toBe(51);
    const wifi = {
      ...reading.network.value!,
      receiveLinkBps: null,
      transmitLinkBps: 2401000000,
      interface: {
        ...reading.network.value!.interface,
        kind: 'wifi' as const,
        details: {
          ...reading.network.value!.interface.details!,
          wifi: { standard: '802.11ax', channel: 36, bandGhz: 5, channelWidthMhz: 160, signalDbm: -41, noiseDbm: -91 },
        },
      },
    };
    await wrapper.setProps({ reading: { ...reading, network: { ...reading.network, value: wifi } } });
    expect(wrapper.text()).toContain('2.4 Gbps');
    expect(wrapper.text()).not.toContain('↓');
    expect(wrapper.text()).toContain('802.11ax');
    expect(wrapper.text()).toContain('36 (5 GHz, 160 MHz)');
    expect(wrapper.text()).not.toContain('-41 / -91 dBm');
    await wrapper.setProps({ reading: { ...reading, network: { ...reading.network, status: 'stale' } } });
    expect(wrapper.text()).toContain('↓ 1 Gbps · ↑ 100 Mbps');
    expect(wrapper.text()).toContain('192.0.2.1');
    expect(wrapper.text()).toContain('02:00:00:00:00:01');
    await wrapper.setProps({ reading: { ...reading, network: { ...reading.network, status: 'disconnected' } } });
    expect(wrapper.findAll('dd').every(value => value.text() === '—')).toBe(true);
  });
});
