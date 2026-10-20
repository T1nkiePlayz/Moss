import { describe, expect, it } from 'vitest';
import { cleanCode, looksLikeShareCode, mockDecode, mockEncode } from './blueprint-share';

const doc = (o: Record<string, any> = {}) => ({ format: 'moss-blueprint', version: 1, name: 'x', runtime: 'paper', game_version: '1.21.4', properties: { motd: 'hi' }, content: [{ provider: 'modrinth', project_id: 'AABBCCDD' }], schedules: [], ...o });

describe('share codes', () => {
  it('tells codes from JSON', () => {
    expect(looksLikeShareCode(' moss1_abc\n')).toBe(true);
    expect(looksLikeShareCode('{"format":"moss-blueprint"}')).toBe(false);
    expect(cleanCode(' moss1_ab\n cd ')).toBe('moss1_abcd');
  });
  it('round trips in the mock codec, even with non-ASCII names', () => {
    const d = doc({ name: 'Café ☕' });
    expect(mockDecode(mockEncode(d))).toEqual(d);
  });
  it('refuses damaged or unsafe codes with a reason', () => {
    expect(() => mockDecode('hello')).toThrow(/moss1_/);
    expect(() => mockDecode(mockEncode(doc()).slice(0, -3) + '!!')).toThrow(/not valid/);
    expect(() => mockDecode(mockEncode(doc({ content: [{ provider: 'modrinth', project_id: '../x' }] })))).toThrow(/project ids/);
    expect(() => mockDecode(mockEncode(doc({ properties: { motd: '/etc/passwd' } })))).toThrow(/paths/);
    expect(() => mockDecode(mockEncode(doc({ properties: { motd: '<script>x</script>' } })))).toThrow(/scripts/);
  });
});
