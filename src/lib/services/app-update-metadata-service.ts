import type { AppDistribution } from '@/lib/models/app-update';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/telemetry';
import { ClientRequestMetadataService } from '@/lib/services/client-request-metadata-service';
import { LoggerService } from '@/lib/services/logger-service';
import { normalizeError } from '@/lib/utils/error';
import { clientRequestHeaders } from '@/lib/utils/client-request';

/** Telemetry remains best-effort and must never prevent signed application updates. */
export class AppUpdateMetadataService {
  static async createHeaders(language: string, distribution: AppDistribution): Promise<Record<string, string>> {
    const metadata = await ClientRequestMetadataService.collect(language, distribution, (source, error) => {
      LoggerService.warn(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateMetadataUnavailable, {
        source,
        diagnostic: normalizeError(error),
      });
    });
    return clientRequestHeaders(metadata);
  }
}
