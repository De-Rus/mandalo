/**
 * MQTT 3.1.1 over a browser WebSocket. The page has no TCP socket, so `mqtt://`
 * is out of reach here and only the WebSocket transport exists; what a broker
 * can be spoken to at all, it is spoken to properly — CONNECT/CONNACK,
 * PUBLISH/PUBACK, SUBSCRIBE/SUBACK, UNSUBSCRIBE/UNSUBACK, PINGREQ/PINGRESP.
 */

export const MQTT_SUBPROTOCOL = "mqtt";

const CONNECT = 1;
const CONNACK = 2;
const PUBLISH = 3;
const PUBACK = 4;
const SUBSCRIBE = 8;
const SUBACK = 9;
const UNSUBSCRIBE = 10;
const UNSUBACK = 11;
const PINGREQ = 12;
const PINGRESP = 13;
const DISCONNECT = 14;

const CONNACK_REASON: Record<number, string> = {
  1: "the broker refused this protocol version",
  2: "the broker rejected the client id",
  3: "the broker is unavailable",
  4: "the user name or password was wrong",
  5: "the broker refused the connection — not authorised",
};

class Writer {
  private bytes: number[] = [];

  byte(value: number): this {
    this.bytes.push(value & 0xff);
    return this;
  }

  short(value: number): this {
    return this.byte(value >> 8).byte(value);
  }

  raw(values: Uint8Array): this {
    for (const b of values) this.bytes.push(b);
    return this;
  }

  string(value: string): this {
    const encoded = new TextEncoder().encode(value);
    return this.short(encoded.length).raw(encoded);
  }

  done(): Uint8Array {
    return new Uint8Array(this.bytes);
  }
}

function varint(length: number): number[] {
  const out: number[] = [];
  let value = length;
  do {
    let byte = value % 128;
    value = Math.floor(value / 128);
    if (value > 0) byte |= 0x80;
    out.push(byte);
  } while (value > 0);
  return out;
}

function packet(type: number, flags: number, body: Uint8Array): Uint8Array {
  const header = [(type << 4) | flags, ...varint(body.length)];
  const out = new Uint8Array(header.length + body.length);
  out.set(header);
  out.set(body, header.length);
  return out;
}

export interface ConnectOptions {
  clientId: string;
  username?: string;
  password?: string;
  cleanSession: boolean;
  keepAliveSecs: number;
}

export function encodeConnect(options: ConnectOptions): Uint8Array {
  let flags = options.cleanSession ? 0x02 : 0;
  if (options.username !== undefined) flags |= 0x80;
  if (options.password !== undefined) flags |= 0x40;
  const body = new Writer()
    .string("MQTT")
    .byte(4)
    .byte(flags)
    .short(options.keepAliveSecs)
    .string(options.clientId);
  if (options.username !== undefined) body.string(options.username);
  if (options.password !== undefined) body.string(options.password);
  return packet(CONNECT, 0, body.done());
}

export function encodePublish(
  topic: string,
  payload: string,
  qos: number,
  retain: boolean,
  packetId: number,
): Uint8Array {
  const body = new Writer().string(topic);
  if (qos > 0) body.short(packetId);
  body.raw(new TextEncoder().encode(payload));
  return packet(PUBLISH, (qos << 1) | (retain ? 1 : 0), body.done());
}

export function encodeSubscribe(
  topics: { topic: string; qos: number }[],
  packetId: number,
): Uint8Array {
  const body = new Writer().short(packetId);
  for (const t of topics) body.string(t.topic).byte(t.qos);
  return packet(SUBSCRIBE, 0x02, body.done());
}

export function encodeUnsubscribe(topics: string[], packetId: number): Uint8Array {
  const body = new Writer().short(packetId);
  for (const topic of topics) body.string(topic);
  return packet(UNSUBSCRIBE, 0x02, body.done());
}

export function encodePingreq(): Uint8Array {
  return packet(PINGREQ, 0, new Uint8Array(0));
}

export function encodeDisconnect(): Uint8Array {
  return packet(DISCONNECT, 0, new Uint8Array(0));
}

export type Incoming =
  | { type: "connack"; sessionPresent: boolean; code: number }
  | { type: "publish"; topic: string; payload: Uint8Array; qos: number; retain: boolean; packetId: number | null }
  | { type: "puback"; packetId: number }
  | { type: "suback"; packetId: number; codes: number[] }
  | { type: "unsuback"; packetId: number }
  | { type: "pingresp" }
  | { type: "other"; code: number };

/** Streams whole packets out of a byte stream that arrives in arbitrary chunks. */
export class PacketReader {
  private buffer = new Uint8Array(0);

  feed(bytes: Uint8Array): Incoming[] {
    const merged = new Uint8Array(this.buffer.length + bytes.length);
    merged.set(this.buffer);
    merged.set(bytes, this.buffer.length);
    this.buffer = merged;
    const out: Incoming[] = [];
    for (;;) {
      const framed = this.take();
      if (!framed) break;
      out.push(framed);
    }
    return out;
  }

  private take(): Incoming | null {
    if (this.buffer.length < 2) return null;
    let length = 0;
    let multiplier = 1;
    let cursor = 1;
    for (;;) {
      if (cursor >= this.buffer.length) return null;
      const byte = this.buffer[cursor];
      length += (byte & 0x7f) * multiplier;
      cursor += 1;
      if ((byte & 0x80) === 0) break;
      multiplier *= 128;
      if (multiplier > 128 * 128 * 128)
        throw new Error("the broker sent a packet with a malformed length");
    }
    if (this.buffer.length < cursor + length) return null;
    const head = this.buffer[0];
    const body = this.buffer.slice(cursor, cursor + length);
    this.buffer = this.buffer.slice(cursor + length);
    return decode(head, body);
  }
}

function readString(body: Uint8Array, at: number): [string, number] {
  const length = (body[at] << 8) | body[at + 1];
  const text = new TextDecoder().decode(body.slice(at + 2, at + 2 + length));
  return [text, at + 2 + length];
}

function decode(head: number, body: Uint8Array): Incoming {
  const type = head >> 4;
  switch (type) {
    case CONNACK:
      return { type: "connack", sessionPresent: (body[0] & 1) === 1, code: body[1] };
    case PUBLISH: {
      const qos = (head >> 1) & 0x03;
      const [topic, after] = readString(body, 0);
      let at = after;
      let packetId: number | null = null;
      if (qos > 0) {
        packetId = (body[at] << 8) | body[at + 1];
        at += 2;
      }
      return {
        type: "publish",
        topic,
        payload: body.slice(at),
        qos,
        retain: (head & 1) === 1,
        packetId,
      };
    }
    case PUBACK:
      return { type: "puback", packetId: (body[0] << 8) | body[1] };
    case SUBACK:
      return {
        type: "suback",
        packetId: (body[0] << 8) | body[1],
        codes: Array.from(body.slice(2)),
      };
    case UNSUBACK:
      return { type: "unsuback", packetId: (body[0] << 8) | body[1] };
    case PINGRESP:
      return { type: "pingresp" };
    default:
      return { type: "other", code: type };
  }
}

export function encodePuback(packetId: number): Uint8Array {
  return packet(PUBACK, 0, new Writer().short(packetId).done());
}

export function connackError(code: number): string | null {
  if (code === 0) return null;
  return CONNACK_REASON[code] ?? `the broker refused the connection (code ${code})`;
}

export function randomClientId(): string {
  return `mandalo-${Math.random().toString(36).slice(2, 10)}`;
}
