type Runner<T> = () => Promise<T>;

const chains = new Map<string, Promise<unknown>>();

export function locksSupported(): boolean {
  return (
    typeof navigator !== "undefined" &&
    typeof (navigator as Navigator & { locks?: LockManager }).locks?.request ===
      "function"
  );
}

/**
 * Without the Locks API the queue is per-tab only: it still stops this tab from
 * interleaving its own saves, but two tabs can reach the same file at once.
 */
function queued<T>(key: string, run: Runner<T>): Promise<T> {
  const previous = chains.get(key) ?? Promise.resolve();
  const next = previous.then(run, run);
  chains.set(
    key,
    next.catch(() => undefined),
  );
  void next
    .catch(() => undefined)
    .finally(() => {
      if (chains.get(key) === next) chains.delete(key);
    });
  return next;
}

export function withWriteLock<T>(workspace: string, run: Runner<T>): Promise<T> {
  const key = `mandalo.write.${workspace}`;
  if (!locksSupported()) return queued(key, run);
  return (navigator as Navigator & { locks: LockManager }).locks.request(key, () =>
    run(),
  ) as Promise<T>;
}
