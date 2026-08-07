export type ChangeScope = "request" | "tree" | "environments";

export interface WorkspaceChange {
  workspace: string;
  scope: ChangeScope;
  collection?: string;
  path?: string;
}

type Listener = (change: WorkspaceChange) => void;

const CHANNEL = "mandalo.workspace";

const listeners = new Set<Listener>();
let channel: BroadcastChannel | null = null;

export function syncSupported(): boolean {
  return typeof BroadcastChannel === "function";
}

/** One channel for the whole tab: a sender never receives its own message back. */
function open(): BroadcastChannel | null {
  if (!syncSupported()) return null;
  if (!channel) {
    channel = new BroadcastChannel(CHANNEL);
    channel.onmessage = (event: MessageEvent<WorkspaceChange>) => {
      for (const listener of listeners) listener(event.data);
    };
  }
  return channel;
}

export function publish(change: WorkspaceChange): void {
  open()?.postMessage(change);
}

export function subscribe(listener: Listener): () => void {
  open();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function closeSync(): void {
  channel?.close();
  channel = null;
  listeners.clear();
}
