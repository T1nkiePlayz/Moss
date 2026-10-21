// Helpers for blueprint share codes. The real encoding and validation live in the controller (src/blueprints/share.rs);
// the browser only decides whether pasted text looks like a code, and the mock mode uses the simple codec below.

export const CODE_PREFIX = 'moss1_';

/** True when pasted text is a share code rather than blueprint JSON. */
export function looksLikeShareCode(text: string): boolean {
  return text.trim().replace(/\s+/g, '').startsWith(CODE_PREFIX);
}

/** Removes whitespace that copy and paste adds (line breaks, spaces). */
export function cleanCode(text: string): string { return text.replace(/\s+/g, ''); }

const b64 = (s: string) => btoa(unescape(encodeURIComponent(s))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
const unb64 = (s: string) => decodeURIComponent(escape(atob(s.replace(/-/g, '+').replace(/_/g, '/'))));

/** Mock-only codec (the controller also deflates and adds a checksum). */
export function mockEncode(doc: unknown): string { return CODE_PREFIX + b64(JSON.stringify(doc)); }

/** Mock-only decoder with the same strict content rules as the controller. Throws an Error with a plain message. */
export function mockDecode(code: string, opts: { slugs?: boolean } = {}): Record<string, any> {
  const c = cleanCode(code);
  if (!c.startsWith(CODE_PREFIX)) throw new Error('This share code is not valid: it should start with moss1_');
  let d: any;
  try { d = JSON.parse(unb64(c.slice(CODE_PREFIX.length))); } catch { throw new Error('This share code is not valid: it contains characters that are not allowed, so it was probably cut off or changed'); }
  if (!d || d.format !== 'moss-blueprint' || d.version !== 1) throw new Error('This share code is not valid: it does not contain a blueprint');
  for (const i of d.content ?? []) {
    // `slugs` lets mock mode round-trip its own demo content, whose ids are slugs rather than real project ids.
    const ok = opts.slugs && /^[a-z0-9-]{2,40}$/.test(i.project_id ?? '') ? true : i.provider === 'modrinth' ? /^[A-Za-z0-9]{8}$/.test(i.project_id ?? '') : i.provider === 'curseforge' ? /^\d{1,10}$/.test(i.project_id ?? '') : i.provider === 'hangar' ? /^[A-Za-z0-9_-]{1,64}$/.test(i.project_id ?? '') : false;
    if (!ok) throw new Error('This share code is not safe to import: content items must be Modrinth, CurseForge or Hangar project ids');
  }
  if (/(^|")\s*(\/|~|[A-Za-z]:[\\/])|\.\.[\\/]|<script|javascript:/i.test(JSON.stringify(d.properties ?? {}) + JSON.stringify(d.schedules ?? []))) throw new Error('This share code is not safe to import: absolute paths and scripts are not allowed');
  return d;
}
