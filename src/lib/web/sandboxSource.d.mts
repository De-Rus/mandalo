export type SealedGlobal = readonly [string, string];

export function readPrelude(rust: string, where?: string): string;
export function readBlocked(rust: string, where?: string): SealedGlobal[];
export function readLimits(
  rust: string,
  where?: string,
): { memoryBytes: number; timeoutMs: number };
export const EXTRA_SEALED: readonly SealedGlobal[];
