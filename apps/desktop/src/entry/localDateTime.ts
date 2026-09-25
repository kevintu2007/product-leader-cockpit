/**
 * Dates as the person enters them, in the workspace's configured time zone
 * (DG3 record-entry amendment §3.5), converted to the UTC instant the domain
 * stores. Never the browser's incidental zone: the top bar names the
 * configured one, and a field says beside it which zone it is in.
 */

const FIELD_PATTERN = /^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2}))?$/;

/** The zone's offset from UTC, in minutes, at the given instant. */
function offsetMinutesAt(millis: number, timeZone: string): number {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).formatToParts(new Date(millis));
  const read = (type: string) => Number(parts.find((part) => part.type === type)?.value ?? "0");
  const asUtc = Date.UTC(
    read("year"),
    read("month") - 1,
    read("day"),
    read("hour"),
    read("minute"),
    read("second"),
  );
  return Math.round((asUtc - millis) / 60_000);
}

/**
 * The UTC instant for a field value entered in `timeZone`: `YYYY-MM-DD`
 * (midnight of that calendar day) or `YYYY-MM-DDTHH:mm`. `undefined` for
 * anything else — including a local time a daylight-saving transition skips
 * or repeats — so a sheet can refuse before the host has to.
 */
export function localToUtcMillis(field: string, timeZone: string): number | undefined {
  const match = FIELD_PATTERN.exec(field.trim());
  if (match === null) {
    return undefined;
  }
  const [, year, month, day, hour = "00", minute = "00"] = match;
  const naive = Date.UTC(
    Number(year),
    Number(month) - 1,
    Number(day),
    Number(hour),
    Number(minute),
  );
  if (Number.isNaN(naive)) {
    return undefined;
  }
  // A local time maps to one instant only when exactly one offset makes it
  // round-trip: a time skipped by a transition maps to none, a time repeated
  // by one maps to two. Both are refused, so a sheet says so instead of
  // silently choosing an hour.
  const withTime = match[4] !== undefined;
  const wanted = withTime ? field.trim() : `${field.trim()}T00:00`;
  const candidates = new Set<number>();
  for (const probe of [naive - 24 * 3_600_000, naive, naive + 24 * 3_600_000]) {
    const guess = naive - offsetMinutesAt(probe, timeZone) * 60_000;
    if (utcMillisToLocal(guess, timeZone, true) === wanted) {
      candidates.add(guess);
    }
  }
  if (candidates.size !== 1) {
    return undefined;
  }
  const [instant] = candidates;
  return instant;
}

/** The instant as a field value in `timeZone`: date only, or date and time. */
export function utcMillisToLocal(millis: number, timeZone: string, withTime: boolean): string {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).formatToParts(new Date(millis));
  const read = (type: string) => parts.find((part) => part.type === type)?.value ?? "00";
  const date = `${read("year")}-${read("month")}-${read("day")}`;
  return withTime ? `${date}T${read("hour")}:${read("minute")}` : date;
}
