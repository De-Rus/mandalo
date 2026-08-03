export type Report =
  | {
      kind: "sent";
      host: string;
      url: string;
      headerCount: number;
      exposed: boolean;
    }
  | {
      kind: "unreachable";
      host: string;
      url: string;
      cors: boolean;
      preflight: boolean;
      detail: string;
    };

type Listener = (report: Report | null) => void;

const listeners = new Set<Listener>();
let current: Report | null = null;

export function report(next: Report): void {
  current = next;
  for (const listener of listeners) listener(current);
}

export function dismiss(): void {
  current = null;
  for (const listener of listeners) listener(null);
}

export function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function snapshot(): Report | null {
  return current;
}
