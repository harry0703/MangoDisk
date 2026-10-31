import type { Composer } from 'vue-i18n';
import type { PermanentDeleteBatchResult } from '@/lib/models/filesystem/permanent-delete';

export const DUPLICATE_FILE_DELETE_TOAST_ID = 'duplicate-file-delete-result';

export function duplicateFileDeleteNotification(
  result: PermanentDeleteBatchResult,
  t: Composer['t'],
  format: { integer: (value: number) => string; bytes: (value: number) => string },
  showDetails: () => void
) {
  const hasFailures = result.failed.length > 0;
  const absentCount = result.alreadyAbsentPaths?.length ?? 0;
  const absentDescription = absentCount ? `${t('fileDeletion.reasons.notFound')}: ${format.integer(absentCount)}` : '';
  const description =
    hasFailures && !result.removedPaths.length
      ? t('fileDeletion.failedSummary', { failed: format.integer(result.failed.length) })
      : t(
          'duplicateFiles.deleteCompletedDescription',
          {
            count: format.integer(result.removedPaths.length),
            size: `${result.releasedBytesIsEstimate ? '~' : ''}${format.bytes(result.releasedBytes)}`,
            failed: format.integer(result.failed.length),
          },
          result.removedPaths.length
        );
  return {
    title: t(
      hasFailures
        ? result.removedPaths.length
          ? 'fileDeletion.partialTitle'
          : 'fileDeletion.failedTitle'
        : 'duplicateFiles.deleteCompleted'
    ),
    tone: hasFailures ? ('warning' as const) : ('success' as const),
    options: {
      id: DUPLICATE_FILE_DELETE_TOAST_ID,
      description: description + (absentDescription ? ` · ${absentDescription}` : ''),
      // Same-ID notifications merge their options, so success must clear the prior action.
      action: hasFailures ? { label: t('fileDeletion.viewDetails'), onClick: showDetails } : undefined,
    },
  };
}
