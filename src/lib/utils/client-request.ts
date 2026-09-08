import { CLIENT_REQUEST_HEADERS, type ClientRequestMetadata } from '@/lib/models/client-request';

/** Encode collected facts without imposing the AI caller's required-field policy. */
export function clientRequestHeaders(metadata: ClientRequestMetadata): Record<string, string> {
  const headers: Record<string, string> = {
    'Accept-Language': metadata.locale,
    [CLIENT_REQUEST_HEADERS.locale]: metadata.locale,
    [CLIENT_REQUEST_HEADERS.distribution]: metadata.distribution,
  };
  if (metadata.installId) headers[CLIENT_REQUEST_HEADERS.installId] = metadata.installId;
  if (metadata.osVersion) headers[CLIENT_REQUEST_HEADERS.osVersion] = metadata.osVersion;
  if (metadata.timezone) headers[CLIENT_REQUEST_HEADERS.timezone] = metadata.timezone;
  return headers;
}
