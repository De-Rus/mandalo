import { fetchTextForImport, readTextFileForImport } from "./api";
import { currentHost } from "./host";

/** Matches `MAX_TRANSFER_BYTES` in the Tauri edge, so both paths refuse alike. */
export const MAX_IMPORT_BYTES = 64 * 1024 * 1024;

export type DocumentOrigin = "file" | "url" | "text";

export interface LoadedDocument {
  origin: DocumentOrigin;
  name: string;
  text: string;
  bytes: number;
}

function byteLength(text: string): number {
  return new TextEncoder().encode(text).length;
}

function tooBig(name: string, bytes: number): Error {
  return new Error(
    `${name} is ${bytes} bytes, over the ${MAX_IMPORT_BYTES} byte limit for an import.`,
  );
}

export function documentFromText(text: string): LoadedDocument {
  const bytes = byteLength(text);
  if (bytes > MAX_IMPORT_BYTES) throw tooBig("This document", bytes);
  return { origin: "text", name: "Pasted document", text, bytes };
}

export async function documentFromFile(file: File): Promise<LoadedDocument> {
  if (file.size > MAX_IMPORT_BYTES) throw tooBig(file.name, file.size);
  const text = await file.text();
  return { origin: "file", name: file.name, text, bytes: byteLength(text) };
}

export async function documentFromPath(path: string): Promise<LoadedDocument> {
  const text = await readTextFileForImport(path);
  return {
    origin: "file",
    name: path.split(/[\\/]/).pop() ?? path,
    text,
    bytes: byteLength(text),
  };
}

export function importUrl(raw: string): URL {
  const trimmed = raw.trim();
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    throw new Error(`${trimmed} is not a URL Mándalo can fetch.`);
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:")
    throw new Error(
      `${trimmed} is a ${parsed.protocol.replace(":", "")} URL. An import can only be fetched over http or https.`,
    );
  return parsed;
}

/**
 * A page cannot be told why a cross-origin fetch failed — the browser withholds
 * the CORS decision from scripts on purpose — so say both things it can mean
 * rather than guessing one, exactly as the send path does.
 */
async function fetchInBrowser(url: URL): Promise<LoadedDocument> {
  let response: Response;
  try {
    response = await fetch(url.href, {
      redirect: "follow",
      credentials: "omit",
      referrerPolicy: "no-referrer",
      cache: "no-store",
    });
  } catch (e) {
    throw new Error(
      `The browser would not fetch ${url.host}. Either that server sends no CORS headers, or it was genuinely unreachable — the browser refuses to say which. Download the file and drop it on this dialog instead. Detail: ${e instanceof Error ? e.message : String(e)}`,
    );
  }
  if (!response.ok)
    throw new Error(
      `${url.href} answered ${response.status} ${response.statusText}.`,
    );
  const text = await response.text();
  const bytes = byteLength(text);
  if (bytes > MAX_IMPORT_BYTES) throw tooBig(url.href, bytes);
  return { origin: "url", name: url.href, text, bytes };
}

export async function documentFromUrl(raw: string): Promise<LoadedDocument> {
  const url = importUrl(raw);
  if (currentHost() === "browser") return fetchInBrowser(url);
  const fetched = await fetchTextForImport(url.href);
  const bytes = fetched.bytes || byteLength(fetched.text);
  if (bytes > MAX_IMPORT_BYTES) throw tooBig(url.href, bytes);
  return { origin: "url", name: fetched.url, text: fetched.text, bytes };
}
