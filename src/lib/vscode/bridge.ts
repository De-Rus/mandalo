import {
  isEvent,
  isReply,
  PROTOCOL_VERSION,
  type CallMessage,
  type EventMessage,
} from "./protocol";

export interface Channel {
  postMessage(message: unknown): void;
}

interface Pending {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
}

export type EventListener = (event: EventMessage) => void;

/**
 * The WebView half of the request/response protocol. Every call carries a
 * correlation id because `postMessage` is a one-way pipe: replies arrive out of
 * order and the only way back to the caller is the id it was issued with.
 */
export class Bridge {
  private readonly pending = new Map<string, Pending>();
  private readonly listeners = new Set<EventListener>();
  private counter = 0;

  constructor(private readonly channel: Channel) {}

  call<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    this.counter += 1;
    const id = `c${this.counter}`;
    const message: CallMessage = { v: PROTOCOL_VERSION, type: "call", id, command, args };
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
      try {
        this.channel.postMessage(message);
      } catch (error) {
        this.pending.delete(id);
        reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  receive(raw: unknown): void {
    if (isEvent(raw)) {
      for (const listener of this.listeners) listener(raw);
      return;
    }
    if (!isReply(raw)) return;
    const pending = this.pending.get(raw.id);
    if (!pending) return;
    this.pending.delete(raw.id);
    if (raw.ok) pending.resolve(raw.value);
    else pending.reject(new Error(raw.error?.message ?? "The Mándalo extension host returned an error"));
  }

  on(listener: EventListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  get inFlight(): number {
    return this.pending.size;
  }
}

declare function acquireVsCodeApi(): Channel;

let shared: Bridge | undefined;

export function bridge(): Bridge {
  if (!shared) {
    shared = new Bridge(acquireVsCodeApi());
    window.addEventListener("message", (e) => shared?.receive(e.data));
  }
  return shared;
}
