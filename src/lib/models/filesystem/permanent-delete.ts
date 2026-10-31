export interface PermanentDeleteCandidate {
  path: string;
  expectedBytes: number;
  expectedModifiedAtMs: number | null;
}

export type PermanentDeleteFailureReason =
  'accessDenied' | 'resourceBusy' | 'itemChanged' | 'protected' | 'notFound' | 'other';
export type PermanentDeleteFailureStage = 'validate' | 'prepareTarget' | 'createStaging' | 'moveToStaging' | 'remove';

export interface PermanentDeleteFailureDetails {
  message: string;
  // Optional metadata preserves older history records and legacy command fixtures.
  reason?: PermanentDeleteFailureReason;
  stage?: PermanentDeleteFailureStage;
  nativeCode?: number | null;
  partial?: boolean;
  remainingRestored?: boolean;
}

export interface PermanentDeleteFailure extends PermanentDeleteFailureDetails {
  path: string;
}

export interface PermanentDeleteBatchResult {
  removedPaths: string[];
  alreadyAbsentPaths?: string[];
  releasedBytesIsEstimate?: boolean;
  failed: PermanentDeleteFailure[];
  releasedBytes: number;
}
