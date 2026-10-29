/** Region names come from the runtime's locale data, without flags or a second lookup. */
export function networkCountryLabel(code: string | null, locale: string): string | null {
  if (!code || !/^[A-Z]{2}$/.test(code) || code === 'XX') return null;
  try {
    const name = new Intl.DisplayNames([locale], { type: 'region', fallback: 'none' }).of(code);
    return name ? `${code} · ${name}` : code;
  } catch {
    return code;
  }
}
