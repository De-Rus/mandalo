/**
 * The `text/event-stream` wire format, byte for byte the same rules the Rust
 * engine applies in `crates/core/src/stream/sse.rs`: multi-line `data:` joined
 * with newlines, `event:`, `id:`, `retry:`, `:` comments, a leading BOM stripped
 * once, and a blank line as the only thing that dispatches. Two parsers that
 * disagree would make the browser build quietly a different product.
 */
export interface SseFrame {
  event: string | null;
  data: string;
  id: string | null;
}

const DECODER = new TextDecoder("utf-8");
const ENCODER = new TextEncoder();

export class SseTooBig extends Error {
  readonly code = "E_STREAM_LIMIT";
  constructor(bytes: number, limit: number) {
    super(`the message is ${bytes} bytes, over the ${limit} byte limit for this stream`);
  }
}

export class SseParser {
  private buffer = new Uint8Array(0);
  private data = "";
  private event: string | null = null;
  private id: string | null = null;
  private lastEventId: string | null = null;
  private retry: number | null = null;
  private started = false;

  retryMs(): number | null {
    return this.retry;
  }

  lastId(): string | null {
    return this.lastEventId;
  }

  feed(bytes: Uint8Array, maxBytes: number): SseFrame[] {
    const merged = new Uint8Array(this.buffer.length + bytes.length);
    merged.set(this.buffer);
    merged.set(bytes, this.buffer.length);
    this.buffer = merged;
    if (this.buffer.length > maxBytes) throw new SseTooBig(this.buffer.length, maxBytes);
    const frames: SseFrame[] = [];
    for (;;) {
      const line = this.takeLine();
      if (line === null) break;
      const frame = this.line(line, maxBytes);
      if (frame) frames.push(frame);
    }
    return frames;
  }

  /** A trailing `\r` is held back: it may be the first half of a `\r\n`. */
  private takeLine(): string | null {
    let position = -1;
    for (let i = 0; i < this.buffer.length; i += 1)
      if (this.buffer[i] === 10 || this.buffer[i] === 13) {
        position = i;
        break;
      }
    if (position === -1) return null;
    if (this.buffer[position] === 13 && position + 1 === this.buffer.length) return null;
    const skip =
      this.buffer[position] === 13 && this.buffer[position + 1] === 10 ? 2 : 1;
    const line = DECODER.decode(this.buffer.slice(0, position));
    this.buffer = this.buffer.slice(position + skip);
    return line;
  }

  private line(raw: string, maxBytes: number): SseFrame | null {
    let line = raw;
    if (!this.started) {
      this.started = true;
      if (line.startsWith("﻿")) line = line.slice(1);
    }
    if (line === "") return this.dispatch();
    if (line.startsWith(":")) return null;
    const colon = line.indexOf(":");
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) value = value.slice(1);

    switch (field) {
      case "event":
        this.event = value;
        break;
      case "data": {
        this.data += `${value}\n`;
        const bytes = ENCODER.encode(this.data).length;
        if (bytes > maxBytes) throw new SseTooBig(bytes, maxBytes);
        break;
      }
      case "id":
        if (!value.includes("\0")) {
          this.id = value;
          this.lastEventId = value;
        }
        break;
      case "retry": {
        const ms = Number(value);
        if (/^\d+$/.test(value) && Number.isFinite(ms)) this.retry = ms;
        break;
      }
    }
    return null;
  }

  private dispatch(): SseFrame | null {
    const event = this.event;
    const id = this.id;
    this.event = null;
    this.id = null;
    if (this.data === "") return null;
    const data = this.data.slice(0, -1);
    this.data = "";
    return { event, data, id: id ?? this.lastEventId };
  }
}
