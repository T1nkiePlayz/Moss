import createClient, { type Client, type ClientOptions } from "openapi-fetch";
import type { paths } from "./schema.js";

export type { paths, components, operations } from "./schema.js";
export type MossClient = Client<paths>;

export interface MossClientOptions extends Omit<ClientOptions, "baseUrl"> {
  /** Origin of the Moss controller, for example `http://localhost:8080`. `/api/v1` is added for you. */
  baseUrl: string;
  /** A Moss API token (`moss_pat_...`). Create one in Settings, or with `POST /tokens`. */
  token: string;
}

/** Path every route in the frozen spec lives under (the spec's `servers[0].url`). */
export const API_BASE_PATH = "/api/v1";

function apiBase(baseUrl: string): string {
  const root = baseUrl.replace(/\/+$/, "");
  return root.endsWith(API_BASE_PATH) ? root : root + API_BASE_PATH;
}

/** Creates a typed client. Every request carries `Authorization: Bearer <token>`. */
export function createMossClient(options: MossClientOptions): MossClient {
  const { baseUrl, token, headers, ...rest } = options;
  return createClient<paths>({
    ...rest,
    baseUrl: apiBase(baseUrl),
    headers: { ...(headers as Record<string, string> | undefined), Authorization: `Bearer ${token}` },
  });
}

/** One parsed Server-Sent Event. `data` is parsed JSON when possible. Loosely typed on purpose. */
export interface MossEvent {
  /** `op`, `server`, `activity`, `update` or `hello`. */
  event: string;
  data: unknown;
  id?: string;
}

export interface StreamEventsOptions {
  baseUrl: string;
  token: string;
  signal?: AbortSignal;
  fetch?: typeof fetch;
}

/**
 * Streams `GET /events`. Streams cannot send headers in the browser, so this asks for a single-use ticket
 * with `POST /ticket` (which needs a full-scope token) and passes it as `?ticket=`. Uses fetch, not EventSource.
 */
export async function* streamEvents(opts: StreamEventsOptions): AsyncGenerator<MossEvent> {
  const f = opts.fetch ?? fetch;
  const base = apiBase(opts.baseUrl);
  const t = await f(`${base}/ticket`, {
    method: "POST",
    headers: { Authorization: `Bearer ${opts.token}` },
    signal: opts.signal,
  });
  if (!t.ok) throw new Error(`ticket request failed: ${t.status}`);
  const { ticket } = (await t.json()) as { ticket: string };
  const res = await f(`${base}/events?ticket=${encodeURIComponent(ticket)}`, {
    headers: { Accept: "text/event-stream" },
    signal: opts.signal,
  });
  if (!res.ok || !res.body) throw new Error(`events request failed: ${res.status}`);

  const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
  let buf = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return;
    buf += value;
    let i: number;
    while ((i = buf.search(/\r?\n\r?\n/)) >= 0) {
      const block = buf.slice(0, i);
      buf = buf.slice(i).replace(/^\r?\n\r?\n/, "");
      const ev = parseBlock(block);
      if (ev) yield ev;
    }
  }
}

function parseBlock(block: string): MossEvent | null {
  let event = "message";
  let id: string | undefined;
  const data: string[] = [];
  for (const line of block.split(/\r?\n/)) {
    if (line.startsWith(":") || line === "") continue;
    const c = line.indexOf(":");
    const field = c < 0 ? line : line.slice(0, c);
    const val = c < 0 ? "" : line.slice(c + 1).replace(/^ /, "");
    if (field === "event") event = val;
    else if (field === "data") data.push(val);
    else if (field === "id") id = val;
  }
  if (!data.length) return null;
  const raw = data.join("\n");
  let parsed: unknown = raw;
  try {
    parsed = JSON.parse(raw);
  } catch {
    /* keep the raw text */
  }
  return { event, data: parsed, id };
}
