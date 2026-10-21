import { describe, expect, it } from 'vitest';
import { keyBytes } from './push';

describe('keyBytes', () => {
  it('decodes a base64url VAPID key to 65 bytes starting with 0x04', () => {
    const b = keyBytes('BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8');
    expect(b.length).toBe(65);
    expect(b[0]).toBe(4);
  });
  it('handles url-safe characters and missing padding', () => {
    expect([...keyBytes('_-8')]).toEqual([255, 239]);
  });
});
