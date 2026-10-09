import type { ApplicationCloseItem } from '@/lib/models/application-close';
import type { ApplicationCloseBatchResult } from '@/lib/models/application-close';
import type {
  CleanupApplicationCloseIdentity,
  CleanupApplicationIcon,
  PresentedScanRuleResult,
} from '@/lib/models/cleanup';

export interface CleanupApplicationCloseGroup extends ApplicationCloseItem {
  ruleIds: string[];
  applicationId?: string;
}

export interface CleanupApplicationCloseRetry {
  items: ApplicationCloseItem[];
  ruleIds: string[];
}

/**
 * Groups cleanup rules by Core-owned application identity. Legacy responses
 * without ownership retain process grouping until the next scan.
 * Several cache rules may belong to the same application, so presenting one
 * row per rule would ask users to close the same application repeatedly.
 */
export function cleanupApplicationCloseGroups(
  rules: readonly PresentedScanRuleResult[],
  applicationIcons: readonly CleanupApplicationIcon[] = [],
  applicationIdentities: readonly CleanupApplicationCloseIdentity[] = []
): CleanupApplicationCloseGroup[] {
  const iconPaths = new Map(applicationIcons.map(item => [normalizeProcess(item.processName), item.iconPath] as const));
  const identities = new Map(applicationIdentities.map(identity => [identity.ruleId, identity]));
  const groups: CleanupApplicationCloseGroup[] = [];
  for (const rule of rules.filter(item => item.requiresAppClose && item.runningProcesses.length)) {
    const identity = identities.get(rule.ruleId);
    const iconPath = identity
      ? (identity.iconPath ?? undefined)
      : rule.runningProcesses.map(process => iconPaths.get(normalizeProcess(process))).find(Boolean);
    const normalized = new Set(rule.runningProcesses.map(normalizeProcess));
    const overlapping = groups.filter(group =>
      identity
        ? group.applicationId === identity.applicationId
        : !group.applicationId && group.processes.some(process => normalized.has(normalizeProcess(process)))
    );
    if (!overlapping.length) {
      groups.push({
        id: rule.ruleId,
        applicationId: identity?.applicationId,
        iconPath,
        name: identity?.applicationName ?? rule.name,
        processes: [...new Set(rule.runningProcesses)],
        ruleIds: [rule.ruleId],
      });
      continue;
    }

    const primary = overlapping[0];
    primary.iconPath ??= iconPath;
    primary.processes = uniqueCaseInsensitive([...primary.processes, ...rule.runningProcesses]);
    primary.ruleIds = [...new Set([...primary.ruleIds, rule.ruleId])];
    for (const merged of overlapping.slice(1)) {
      primary.processes = uniqueCaseInsensitive([...primary.processes, ...merged.processes]);
      primary.iconPath ??= merged.iconPath;
      primary.ruleIds = [...new Set([...primary.ruleIds, ...merged.ruleIds])];
      groups.splice(groups.indexOf(merged), 1);
    }
  }
  return groups.map(group => ({
    ...group,
    // Safari 15.6 does not provide Array.prototype.toSorted. Copy before
    // sorting so the stable group ID remains deterministic without mutating
    // the authorization list owned by the cleanup rule.
    id: [...group.ruleIds].sort().join(':'),
  }));
}

/**
 * Narrows the destructive retry to targets that Core could not confirm as
 * stopped. The returned IDs are the exact authorization sent by the force
 * action; display grouping must never broaden that request back to the user's
 * original selection.
 */
export function cleanupApplicationCloseRetry(
  groups: readonly CleanupApplicationCloseGroup[],
  result: ApplicationCloseBatchResult
): CleanupApplicationCloseRetry {
  const retryTargets = result.targets.filter(
    target => target.status === 'failed' || target.remainingProcesses.length > 0
  );
  const ruleIds = retryTargets.map(target => target.targetId);
  const retryRules = new Set(ruleIds);
  const remainingProcesses = new Set(retryTargets.flatMap(target => target.remainingProcesses).map(normalizeProcess));

  return {
    ruleIds,
    items: groups
      .filter(group => group.ruleIds.some(ruleId => retryRules.has(ruleId)))
      .map(group => {
        const matchingProcesses = group.processes.filter(process => remainingProcesses.has(normalizeProcess(process)));
        return {
          ...group,
          processes: matchingProcesses.length ? matchingProcesses : group.processes,
        };
      }),
  };
}

function uniqueCaseInsensitive(values: string[]): string[] {
  const seen = new Set<string>();
  return values.filter(value => {
    const normalized = normalizeProcess(value);
    if (seen.has(normalized)) return false;
    seen.add(normalized);
    return true;
  });
}

function normalizeProcess(value: string): string {
  return value.trim().toLocaleLowerCase();
}
