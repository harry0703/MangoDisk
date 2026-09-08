import { expect, it } from 'vitest';
import { clientRequestHeaders } from './client-request';

it('encodes optional collected metadata without inventing missing fields', () => {
  const metadata = { locale: 'fr-FR', distribution: 'installed' as const };
  expect(clientRequestHeaders(metadata)).toEqual({
    'Accept-Language': 'fr-FR',
    'x-mangodisk-locale': 'fr-FR',
    'x-mangodisk-distribution': 'installed',
  });
  expect(metadata).toEqual({ locale: 'fr-FR', distribution: 'installed' });
  expect(clientRequestHeaders({ ...metadata, installId: 'fixture', osVersion: '11', timezone: 'UTC' })).toMatchObject({
    'x-mangodisk-install-id': 'fixture',
    'x-mangodisk-os-version': '11',
    'x-mangodisk-timezone': 'UTC',
  });
});
