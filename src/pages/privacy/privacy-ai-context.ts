import type { AiContext } from '@/lib/models/ai';
import type { PrivacyItem, PrivacyTimeRange } from '@/lib/models/privacy';

export function privacyAiContext(
  item: PrivacyItem,
  kindName: string,
  timeRange: PrivacyTimeRange,
  platform: AiContext['platform']
): AiContext {
  // Profile labels, tokens, source IDs and detail records are deliberately excluded.
  const source = item.sourceName;
  return {
    schemaVersion: 2,
    platform,
    title: source ? `${source} · ${kindName}` : kindName,
    description: '',
    subject: {
      module: 'privacy',
      timeRange,
      kind: item.kind,
      impact: item.impact,
      capability: item.capability,
      recommendation: item.recommendation,
      itemCount: item.itemCount,
      estimatedBytes: item.estimatedBytes,
      requiresBrowserClose: item.requiresBrowserClose,
      synchronizationMayPropagate: item.synchronizationMayPropagate,
    },
  };
}
