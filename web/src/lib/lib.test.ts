import { describe, expect, it } from 'vitest';
import { parseAnsi, stripAnsi, levelOf } from './ansi';
import { bytes, duration, relTime } from './format';
import { parseHash } from './route';
import { fuzzy } from './fuzzy';
import { setProp } from './props';

describe('ansi', () => {
  it('parses colours and resets', () => {
    const s = parseAnsi('a\x1b[31mred\x1b[0m b');
    expect(s.map((x) => x.text)).toEqual(['a', 'red', ' b']);
    expect(s[1].fg).toBeDefined();
    expect(s[2].fg).toBeUndefined();
  });
  it('never emits markup and strips escapes', () => {
    expect(stripAnsi('\x1b[1;32mok\x1b[0m')).toBe('ok');
    expect(parseAnsi('<img onerror=x>')[0].text).toBe('<img onerror=x>');
  });
  it('rejects out-of-range 256 colours', () => {
    expect(parseAnsi('\x1b[38;5;999mx')[0].fg).toBeUndefined();
  });
  it('detects level', () => {
    expect(levelOf('[12:00:00 WARN]: hi')).toBe('warn');
    expect(levelOf('[12:00:00 ERROR]: hi')).toBe('error');
  });
});

describe('format', () => {
  it('bytes', () => {
    expect(bytes(0)).toBe('0 B');
    expect(bytes(1536)).toBe('1.5 KB');
    expect(bytes(null)).toBe('—');
  });
  it('duration', () => {
    expect(duration(42)).toBe('42s');
    expect(duration(3700)).toBe('1h 1m');
    expect(duration(93784)).toBe('1d 2h');
  });
  it('relTime', () => {
    const now = Date.now();
    expect(relTime(now - 1000, now)).toBe('just now');
    expect(relTime(null)).toBe('—');
  });
});

describe('router', () => {
  it('matches routes', () => {
    expect(parseHash('#/').name).toBe('overview');
    expect(parseHash('#/servers/new').name).toBe('new');
    const r = parseHash('#/servers/abc/console');
    expect(r.name).toBe('server');
    expect(r.params).toEqual({ id: 'abc', tab: 'console' });
    expect(parseHash('#/servers/abc').params.tab).toBe('overview');
    expect(parseHash('#/nope').name).toBe('notfound');
    expect(parseHash('#/activity?x=1').query.get('x')).toBe('1');
  });
});

describe('misc', () => {
  it('fuzzy', () => {
    expect(fuzzy('srv', 'Servers')).toBeGreaterThan(0);
    expect(fuzzy('zzz', 'Servers')).toBe(-1);
  });
  it('setProp preserves comments', () => {
    expect(setProp('#c\na=1\nb=2', 'a', '9')).toBe('#c\na=9\nb=2');
    expect(setProp('a=1\n', 'z', '3')).toBe('a=1\nz=3\n');
  });
});

import { buildPulse } from './pulse';
describe('pulse', () => {
  it('computes uptime', () => {
    const now = Date.now();
    const iso = (h: number) => new Date(now - h * 3600_000).toISOString();
    const p = buildPulse([{ time: iso(12), kind: 'crash', detail: '' }, { time: iso(6), kind: 'start', detail: '' }], true, 24, now);
    expect(p.uptimePct).toBeCloseTo(75, 0);
    expect(p.segs.some((s) => s.kind === 'crash')).toBe(true);
    expect(buildPulse([], false, 24, now).uptimePct).toBe(0);
  });
});

import { describeEngine, imageHint, parseExtraPorts, formatExtraPorts } from './container';
describe('extra ports input', () => {
  it('parses lists, defaults to tcp and formats back', () => {
    const r = parseExtraPorts('19132/udp, 24454/UDP 25000');
    expect(r.error).toBeNull();
    expect(formatExtraPorts(r.ports)).toBe('19132/udp, 24454/udp, 25000/tcp');
    expect(parseExtraPorts('').ports).toEqual([]);
  });
  it('explains what is wrong', () => {
    expect(parseExtraPorts('80/tcp').error).toContain('1024-65535');
    expect(parseExtraPorts('abc').error).toContain('not a port');
    expect(parseExtraPorts('2000/udp 2000/udp').error).toContain('twice');
    expect(parseExtraPorts(Array.from({ length: 9 }, (_, i) => 2000 + i).join(' ')).error).toContain('At most 8');
    expect(parseExtraPorts('1024/icmp').error).toContain('not a port');
  });
});
describe('container isolation text', () => {
  it('explains a missing engine in plain words', () => {
    const i = describeEngine({ available: false, engine: null, version: null, rootless: null, message: 'Neither Docker nor Podman was found.', outbound_restricted: false, default_image: '' });
    expect(i.tone).toBe('warn');
    expect(i.badge).toBe('No container engine');
    expect(i.detail).toContain('Neither Docker nor Podman');
  });
  it('names the engine and mode, and nudges rootful Docker users', () => {
    const ok = describeEngine({ available: true, engine: 'podman', version: '5.2.0', rootless: true, message: null, outbound_restricted: false, default_image: '' });
    expect(ok.badge).toBe('Podman 5.2.0 (rootless)');
    expect(describeEngine({ available: true, engine: 'docker', version: '27.0', rootless: false, message: null, outbound_restricted: false, default_image: '' }).detail).toContain('Rootless Podman');
  });
  it('hints at image problems without being the authority', () => {
    expect(imageHint('')).toBeNull();
    expect(imageHint('eclipse-temurin:21-jre')).toBeNull();
    expect(imageHint('ghcr.io/me/java:21')).toBeNull();
    expect(imageHint('eclipse-temurin')).toContain('tag');
    expect(imageHint('ubuntu:24.04')).toContain('allowed');
    expect(imageHint('ghcr.io/me/java:21 --privileged')).toContain('spaces');
  });
});

import { gravatarUrl, sha256Hex } from './gravatar';
describe('gravatar', () => {
  it('hashes with SHA-256', () => {
    expect(sha256Hex('')).toBe('e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
    expect(sha256Hex('abc')).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
    expect(sha256Hex('a'.repeat(1000))).toBe('41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3');
  });
  it('normalises the email and asks for the default image', () => {
    expect(gravatarUrl('  MyEmailAddress@example.com ')).toBe(gravatarUrl('myemailaddress@example.com'));
    expect(gravatarUrl('a@b.c')).toMatch(/^https:\/\/gravatar\.com\/avatar\/[0-9a-f]{64}\?s=64&d=mp$/);
  });
});
