import { AI_ERROR_CODES, type AiErrorCode } from '@/lib/models/ai';

export function aiErrorCode(error: unknown): AiErrorCode {
  return typeof error === 'string' && AI_ERROR_CODES.some(code => code === error)
    ? (error as AiErrorCode)
    : 'connectionFailed';
}
