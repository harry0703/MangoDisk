import { defineStore } from 'pinia';
import { markRaw } from 'vue';
import type { AiContext, AiErrorCode, AiSettings, AiSubject, AiQuota } from '@/lib/models/ai';
import { AiService, AiSession } from '@/lib/services/ai-service';
import { LoggerService } from '@/lib/services/logger-service';
import { aiErrorCode } from '@/lib/utils/ai-error';
import { isAiServiceUnavailable } from '@/lib/utils/ai-quota';

type AiModule = AiSubject['module'];

/** Module-owned state survives page deactivation and remounting, but not app exit. */
function createWorkspace() {
  return {
    settings: null as AiSettings | null,
    loadingSettings: false,
    context: null as AiContext | null,
    language: '',
    open: false,
    minimized: false,
    text: '',
    reasoning: '',
    responseVersion: 0,
    status: 'idle' as 'idle' | 'generating' | 'completed' | 'cancelled' | 'failed',
    error: null as AiErrorCode | null,
    session: null as AiSession | null,
    pending: null as Promise<void> | null,
    cancelling: false,
    selectionVersion: 0,
  };
}

export const useAiStore = defineStore('ai', {
  state: () => ({
    workspaces: {
      cleanup: createWorkspace(),
      privacy: createWorkspace(),
      startup: createWorkspace(),
      systemOptimization: createWorkspace(),
      systemMaintenance: createWorkspace(),
    } satisfies Record<AiModule, ReturnType<typeof createWorkspace>>,
    changingConfiguration: false,
    acceptingFree: false,
    quota: null as AiQuota | null,
    quotaError: null as AiErrorCode | null,
    quotaReadAt: 0,
    quotaRevision: 0,
    quotaPending: null as Promise<void> | null,
    cache: {} as Record<string, { text: string; reasoning: string }>,
  }),
  getters: {
    open: state => Object.values(state.workspaces).some(workspace => workspace.open),
  },
  actions: {
    async refreshQuota(language: string, force = false): Promise<void> {
      const previous = this.quotaPending;
      if (previous) {
        const requiredRevision = this.quotaRevision + 1;
        await previous;
        if (!force) return;
        // Concurrent callers awaiting the same snapshot share the first newer
        // read. A request completed during that newer read still requires another.
        if (this.quotaRevision >= requiredRevision) {
          await this.quotaPending;
          return;
        }
      }
      // Panel navigation shares a short-lived snapshot. Completion forces a
      // newer read, so an earlier in-flight GET cannot restore an old balance.
      if (!force && this.quota && performance.now() - this.quotaReadAt < 15_000) return;
      const revision = ++this.quotaRevision;
      const pending = (async () => {
        try {
          const quota = await AiService.quota(language);
          if (revision !== this.quotaRevision) return;
          this.quota = quota;
          this.quotaReadAt = performance.now();
          this.quotaError = null;
        } catch (cause) {
          if (revision === this.quotaRevision) this.quotaError = aiErrorCode(cause);
        }
      })();
      this.quotaPending = markRaw(pending);
      try {
        await pending;
      } finally {
        if (this.quotaPending === pending) this.quotaPending = null;
      }
    },
    async acceptFree(module: AiModule) {
      if (this.changingConfiguration || this.acceptingFree) return;
      const workspace = this.workspaces[module];
      if (workspace.loadingSettings) return;
      this.acceptingFree = true;
      workspace.loadingSettings = true;
      try {
        const existing = await AiService.configuration();
        await AiService.save({
          endpoint: existing?.endpoint ?? '',
          model: existing?.model ?? '',
          apiKey: existing?.apiKey ?? '',
          reasoning: existing?.reasoning ?? 'default',
          temperature: existing?.temperature ?? null,
          maxTokens: existing?.maxTokens ?? null,
          mode: 'free',
          freeConsent: true,
        });
        await this.configurationChanged(null, module);
      } catch (cause) {
        workspace.error = aiErrorCode(cause);
      } finally {
        this.acceptingFree = false;
        workspace.loadingSettings = false;
      }
    },
    async configurationChanged(testError: AiErrorCode | null = null, resumeModule?: AiModule) {
      if (this.changingConfiguration) return;
      this.changingConfiguration = true;
      try {
        // A provider change invalidates all in-flight selections. Never restart
        // hidden modules automatically: each restart can incur another charge.
        const modules = Object.keys(this.workspaces) as AiModule[];
        for (const module of modules) ++this.workspaces[module].selectionVersion;
        await Promise.all(modules.map(module => this.stop(module)));
        await Promise.all(modules.map(module => this.workspaces[module].pending));
        this.cache = {};
        // Quota belongs to the installation, not the provider configuration.
        // Preserve its snapshot and pending read when saving or switching modes.
        let settings: AiSettings | null = null;
        let error = testError;
        try {
          settings = await AiService.settings();
        } catch (reason) {
          error = aiErrorCode(reason);
        }
        for (const workspace of Object.values(this.workspaces)) {
          workspace.settings = settings;
          workspace.loadingSettings = false;
        }
        if (resumeModule && error) {
          const workspace = this.workspaces[resumeModule];
          workspace.error = error;
          workspace.status = error === 'cancelled' ? 'cancelled' : 'failed';
        }
        if (error) return;
      } finally {
        this.changingConfiguration = false;
      }
      // Only the panel whose settings editor initiated the change may resume.
      if (resumeModule && this.workspaces[resumeModule].open) await this.generate(resumeModule);
    },
    async show(context: AiContext, language: string) {
      const module = context.subject.module;
      const workspace = this.workspaces[module];
      if (this.changingConfiguration) return;
      const version = ++workspace.selectionVersion;
      workspace.open = true;
      workspace.minimized = false;
      const key = JSON.stringify([language, context]);
      const previous = workspace.pending;
      if (previous) {
        // A repeated click restores the same stream. Only another item in the
        // same module replaces it; other modules have independent IPC sessions.
        if (!workspace.cancelling && key === JSON.stringify([workspace.language, workspace.context])) return;
        // Cancellation acknowledgement can precede stream teardown. A reopened
        // item must wait for that exact request, not reuse it or await a newer one.
        if (workspace.cancelling) LoggerService.info('ai', 'request_restart_waiting', { module });
        else await this.stop(module);
        await previous;
      }
      if (version !== workspace.selectionVersion || !workspace.open) return;
      workspace.context = context;
      workspace.language = language;
      workspace.error = null;
      workspace.text = this.cache[key]?.text ?? '';
      workspace.reasoning = this.cache[key]?.reasoning ?? '';
      ++workspace.responseVersion;
      workspace.status = workspace.text ? 'completed' : 'idle';
      workspace.loadingSettings = true;
      try {
        const settings = await AiService.settings();
        // Late configuration reads must not overwrite a newer selection or a
        // provider change. Page navigation deliberately does not invalidate IO.
        if (version !== workspace.selectionVersion) return;
        workspace.settings = settings;
        workspace.loadingSettings = false;
        if (settings?.mode === 'free' && settings.freeAvailable) void this.refreshQuota(language);
        if (workspace.open && settings && !workspace.text) await this.generate(module);
      } catch (error) {
        if (version === workspace.selectionVersion) {
          workspace.settings = null;
          workspace.error = aiErrorCode(error);
        }
      } finally {
        if (version === workspace.selectionVersion) workspace.loadingSettings = false;
      }
    },
    generate(module: AiModule): Promise<void> {
      const workspace = this.workspaces[module];
      if (
        !workspace.context ||
        workspace.session ||
        workspace.loadingSettings ||
        !workspace.settings ||
        this.changingConfiguration
      ) {
        return Promise.resolve();
      }
      if (
        workspace.settings.mode === 'free' &&
        (!workspace.settings.freeAvailable || !workspace.settings.freeConsent)
      ) {
        return Promise.resolve();
      }
      // A recent service-wide rejection avoids dispatching another known-invalid
      // request. Older snapshots never permanently prevent probing for recovery.
      if (
        workspace.settings.mode === 'free' &&
        isAiServiceUnavailable(this.quota) &&
        performance.now() - this.quotaReadAt < 15_000
      ) {
        workspace.status = 'failed';
        workspace.error = 'freeUnavailable';
        return Promise.resolve();
      }
      const context = workspace.context;
      const key = JSON.stringify([workspace.language, context]);
      const session = markRaw(new AiSession());
      workspace.session = session;
      workspace.cancelling = false;
      workspace.status = 'generating';
      workspace.error = null;
      workspace.text = '';
      workspace.reasoning = '';
      ++workspace.responseVersion;
      const pending = session
        .run(
          context,
          workspace.language,
          delta => {
            if (workspace.session !== session || workspace.cancelling) return;
            if (delta.kind === 'reasoning') workspace.reasoning += delta.text;
            else workspace.text += delta.text;
          },
          workspace.settings.mode
        )
        .then(() => {
          if (workspace.cancelling) {
            workspace.status = 'cancelled';
            workspace.error = 'cancelled';
            return;
          }
          workspace.status = 'completed';
          // Only complete answers enter this bounded, memory-only shared cache.
          const keys = Object.keys(this.cache);
          if (keys.length >= 20) delete this.cache[keys[0]!];
          this.cache[key] = { text: workspace.text, reasoning: workspace.reasoning };
        })
        .catch(error => {
          workspace.error = workspace.cancelling ? 'cancelled' : aiErrorCode(error);
          workspace.status = workspace.error === 'cancelled' ? 'cancelled' : 'failed';
        })
        .finally(() => {
          workspace.session = null;
          workspace.pending = null;
          workspace.cancelling = false;
          if (workspace.settings?.mode === 'free' && workspace.settings.freeAvailable)
            void this.refreshQuota(workspace.language, true);
        });
      workspace.pending = markRaw(pending);
      return pending;
    },
    dismissModule(module: AiModule) {
      // Native result changes invalidate only this module's explanation.
      if (this.workspaces[module].open) {
        LoggerService.info('ai', 'context_invalidated', { module });
        void this.close(module);
      }
    },
    minimize(module: AiModule) {
      this.workspaces[module].minimized = true;
    },
    restore(module: AiModule) {
      this.workspaces[module].minimized = false;
    },
    async stop(module: AiModule) {
      const workspace = this.workspaces[module];
      if (!workspace.session) return;
      // Set synchronously before IPC so rapid close/reopen and A/B/A clicks
      // cannot mistake a retiring request for a reusable live stream.
      workspace.cancelling = true;
      try {
        await workspace.session.cancel();
      } catch (error) {
        workspace.error = aiErrorCode(error);
        LoggerService.warn('ai', 'module_cancel_failed', { module, code: workspace.error });
      }
    },
    async close(module: AiModule) {
      const workspace = this.workspaces[module];
      ++workspace.selectionVersion;
      workspace.open = false;
      workspace.minimized = false;
      workspace.loadingSettings = false;
      await this.stop(module);
    },
  },
});
