import type { CleanupSourceDetail, CleanupSourceSelection, RiskLevel, ScanRuleResult } from '@/lib/models/cleanup';

/** Project writers require confirmation and an execution-time guard, not a disabled selection. */
export function sourceSelectable(rule: ScanRuleResult, source: CleanupSourceDetail): boolean {
  return (
    rule.selectable && (!source.blockReason || (rule.category === 'project' && source.blockReason === 'requiresClose'))
  );
}

/** Pure calculations for cleanup selection state and byte totals. */
export interface CleanupRuleSelectionState {
  allSelected: boolean;
  selectedCount: number;
  selectableRuleIds: string[];
  someSelected: boolean;
}
export type CleanupRuleSelectionLevel = 'all' | 'partial' | 'none';
export type CleanupSelectionMode = 'smart' | 'all' | 'none' | 'manual';
export function defaultSelectedRuleIds(rules: readonly ScanRuleResult[]): string[] {
  return recommendedRuleIds(rules);
}
export function recommendedRuleIds(rules: readonly ScanRuleResult[]): string[] {
  return rules
    .filter(rule => rule.recommendedSelected && bulkSelectableBytesForRule(rule) > 0)
    .map(rule => rule.ruleId);
}
export function selectableRuleIds(rules: readonly ScanRuleResult[]): string[] {
  return rules.filter(rule => rule.selectable).map(rule => rule.ruleId);
}
export function bulkSelectableRuleIds(rules: readonly ScanRuleResult[]): string[] {
  return rules.filter(rule => bulkSelectableBytesForRule(rule) > 0).map(rule => rule.ruleId);
}
export function foundBytes(rules: readonly ScanRuleResult[]): number {
  return rules.reduce((total, rule) => total + (rule.selectable ? rule.bytes : 0), 0);
}
export function selectableBytes(rules: readonly ScanRuleResult[]): number {
  return rules.reduce((total, rule) => total + bulkSelectableBytesForRule(rule), 0);
}
export function recommendedBytes(rules: readonly ScanRuleResult[]): number {
  return rules.reduce((total, rule) => total + (rule.recommendedSelected ? bulkSelectableBytesForRule(rule) : 0), 0);
}
export function selectionMode(
  rules: readonly ScanRuleResult[],
  selectedRuleIds: readonly string[],
  sourceSelections: readonly CleanupSourceSelection[]
): CleanupSelectionMode {
  const selected = new Set(selectedRuleIds);
  const selectable = bulkSelectableRuleIds(rules);
  if (!selected.size) return 'none';
  const matches = (ruleIds: readonly string[]) => {
    const expected = new Set(ruleIds);
    if (
      selected.size !== expected.size ||
      !ruleIds.every(ruleId => selected.has(ruleId)) ||
      sourceSelections.some(selection => !expected.has(selection.ruleId))
    ) {
      return false;
    }
    return rules
      .filter(rule => expected.has(rule.ruleId))
      .every(rule => ruleSelectionLevel(rule, selectedRuleIds, sourceSelections) === 'all');
  };
  if (matches(recommendedRuleIds(rules))) return 'smart';
  if (matches(selectable)) return 'all';
  return 'manual';
}
export function selectableByRisk(rules: readonly ScanRuleResult[], risk: RiskLevel): ScanRuleResult[] {
  return rules.filter(rule => rule.risk === risk && rule.selectable);
}
export function selectedRules<Rule extends ScanRuleResult>(
  rules: readonly Rule[],
  selectedRuleIds: readonly string[]
): Rule[] {
  const selectedIds = new Set(selectedRuleIds);
  return rules.filter(rule => selectedIds.has(rule.ruleId));
}
export function ruleSelectionLevel(
  rule: ScanRuleResult,
  selectedRuleIds: readonly string[],
  sourceSelections: readonly CleanupSourceSelection[]
): CleanupRuleSelectionLevel {
  if (!selectedRuleIds.includes(rule.ruleId)) return 'none';
  const selection = sourceSelections.find(item => item.ruleId === rule.ruleId);
  if (!selection) return 'all';
  if (selection.mode === 'exclude') {
    return selection.paths.length ? 'partial' : 'all';
  }
  const selectableSources = rule.sources.filter(source => sourceSelectable(rule, source));
  if (
    !rule.sourcesTruncated &&
    selectableSources.length > 0 &&
    selectableSources.every(source => selection.paths.includes(source.path))
  ) {
    return 'all';
  }
  return selection.paths.length ? 'partial' : 'none';
}
export function sourceSelected(
  ruleId: string,
  sourcePath: string,
  selectedRuleIds: readonly string[],
  sourceSelections: readonly CleanupSourceSelection[]
): boolean {
  if (!selectedRuleIds.includes(ruleId)) return false;
  const selection = sourceSelections.find(item => item.ruleId === ruleId);
  if (!selection) return true;
  const contains = selection.paths.includes(sourcePath);
  return selection.mode === 'include' ? contains : !contains;
}
export function selectedBytes(
  rules: readonly ScanRuleResult[],
  selectedRuleIds: readonly string[],
  sourceSelections: readonly CleanupSourceSelection[]
): number {
  const selectedIds = new Set(selectedRuleIds);
  const selectionsByRule = new Map(sourceSelections.map(selection => [selection.ruleId, selection]));
  return rules.reduce((total, rule) => {
    if (!selectedIds.has(rule.ruleId)) return total;
    const selection = selectionsByRule.get(rule.ruleId);
    if (!selection) return total + rule.bytes;
    const selectedPaths = new Set(selection.paths);
    const overriddenBytes = rule.sources.reduce(
      (bytes, source) => bytes + (selectedPaths.has(source.path) ? source.bytes : 0),
      0
    );
    return total + (selection.mode === 'include' ? overriddenBytes : Math.max(0, rule.bytes - overriddenBytes));
  }, 0);
}
export function selectedBytesForRule(
  rule: ScanRuleResult,
  selectedRuleIds: readonly string[],
  sourceSelections: readonly CleanupSourceSelection[]
): number {
  return selectedBytes([rule], selectedRuleIds, sourceSelections);
}
function bulkSelectableBytesForRule(rule: ScanRuleResult): number {
  if (!rule.selectable) return 0;
  // Truncated source inventories cannot safely reconstruct the whole rule, so
  // bulk selection retains the Core-owned aggregate. Complete inventories can
  // exclude disabled sources and keep the preset amount equal to what the UI
  // will actually select.
  if (rule.sourcesTruncated || rule.sources.every(source => sourceSelectable(rule, source))) {
    return rule.bytes;
  }
  return rule.sources.reduce((total, source) => total + (sourceSelectable(rule, source) ? source.bytes : 0), 0);
}
export function selectionState(
  rules: readonly ScanRuleResult[],
  selectedRuleIds: readonly string[]
): CleanupRuleSelectionState {
  const selectableRuleIds = rules.filter(rule => rule.selectable).map(rule => rule.ruleId);
  const selectedIds = new Set(selectedRuleIds);
  const selectedCount = selectableRuleIds.reduce((count, ruleId) => count + Number(selectedIds.has(ruleId)), 0);
  return {
    allSelected: selectableRuleIds.length > 0 && selectedCount === selectableRuleIds.length,
    selectedCount,
    selectableRuleIds,
    someSelected: selectedCount > 0 && selectedCount < selectableRuleIds.length,
  };
}
