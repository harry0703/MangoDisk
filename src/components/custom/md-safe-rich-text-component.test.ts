// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { expect, it, vi } from 'vitest';
import DOMPurify from 'dompurify';
import { LoggerService } from '@/lib/services/logger-service';
import MdSafeRichText from './md-safe-rich-text.vue';

vi.mock('@/lib/services/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));

it('falls back to inert text when the browser cannot sanitize HTML', () => {
  const supported = DOMPurify.isSupported;
  try {
    DOMPurify.isSupported = false;
    const wrapper = mount(MdSafeRichText, { props: { content: '<img src=x onerror=alert(1)>**Keep**' } });
    expect(wrapper.find('img,strong').exists()).toBe(false);
    expect(wrapper.text()).toContain('<img');
    expect(LoggerService.warn).toHaveBeenCalledWith('rich_text', 'sanitizer_unavailable');
    wrapper.unmount();
  } finally {
    DOMPurify.isSupported = supported;
  }
});

it('renders compact, selectable answer formatting as chunks complete', async () => {
  const wrapper = mount(MdSafeRichText, { props: { content: '**Keep', variant: 'answer', allowLinks: false } });
  expect(wrapper.text()).toBe('**Keep');
  await wrapper.setProps({
    content:
      '**Keep the service** if you use it.\n\n- Disable affects future startup.\n- File: `InfoDaemon.BOC`\n\nNo current process is stopped.',
  });
  expect(wrapper.get('strong').text()).toBe('Keep the service');
  expect(wrapper.findAll('li')).toHaveLength(2);
  expect(wrapper.get('code').text()).toBe('InfoDaemon.BOC');
  expect(wrapper.get('.md-safe-rich-text').classes()).toContain('md-safe-rich-text-answer');
  expect(wrapper.findAll('p')).toHaveLength(2);
  await wrapper.setProps({ content: 'First line\nSecond line' });
  expect(wrapper.find('br').exists()).toBe(true);
  wrapper.unmount();
});

it('sanitizes every streaming prefix and keeps model links inert', async () => {
  const content =
    '**Advice**\n\n<img src="https://example.invalid/track" onerror="alert(1)"><script>alert(1)</script><svg onload="alert(1)"></svg><iframe src="https://example.invalid"></iframe><style>body{display:none}</style><input autofocus>\n\n[site](https://example.invalid) [run](javascript:alert(1)) [file](file:///tmp/test)\n\n<strong style="color:red" onclick="alert(1)" data-x="private" aria-label="fake">safe</strong>';
  const wrapper = mount(MdSafeRichText, { props: { content: '', allowLinks: false } });
  for (let end = 1; end <= content.length; end += 7) {
    await wrapper.setProps({ content: content.slice(0, end) });
    expect(
      wrapper
        .find('a,img,script,svg,iframe,style,input,[onclick],[onerror],[style],[href],[data-x],[aria-label]')
        .exists()
    ).toBe(false);
  }
  await wrapper.setProps({ content });
  expect(wrapper.text()).toContain('site');
  expect(wrapper.text()).toContain('safe');
  expect(wrapper.emitted('openLink')).toBeUndefined();
  wrapper.unmount();
});

it('preserves safe release-note link handling without sharing answer policy', async () => {
  const wrapper = mount(MdSafeRichText, {
    props: { content: '[Release](https://example.com/release) ![image](https://example.invalid/image)' },
  });
  expect(wrapper.get('.md-safe-rich-text').classes()).not.toContain('md-safe-rich-text-answer');
  expect(wrapper.find('img').exists()).toBe(false);
  await wrapper.get('a').trigger('click');
  expect(wrapper.emitted('openLink')).toEqual([['https://example.com/release']]);
  await wrapper.setProps({ allowLinks: false });
  expect(wrapper.find('a').exists()).toBe(false);
  wrapper.unmount();
});
