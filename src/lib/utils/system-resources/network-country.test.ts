import { describe, expect, it } from 'vitest';
import { networkCountryLabel } from './network-country';
describe('network country names', () => {
  it('localizes country names without flags', () => {
    expect(networkCountryLabel('SG', 'zh-CN')).toBe('SG · 新加坡');
    expect(networkCountryLabel('SG', 'en-US')).toBe('SG · Singapore');
    for (const locale of ['zh-TW', 'de-DE', 'es-ES', 'fr-FR', 'ja-JP', 'ko-KR', 'pt-BR', 'ru-RU', 'tr-TR']) {
      expect(networkCountryLabel('CN', locale)).toMatch(/^CN · .+/);
    }
  });
  it('keeps missing or invalid countries absent and handles unavailable locale data', () => {
    for (const code of [null, 'XX', 'T1', 'cn', '']) expect(networkCountryLabel(code, 'en-US')).toBeNull();
    expect(networkCountryLabel('SG', 'invalid_locale')).toBe('SG');
  });
});
