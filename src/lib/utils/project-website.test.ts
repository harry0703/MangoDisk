import { describe, expect, it } from 'vitest';
import { projectWebsiteUrl } from './project-website';

describe('localized project website links', () => {
  it.each([
    ['zh-CN', '/zh'],
    ['zh-TW', '/tw'],
    ['ja-JP', '/ja'],
    ['en-US', ''],
    ['unknown', ''],
  ])('maps %s to a stable documentation route without query parameters', (locale, prefix) => {
    expect(projectWebsiteUrl(locale)).toBe(`https://mangodisk.app${prefix}`);
    expect(projectWebsiteUrl(locale, '/docs/ai#custom-service')).toBe(
      `https://mangodisk.app${prefix}/docs/ai#custom-service`
    );
  });
});
