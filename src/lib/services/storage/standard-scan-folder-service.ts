import {
  STANDARD_SCAN_FOLDER_IDS,
  type StandardScanFolder,
  type StandardScanFolderId,
} from '@/lib/models/storage/standard-scan-folder';
import { audioDir, documentDir, downloadDir, pictureDir, videoDir } from '@tauri-apps/api/path';

import * as PathUtils from '@/lib/utils/filesystem/path';

interface StandardScanFolderDefinition {
  id: StandardScanFolderId;
  resolvePath: () => Promise<string>;
}

const standardFolderDefinitions: readonly StandardScanFolderDefinition[] = [
  { id: STANDARD_SCAN_FOLDER_IDS.downloads, resolvePath: downloadDir },
  { id: STANDARD_SCAN_FOLDER_IDS.documents, resolvePath: documentDir },
  { id: STANDARD_SCAN_FOLDER_IDS.pictures, resolvePath: pictureDir },
  { id: STANDARD_SCAN_FOLDER_IDS.videos, resolvePath: videoDir },
  { id: STANDARD_SCAN_FOLDER_IDS.music, resolvePath: audioDir },
];

/** Resolves operating-system user folders without assuming localized names or home paths. */
export class StandardScanFolderService {
  static async list(): Promise<StandardScanFolder[]> {
    const resolved = await Promise.allSettled(
      standardFolderDefinitions.map(async definition => ({
        id: definition.id,
        path: PathUtils.display(await definition.resolvePath()),
      }))
    );
    const candidates = resolved.flatMap(result => (result.status === 'fulfilled' ? [result.value] : []));
    if (candidates.length === 0) return [];

    // Rendering shortcuts must not touch their targets or request privacy access.
    const seen = new Set<string>();
    return candidates.flatMap(candidate => {
      const target = candidate.path;
      const key = PathUtils.comparisonKey(target);
      if (!key || seen.has(key)) return [];
      seen.add(key);
      return [{ id: candidate.id, path: target }];
    });
  }
}
