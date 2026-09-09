import { describe, expect, it } from 'vitest';

import type { ApplicationUninstallCandidate, ApplicationUninstallComponentSummary } from '@/lib/models/application';

import {
  selectedApplicationBytes,
  selectionIncludesUserData,
  setVisibleApplicationSelection,
  toggleApplicationComponent,
  toggleApplicationSelection,
  type ApplicationUninstallSelection,
} from './application-uninstall-selection';

function component(
  componentId: string,
  options: Partial<ApplicationUninstallComponentSummary> = {}
): ApplicationUninstallComponentSummary {
  return {
    componentId,
    kind: 'cache',
    risk: 'rebuildable',
    path: `/cache/${componentId}`,
    bytes: 10,
    fileCount: 1,
    defaultSelected: true,
    ...options,
  };
}

function candidate(
  applicationId: string,
  components: ApplicationUninstallComponentSummary[]
): ApplicationUninstallCandidate {
  return {
    applicationId,
    primaryIdentifier: applicationId,
    systemKind: 'unclassified',
    sourceIdentities: [{ source: 'macosBundle', identifier: applicationId }],
    name: applicationId,
    version: null,
    publisher: null,
    estimatedBytes: 0,
    lastUsedAtMs: null,
    installedAtMs: null,
    platform: 'macosBundle',
    installerKind: null,
    executionMode: null,
    capability: 'ready',
    recordState: 'installed',
    uninstallDiagnostic: null,
    applicationPath: `/Applications/${applicationId}.app`,
    possibleRelatedPaths: [],
    iconPath: null,
    runningProcesses: [],
    totalBytes: components.reduce((total, value) => total + value.bytes, 0),
    defaultSelectedBytes: components
      .filter(value => value.defaultSelected)
      .reduce((total, value) => total + value.bytes, 0),
    associatedDataComplete: true,
    components,
  };
}

const EMPTY_SELECTION: ApplicationUninstallSelection = {
  applicationIds: [],
  componentIds: {},
};

describe('application uninstall selection', () => {
  it('selects only default components and clears them atomically', () => {
    const application = candidate('editor', [
      component('binary', { kind: 'applicationBinary', risk: 'required' }),
      component('cache'),
      component('documents', { risk: 'userData', defaultSelected: false }),
    ]);

    const selected = toggleApplicationSelection(EMPTY_SELECTION, application);
    expect(selected).toEqual({
      applicationIds: ['editor'],
      componentIds: { editor: ['binary', 'cache'] },
    });
    expect(toggleApplicationSelection(selected, application)).toEqual(EMPTY_SELECTION);
  });

  it('promotes a component choice into an application selection', () => {
    const documents = component('documents', { risk: 'userData', defaultSelected: false });
    const application = candidate('editor', [component('binary', { risk: 'required' }), documents]);

    const selected = toggleApplicationComponent(EMPTY_SELECTION, application, documents);
    expect(selected.applicationIds).toEqual(['editor']);
    expect(selected.componentIds.editor).toEqual(['binary', 'documents']);
    expect(selectionIncludesUserData([application], selected)).toBe(true);
    expect(selectedApplicationBytes([application], selected)).toBe(20);
  });

  it('updates only applications visible to a bulk action', () => {
    const editor = candidate('editor', [component('editor-cache')]);
    const browser = candidate('browser', [component('browser-cache')]);
    const initiallySelected = toggleApplicationSelection(EMPTY_SELECTION, editor);

    const selected = setVisibleApplicationSelection(initiallySelected, [browser], true);
    expect(selected.applicationIds).toEqual(['editor', 'browser']);
    expect(setVisibleApplicationSelection(selected, [browser], false)).toEqual(initiallySelected);
  });

  it('excludes applications without default components from bulk selection', () => {
    const selectable = candidate('selectable', [component('cache')]);
    const viewOnly = candidate('view-only', [component('documents', { defaultSelected: false })]);

    expect(setVisibleApplicationSelection(EMPTY_SELECTION, [selectable, viewOnly], true)).toEqual({
      applicationIds: ['selectable'],
      componentIds: { selectable: ['cache'] },
    });
  });
});
