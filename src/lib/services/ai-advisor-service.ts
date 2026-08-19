import type { LargeFileEntry } from '@/lib/models/large-file';
import { LoggerService } from '@/lib/services/logger-service';

export interface AiAdvisorConfig {
  apiKey?: string;
  baseUrl?: string;
  model?: string;
}

export class AiAdvisorService {
  /**
   * Analyzes large files and recommends candidates for deletion using an LLM.
   * If a custom API key is provided, it uses that. Otherwise, it falls back to the MangoDisk proxy.
   */
  static async analyzeLargeFiles(
    files: readonly LargeFileEntry[],
    config: AiAdvisorConfig = {},
    signal?: AbortSignal
  ): Promise<string[]> {
    if (files.length === 0) return [];
    if (signal?.aborted) {
      const error = new Error('AbortError');
      error.name = 'AbortError';
      throw error;
    }

    LoggerService.info('ai-advisor', 'analyze_large_files_started', {
      fileCount: files.length,
    });

    const apiKey = config.apiKey || ''; 
    // Fallback to MangoDisk's provided proxy if no custom key is set
    const baseUrl = config.baseUrl || 'https://api.mangodisk.com/v1';
    const model = config.model || 'deepseek-chat';

    // To save tokens, we only send the filename and size, not the full absolute path
    // We map them by an index so we can resolve the full path later.
    const payloadFiles = files.map((f, i) => ({
      id: i,
      name: f.name,
      sizeMB: Math.round(f.bytes / 1024 / 1024),
    }));

    const systemPrompt = `You are an AI disk cleanup advisor. 
The user wants to clean up large files to save space on their disk.
Review the following list of files. Recommend files that are safe to delete (e.g. caches, logs, temp files, replaceable virtual machine disks, downloaded installers).
Do NOT recommend deleting personal documents, source code, photos, or movies.
Output ONLY a valid JSON object with a single key "ids" containing an array of the integer IDs you recommend deleting. No markdown, no explanations.`;

    const userPrompt = JSON.stringify(payloadFiles);

    try {
      const headers: Record<string, string> = {
        'Content-Type': 'application/json',
      };
      
      if (apiKey) {
        headers['Authorization'] = `Bearer ${apiKey}`;
      }

      // TODO(demo): Remove this block once the real proxy is deployed.
      // If using the default un-implemented endpoint, fallback to a local mock so the UI demo works!
      if (baseUrl === 'https://api.mangodisk.com/v1' && !apiKey) {
        await new Promise(r => setTimeout(r, 1500));
        if (signal?.aborted) throw new DOMException('Aborted', 'AbortError');
        const RECOMMENDED_EXTENSIONS = new Set(['.vhdx', '.iso', '.img', '.vmdk', '.ucas', '.log']);
        const mockIds = payloadFiles
          .filter(f => [...RECOMMENDED_EXTENSIONS].some(ext => f.name.toLowerCase().endsWith(ext)))
          .map(f => f.id);
        
        const mockRecommendedPaths = mockIds.map(id => files[id]?.path).filter(Boolean) as string[];
        return mockRecommendedPaths;
      }

      const response = await fetch(`${baseUrl}/chat/completions`, {
        method: 'POST',
        headers,
        body: JSON.stringify({
          model,
          messages: [
            { role: 'system', content: systemPrompt },
            { role: 'user', content: userPrompt },
          ],
          temperature: 0.1,
          response_format: { type: "json_object" } // if supported, else just rely on prompt
        }),
        signal,
      });

      if (!response.ok) {
        throw new Error(`AI API error: ${response.status} ${response.statusText}`);
      }

      const data = await response.json();
      const content = data.choices?.[0]?.message?.content || '[]';
      
      // Parse the JSON array of IDs
      let recommendedIds: number[] = [];
      try {
        const parsed = JSON.parse(content);
        if (Array.isArray(parsed)) {
          recommendedIds = parsed;
        } else if (parsed.ids && Array.isArray(parsed.ids)) {
          recommendedIds = parsed.ids;
        }
      } catch (e) {
        LoggerService.warn('ai-advisor', 'analyze_large_files_parse_error', { content });
      }

      const recommendedPaths = recommendedIds
        .map(id => files[id]?.path)
        .filter(path => Boolean(path));

      LoggerService.info('ai-advisor', 'analyze_large_files_completed', {
        fileCount: files.length,
        recommendedCount: recommendedPaths.length,
      });

      return recommendedPaths;
    } catch (error: any) {
      if (error.name === 'AbortError') {
        LoggerService.info('ai-advisor', 'analyze_large_files_aborted', {});
        throw error;
      }
      LoggerService.warn('ai-advisor', 'analyze_large_files_failed', { error: String(error) });
      return [];
    }
  }

  static async analyzeCleanupRules(
    rules: readonly any[],
    config: AiAdvisorConfig = {},
    signal?: AbortSignal
  ): Promise<string[]> {
    if (rules.length === 0) return [];
    if (signal?.aborted) {
      const error = new Error('AbortError');
      error.name = 'AbortError';
      throw error;
    }

    LoggerService.info('ai-advisor', 'analyze_cleanup_rules_started', { count: rules.length });

    const apiKey = config.apiKey || ''; 
    const baseUrl = config.baseUrl || 'https://api.mangodisk.com/v1';
    const model = config.model || 'deepseek-chat';

    const payload = rules.map((r, i) => ({ id: r.id || i, name: r.name || r.title, description: r.description }));
    const systemPrompt = "You are an AI disk cleanup advisor. Review the following cleanup rules. Recommend rules that are safe to apply (e.g. caches, temp files, trash). Output ONLY a valid JSON object with a single key 'ids' containing an array of the string IDs you recommend.";
    
    if (baseUrl === 'https://api.mangodisk.com/v1' && !apiKey) {
      await new Promise(r => setTimeout(r, 1500));
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError');
      return payload.filter(r => (r.name || '').toLowerCase().includes('cache') || (r.name || '').toLowerCase().includes('temp')).map(r => String(r.id));
    }

    try {
      const response = await fetch(`${baseUrl}/chat/completions`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...(apiKey ? { 'Authorization': `Bearer ${apiKey}` } : {}) },
        body: JSON.stringify({ model, messages: [{ role: 'system', content: systemPrompt }, { role: 'user', content: JSON.stringify(payload) }], temperature: 0.1, response_format: { type: "json_object" } }),
        signal,
      });
      const data = await response.json();
      const content = data.choices?.[0]?.message?.content || '[]';
      const parsed = JSON.parse(content);
      return Array.isArray(parsed) ? parsed.map(String) : (parsed.ids || []).map(String);
    } catch (error: any) {
      if (error.name === 'AbortError') throw error;
      LoggerService.warn('ai-advisor', 'analyze_cleanup_rules_failed', { error: String(error) });
      return [];
    }
  }

  static async analyzeDuplicateFiles(
    duplicateGroups: readonly any[],
    config: AiAdvisorConfig = {},
    signal?: AbortSignal
  ): Promise<string[]> {
    if (duplicateGroups.length === 0) return [];
    if (signal?.aborted) {
      const error = new Error('AbortError');
      error.name = 'AbortError';
      throw error;
    }

    LoggerService.info('ai-advisor', 'analyze_duplicates_started', { count: duplicateGroups.length });

    const apiKey = config.apiKey || ''; 
    const baseUrl = config.baseUrl || 'https://api.mangodisk.com/v1';
    const model = config.model || 'deepseek-chat';

    const payload = duplicateGroups.map((g, i) => ({ id: i, files: (g.files || []).map((f: any) => ({ path: f.path, modifiedTime: f.modifiedTime })) }));
    const systemPrompt = "You are an AI disk cleanup advisor. For each duplicate group, recommend which file paths are safe to DELETE. Typically, keep the oldest or the one in the most 'original' looking folder, and delete the newer copies or those in temp/download folders. Output ONLY a valid JSON object with a single key 'paths' containing an array of string paths to delete.";
    
    if (baseUrl === 'https://api.mangodisk.com/v1' && !apiKey) {
      await new Promise(r => setTimeout(r, 1500));
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError');
      const pathsToDelete: string[] = [];
      for (const g of duplicateGroups) {
        if (g.files && g.files.length > 1) {
          const sorted = [...g.files].sort((a, b) => (b.modifiedTime || 0) - (a.modifiedTime || 0));
          pathsToDelete.push(...sorted.slice(0, sorted.length - 1).map(f => f.path));
        }
      }
      return pathsToDelete;
    }

    try {
      const response = await fetch(`${baseUrl}/chat/completions`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...(apiKey ? { 'Authorization': `Bearer ${apiKey}` } : {}) },
        body: JSON.stringify({ model, messages: [{ role: 'system', content: systemPrompt }, { role: 'user', content: JSON.stringify(payload) }], temperature: 0.1, response_format: { type: "json_object" } }),
        signal,
      });
      const data = await response.json();
      const content = data.choices?.[0]?.message?.content || '[]';
      const parsed = JSON.parse(content);
      return Array.isArray(parsed) ? parsed.map(String) : (parsed.paths || []).map(String);
    } catch (error: any) {
      if (error.name === 'AbortError') throw error;
      LoggerService.warn('ai-advisor', 'analyze_duplicates_failed', { error: String(error) });
      return [];
    }
  }

  static async analyzeApplications(
    apps: readonly any[],
    config: AiAdvisorConfig = {},
    signal?: AbortSignal
  ): Promise<string[]> {
    if (apps.length === 0) return [];
    if (signal?.aborted) {
      const error = new Error('AbortError');
      error.name = 'AbortError';
      throw error;
    }

    LoggerService.info('ai-advisor', 'analyze_apps_started', { count: apps.length });

    const apiKey = config.apiKey || ''; 
    const baseUrl = config.baseUrl || 'https://api.mangodisk.com/v1';
    const model = config.model || 'deepseek-chat';

    const payload = apps.map((a, i) => ({ id: a.id || i, name: a.name, publisher: a.publisher, sizeMB: a.sizeMB }));
    const systemPrompt = "You are an AI disk cleanup advisor. Recommend applications that are likely bloatware, unwanted toolbars, or rarely used massive games. Output ONLY a valid JSON object with a single key 'ids' containing an array of string IDs you recommend uninstalling.";
    
    if (baseUrl === 'https://api.mangodisk.com/v1' && !apiKey) {
      await new Promise(r => setTimeout(r, 1500));
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError');
      return payload.filter(a => (a.name || '').toLowerCase().includes('toolbar') || (a.name || '').toLowerCase().includes('bloat')).map(a => String(a.id));
    }

    try {
      const response = await fetch(`${baseUrl}/chat/completions`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...(apiKey ? { 'Authorization': `Bearer ${apiKey}` } : {}) },
        body: JSON.stringify({ model, messages: [{ role: 'system', content: systemPrompt }, { role: 'user', content: JSON.stringify(payload) }], temperature: 0.1, response_format: { type: "json_object" } }),
        signal,
      });
      const data = await response.json();
      const content = data.choices?.[0]?.message?.content || '[]';
      const parsed = JSON.parse(content);
      return Array.isArray(parsed) ? parsed.map(String) : (parsed.ids || []).map(String);
    } catch (error: any) {
      if (error.name === 'AbortError') throw error;
      LoggerService.warn('ai-advisor', 'analyze_apps_failed', { error: String(error) });
      return [];
    }
  }

  static async analyzeStartupItems(
    startupItems: readonly any[],
    config: AiAdvisorConfig = {},
    signal?: AbortSignal
  ): Promise<string[]> {
    if (startupItems.length === 0) return [];
    if (signal?.aborted) {
      const error = new Error('AbortError');
      error.name = 'AbortError';
      throw error;
    }

    LoggerService.info('ai-advisor', 'analyze_startup_started', { count: startupItems.length });

    const apiKey = config.apiKey || ''; 
    const baseUrl = config.baseUrl || 'https://api.mangodisk.com/v1';
    const model = config.model || 'deepseek-chat';

    const payload = startupItems.map((s, i) => ({ id: s.id || i, name: s.name, command: s.command }));
    const systemPrompt = "You are an AI disk cleanup advisor. Recommend startup items that can be safely disabled to improve boot time (e.g. updater services, game launchers). Do NOT recommend disabling essential system drivers or security software. Output ONLY a valid JSON object with a single key 'ids' containing an array of string IDs you recommend disabling.";
    
    if (baseUrl === 'https://api.mangodisk.com/v1' && !apiKey) {
      await new Promise(r => setTimeout(r, 1500));
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError');
      return payload.filter(s => (s.name || '').toLowerCase().includes('updater') || (s.name || '').toLowerCase().includes('helper')).map(s => String(s.id));
    }

    try {
      const response = await fetch(`${baseUrl}/chat/completions`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...(apiKey ? { 'Authorization': `Bearer ${apiKey}` } : {}) },
        body: JSON.stringify({ model, messages: [{ role: 'system', content: systemPrompt }, { role: 'user', content: JSON.stringify(payload) }], temperature: 0.1, response_format: { type: "json_object" } }),
        signal,
      });
      const data = await response.json();
      const content = data.choices?.[0]?.message?.content || '[]';
      const parsed = JSON.parse(content);
      return Array.isArray(parsed) ? parsed.map(String) : (parsed.ids || []).map(String);
    } catch (error: any) {
      if (error.name === 'AbortError') throw error;
      LoggerService.warn('ai-advisor', 'analyze_startup_failed', { error: String(error) });
      return [];
    }
  }
}
