import { PROJECT_LINKS } from '@/lib/models/application-shell';
import { LANGUAGE_OPTIONS } from '@/lib/models/settings';

/** Uses the shared locale registry so desktop help and About links stay aligned. */
export function projectWebsiteUrl(language: string, path = ''): string {
  const prefix = LANGUAGE_OPTIONS.find(option => option.id === language)?.websitePath ?? '';
  return `${PROJECT_LINKS.website}${prefix}${path}`;
}
