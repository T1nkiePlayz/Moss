// TOTP helpers (pure).
export interface OtpInput { issuer: string; account: string; secret: string }
const enc = encodeURIComponent;
/** Standard otpauth:// key URI that authenticator apps understand. */
export const otpauthUri = ({ issuer, account, secret }: OtpInput) =>
  `otpauth://totp/${enc(issuer)}:${enc(account)}?secret=${enc(secret.replace(/\s+/g, ''))}&issuer=${enc(issuer)}&algorithm=SHA1&digits=6&period=30`;
/** "JBSWY3DPEHPK3PXP" -> "JBSW Y3DP EHPK 3PXP" for reading out loud / typing in. */
export const groupSecret = (s: string) => s.replace(/\s+/g, '').replace(/(.{4})/g, '$1 ').trim();
/** Digits only, at most six (users paste "123 456"). */
export const normalizeCode = (s: string) => s.replace(/\D/g, '').slice(0, 6);
export const isCode = (s: string) => /^\d{6}$/.test(s);
