// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createI18n, type Composer } from 'vue-i18n';
import { toast } from 'vue-sonner';
import en from '@/locales/en-US.json';
import type { PermanentDeleteBatchResult } from '@/lib/models/filesystem/permanent-delete';
import { duplicateFileDeleteNotification, DUPLICATE_FILE_DELETE_TOAST_ID } from './duplicate-file-delete-notification';

const t = createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': en } }).global.t as Composer['t'];
const format = { integer: String, bytes: (value: number) => `${value} B` };
const failed: PermanentDeleteBatchResult = {
  removedPaths: [],
  releasedBytes: 0,
  failed: [{ path: '/fixture/duplicate', reason: 'accessDenied', message: 'denied' }],
};

function publish(result: PermanentDeleteBatchResult, showDetails = vi.fn()) {
  const notification = duplicateFileDeleteNotification(result, t, format, showDetails);
  toast[notification.tone](notification.title, notification.options);
  return notification;
}

function currentToast() {
  return toast.getHistory().find(item => item.id === DUPLICATE_FILE_DELETE_TOAST_ID);
}

afterEach(() => toast.dismiss(DUPLICATE_FILE_DELETE_TOAST_ID));

describe('duplicate file deletion notifications', () => {
  it('clears the failure action when a subsequent batch succeeds', () => {
    const oldDetails = vi.fn();
    publish(failed, oldDetails);
    expect(currentToast()).toMatchObject({ type: 'warning', action: { onClick: oldDetails } });
    const success = publish({ removedPaths: ['/fixture/removed'], failed: [], releasedBytes: 8 });
    expect(success.title).toBe(t('duplicateFiles.deleteCompleted'));
    const current = currentToast();
    expect(current).toMatchObject({ type: 'success', description: success.options.description });
    expect(current && 'action' in current ? current.action : undefined).toBeUndefined();
    expect(oldDetails).not.toHaveBeenCalled();
  });

  it('replaces details with the latest failure and distinguishes partial results', () => {
    const previousDetails = vi.fn();
    const latestDetails = vi.fn();
    expect(publish(failed, previousDetails).title).toBe(t('fileDeletion.failedTitle'));
    const partial = publish({ ...failed, removedPaths: ['/fixture/removed'], releasedBytes: 8 }, latestDetails);
    expect(partial.title).toBe(t('fileDeletion.partialTitle'));
    const current = currentToast();
    const action = current && 'action' in current ? current.action : undefined;
    expect(action).toMatchObject({ onClick: latestDetails });
    if (action && typeof action === 'object' && 'onClick' in action) action.onClick?.(new MouseEvent('click'));
    expect(latestDetails).toHaveBeenCalledOnce();
    expect(previousDetails).not.toHaveBeenCalled();
  });
});
