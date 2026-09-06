import type { AiContext } from '@/lib/models/ai';
import type { StartupArtifact } from '@/lib/models/startup';

export function startupAiContext(
  name: string,
  artifacts: readonly StartupArtifact[],
  fallback: string,
  platform: AiContext['platform']
): AiContext {
  // Preserve source metadata verbatim for attribution. Arbitrary command
  // arguments remain outside this projection's explicit metadata contract.
  const entries = artifacts.map(item => ({
    name: item.displayName || fallback,
    identity: {
      applicationName: item.ownerName ?? '',
      publisher: item.publisher ?? '',
      executableName: item.target.executableName ?? '',
      executablePath: item.target.path ?? '',
      configurationPath: item.configurationPath ?? '',
      description: item.summary ?? '',
      version: item.version ?? '',
      trust: item.trust,
    },
    sourceKind: item.sourceKind,
    triggers: [...item.triggers],
    configuredState: item.configuredState,
    runtimeState: item.runtimeState,
    controlCapability: item.controlCapability,
    diagnostics: [...item.diagnostics],
    removalSupported: item.removalSupported,
  }));
  return {
    schemaVersion: 2,
    platform,
    title: name || fallback,
    description: '',
    subject: { module: 'startup', entries, omittedCount: 0 },
  };
}
