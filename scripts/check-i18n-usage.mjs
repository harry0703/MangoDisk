/**
 * Rejects locale fields that have no production-code consumer. Literal keys
 * are discovered from frontend source, while typed runtime keys are listed
 * explicitly so a removed enum value cannot leave translation work behind.
 */
import { readFileSync, readdirSync } from 'node:fs';
import { extname, join, resolve } from 'node:path';

const projectRoot = resolve(import.meta.dirname, '..');
const sourceRoot = join(projectRoot, 'src');
const coreRoot = join(projectRoot, 'src-tauri', 'crates', 'mangodisk-core');
const localePaths = [
  'src/locales/zh-CN.json',
  'src/locales/zh-TW.json',
  'src/locales/en-US.json',
  'src/locales/ja-JP.json',
];
const sourceExtensions = new Set(['.ts', '.vue']);

const commandErrorCodes = [
  'operationBusy',
  'invalidInput',
  'operationCancelled',
  'operationFailed',
  'permissionDenied',
  'persistenceFailed',
  'taskJoinFailed',
];
const dynamicKeyGroups = {
  navigation: [
    'cleanup',
    'analysis',
    'large-files',
    'duplicate-files',
    'application-uninstall',
    'startup',
    'history',
    'settings',
  ],
  errors: commandErrorCodes,
  errorTitles: commandErrorCodes,
  'folderPicker.standardFolders': ['downloads', 'documents', 'pictures', 'videos', 'music'],
  fileCategories: ['all', 'video', 'audio', 'document', 'installer', 'archive', 'image', 'aiModel', 'other'],
  'cleanup.categoryTitles': [
    'system',
    'userCache',
    'application',
    'browser',
    'development',
    'project',
    'xcode',
    'applicationOptimization',
    'ai',
    'container',
  ],
  'cleanup.categoryDescriptions': [
    'system',
    'userCache',
    'application',
    'browser',
    'development',
    'project',
    'xcode',
    'applicationOptimization',
    'ai',
    'container',
  ],
  'cleanup.selectionState': ['all', 'partial', 'none'],
  'cleanup.selectionMode': ['label', 'smart', 'all', 'none', 'manual'],
  'largeFiles.selectionMode': ['label', 'smart', 'all', 'none', 'manual', 'analyzing'],
  'applicationLeftovers.sources': [
    'sandboxContainer',
    'applicationSupport',
    'preferences',
    'logs',
    'savedState',
    'webData',
    'applicationScripts',
  ],
  applicationUninstall: [
    'all',
    'ready',
    'running',
    'unavailable',
    'applicationRunning',
    'requiresElevation',
    'readyForReview',
    'viewOnly',
    'orphanedRegistration',
  ],
  'applicationUninstall.componentKinds': [
    'applicationBinary',
    'nativeInstaller',
    'windowsAppPackage',
    'windowsMsiPackage',
    'windowsScoopPackage',
    'windowsChocolateyPackage',
    'windowsRegisteredUninstaller',
    'cache',
    'applicationSupport',
    'preferences',
    'logs',
    'savedState',
    'sandboxContainer',
    'webData',
  ],
  'applicationUninstall.executionModes': ['silent', 'interactive', 'externalClient'],
  'applicationUninstall.componentRisks': ['required', 'rebuildable', 'userData'],
  'duplicateFiles.keeperRuleLabels': ['shortestPath', 'shortestName', 'oldestModified', 'newestModified'],
  'settings.permissionStatus': ['notChecked', 'available', 'limited'],
  'startup.filters': ['all', 'enabled', 'disabled'],
  'startup.sourceKinds': [
    'registryRun',
    'startupFolder',
    'scheduledTask',
    'service',
    'packagedStartupTask',
    'launchAgent',
    'launchDaemon',
    'loginItem',
    'backgroundTask',
    'embeddedItem',
    'advancedAutoRun',
  ],
  'startup.configuredStates': ['mixed', 'enabled', 'disabled', 'removed', 'unknown', 'notApplicable'],
  'startup.runtimeStates': ['running', 'stopped', 'loaded', 'unloaded', 'unknown'],
  'startup.trustStates': ['system', 'verified', 'invalid', 'unsigned', 'unknown'],
  'startup.scopes': ['currentUser', 'user', 'allUsers', 'machine', 'system'],
  'startup.diagnostics': [
    'accessDenied',
    'invalidData',
    'missingIdentity',
    'missingTarget',
    'stateUnavailable',
    'unsupportedFormat',
  ],
  'startup.change.descriptions': ['enabled', 'disabled', 'removed'],
  'startup.change.warnings': ['affectsOtherTriggers', 'itemCurrentlyRunning'],
  'startup.detail.startTiming': ['boot', 'userLogon', 'background', 'automatic'],
  'startup.change.skipReasons': [
    'alreadyInDesiredState',
    'catalogExpired',
    'itemChanged',
    'itemMissing',
    'stateUnknown',
    'unsupportedCapability',
    'requiresElevation',
    'targetUnavailable',
  ],
  'history.categories': [
    'deepCleanup',
    'largeFileCleanup',
    'duplicateFileCleanup',
    'applicationUninstall',
    'startupManagement',
  ],
  'history.startupStatuses': ['changed', 'unchanged', 'failed'],
  'history.startupFailureReasons': [
    'itemChanged',
    'permissionDenied',
    'userCancelled',
    'unsupported',
    'verificationFailed',
    'platformFailure',
  ],
  'history.applicationLeftoverReasons': [
    'candidateChanged',
    'ownerReappeared',
    'applicationRunning',
    'permanentDeleteFailed',
  ],
  'history.applicationLeftoverStatuses': ['previewed', 'completed', 'cancelled', 'failed'],
  'history.applicationUninstallReasons': [
    'applicationUnavailable',
    'applicationRunning',
    'processStateUnavailable',
    'catalogChanged',
    'componentUnavailable',
    'componentChanged',
    'unsupportedExecutor',
    'executionAborted',
    'permanentDeleteFailed',
    'recoveryRequired',
    'nativeInstallerFailed',
    'verificationFailed',
  ],
  'history.applicationUninstallStatuses': ['previewed', 'completed', 'failed'],
  'cleanupRules.categories': ['ai', 'system', 'browser', 'container', 'dev', 'app'],
  'cleanupRules.actionMessages': ['blocked', 'previewed', 'completed', 'partial', 'failed'],
  'cleanupRules.actionReasons': [
    'runningProcesses',
    'itemsSkipped',
    'requiredToolUnavailable',
    'preflightFailed',
    'executionFailed',
    'verificationFailed',
    'cleanerUnavailable',
  ],
};

const dynamicKeys = new Set(
  Object.entries(dynamicKeyGroups).flatMap(([prefix, suffixes]) => suffixes.map(suffix => `${prefix}.${suffix}`))
);
const frontendCorpus = collectFiles(sourceRoot, sourceExtensions)
  .filter(path => !path.includes('/locales/') && !path.endsWith('.test.ts'))
  .map(path => readFileSync(path, 'utf8'))
  .join('\n');
const literalFrontendKeys = new Set(
  [...frontendCorpus.matchAll(/(['"`])([A-Za-z0-9][A-Za-z0-9_.-]*)\1/gu)].map(match => match[2])
);
const cleanupRuleCorpus = collectFiles(coreRoot, new Set(['.rs', '.toml']))
  .filter(path => !path.includes('/tests/'))
  .map(path => readFileSync(path, 'utf8'))
  .join('\n');

const violations = [];
for (const localePath of localePaths) {
  const resource = JSON.parse(readFileSync(join(projectRoot, localePath), 'utf8'));
  for (const key of leafKeys(resource)) {
    if (literalFrontendKeys.has(key) || dynamicKeys.has(key) || cleanupRuleEntryIsUsed(key)) continue;
    violations.push(`${localePath}: unused locale key ${key}`);
  }
}

if (violations.length > 0) {
  console.error('Locale usage validation failed:');
  for (const violation of violations) console.error(`- ${violation}`);
  process.exitCode = 1;
} else {
  console.log('Locale fields are referenced by production code');
}

function cleanupRuleEntryIsUsed(key) {
  const match = /^cleanupRules\.entries\.(.+)\.(?:name|description|impact)$/u.exec(key);
  return Boolean(match && cleanupRuleCorpus.includes(match[1]));
}

function leafKeys(value, prefix = '') {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return [prefix];
  return Object.entries(value).flatMap(([key, child]) => leafKeys(child, prefix ? `${prefix}.${key}` : key));
}

function collectFiles(directory, extensions) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return collectFiles(path, extensions);
    return entry.isFile() && extensions.has(extname(entry.name)) ? [path] : [];
  });
}
