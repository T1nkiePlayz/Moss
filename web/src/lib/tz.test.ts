import { describe, expect, it } from 'vitest';
import { dailyWording, dstNote, filterZones, formatInZone, nextDailyRun, nextRunDisplay, observesDst, offsetMinutes, offsetName, parseFixedOffset, scheduleZone, wallClock, zoneLabel, zonedToInstant } from './tz';

const utc = (y: number, mo: number, d: number, h = 0, mi = 0) => Date.UTC(y, mo - 1, d, h, mi);

describe('time zones', () => {
  it('parses legacy fixed offsets', () => {
    expect(parseFixedOffset('UTC+02:00')).toBe(120);
    expect(parseFixedOffset('UTC-0530')).toBe(-330);
    expect(parseFixedOffset('UTC+5')).toBe(300);
    expect(parseFixedOffset('Europe/Berlin')).toBeNull();
    expect(offsetName(330)).toBe('UTC+05:30');
    expect(offsetName(0)).toBe('UTC');
  });
  it('computes offsets across DST', () => {
    expect(offsetMinutes('Europe/Berlin', utc(2026, 1, 15, 12))).toBe(60);
    expect(offsetMinutes('Europe/Berlin', utc(2026, 7, 15, 12))).toBe(120);
    expect(offsetMinutes('UTC+05:30')).toBe(330);
    expect(wallClock('Asia/Kolkata', utc(2026, 1, 1, 0, 0))).toMatchObject({ h: 5, mi: 30 });
  });
  it('converts wall-clock to an instant and keeps wall time over DST', () => {
    expect(zonedToInstant({ y: 2026, mo: 10, d: 24, h: 4, mi: 0 }, 'Europe/Berlin')).toBe(utc(2026, 10, 24, 2));
    expect(zonedToInstant({ y: 2026, mo: 10, d: 26, h: 4, mi: 0 }, 'Europe/Berlin')).toBe(utc(2026, 10, 26, 3)); // after the fall-back
  });
  it('a skipped hour resolves to the first valid instant after the gap', () => {
    // 2026-03-29 02:30 does not exist in Berlin; 03:00 local is 01:00 UTC
    const t = zonedToInstant({ y: 2026, mo: 3, d: 29, h: 2, mi: 30 }, 'Europe/Berlin');
    expect(t).toBeGreaterThanOrEqual(utc(2026, 3, 29, 1));
    expect(t).toBeLessThanOrEqual(utc(2026, 3, 29, 1, 30));
  });
  it('next daily run', () => {
    expect(nextDailyRun('04:00', 'Europe/Berlin', utc(2026, 10, 20, 1))).toBe(utc(2026, 10, 20, 2));
    expect(nextDailyRun('04:00', 'Europe/Berlin', utc(2026, 10, 20, 3))).toBe(utc(2026, 10, 21, 2));
    expect(nextDailyRun('04:00', 'UTC+05:30', utc(2026, 10, 20, 0))).toBe(utc(2026, 10, 19, 22, 30) + 86400000);
    expect(nextDailyRun('nope', 'UTC')).toBeNull();
  });
  it('formats in a zone', () => {
    expect(formatInZone(utc(2026, 7, 1, 10), 'Europe/Berlin')).toContain('12:00');
    expect(formatInZone(utc(2026, 7, 1, 10), 'UTC+05:30')).toContain('15:30');
    expect(formatInZone('garbage', 'UTC')).toBe('—');
  });
  it('reads old offset-only schedules', () => {
    expect(scheduleZone({ tz_offset_min: 120 })).toBe('UTC+02:00');
    expect(scheduleZone({ tz: 'Europe/Berlin', tz_offset_min: 60 })).toBe('Europe/Berlin');
    expect(dailyWording({ daily_at: '04:00', tz_offset_min: 120 })).toBe('Daily at 04:00 UTC+02:00 (fixed offset)');
    expect(dailyWording({ daily_at: '04:00', tz: 'America/New_York', tz_offset_min: -240 })).toBe('Daily at 04:00 America/New York');
    expect(zoneLabel('UTC')).toBe('UTC');
  });
  it('shows next run in the zone and for the viewer', () => {
    const s = { tz: 'Asia/Tokyo', tz_offset_min: 540, next_run: new Date(utc(2026, 10, 20, 19)).toISOString() };
    const r = nextRunDisplay(s, 'UTC')!;
    expect(r.inZone).toContain('04:00');
    expect(r.local).toContain('19:00');
    expect(r.same).toBe(false);
    expect(nextRunDisplay({ ...s, tz: 'UTC' }, 'UTC')!.same).toBe(true);
    expect(nextRunDisplay({ tz_offset_min: 0, next_run: null }, 'UTC')).toBeNull();
  });
  it('DST notes', () => {
    expect(observesDst('Europe/Berlin')).toBe(true);
    expect(observesDst('Asia/Tokyo')).toBe(false);
    expect(dstNote('Europe/Berlin')).toMatch(/daylight/);
    expect(dstNote('UTC')).toBeNull();
    expect(dstNote('UTC+02:00')).toMatch(/Fixed offset/);
  });
  it('searches zones', () => {
    const z = ['America/New_York', 'America/Argentina/Buenos_Aires', 'Europe/York', 'Asia/Tokyo'];
    expect(filterZones(z, 'new york')).toEqual(['America/New_York']);
    expect(filterZones(z, 'york')[0]).toBe('America/New_York');
    expect(filterZones(z, 'tok')).toEqual(['Asia/Tokyo']);
    expect(filterZones(z, '', 2)).toHaveLength(2);
    expect(filterZones(z, 'zzz')).toEqual([]);
  });
});
