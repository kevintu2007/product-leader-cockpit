/**
 * Instants as the person reads them: in the time zone the top bar names, not
 * UTC. A read time printed in UTC beside a local date looked eight hours off
 * in Asia/Taipei and said nothing about why.
 *
 * `sv-SE` formats as `YYYY-MM-DD HH:mm`, the shape every PMC surface uses.
 */
const dateTime = new Intl.DateTimeFormat("sv-SE", {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

const dateOnly = new Intl.DateTimeFormat("sv-SE", {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
});

/** A read or event time, to the minute, in the local time zone. */
export function formatReadAt(millis: number): string {
  return dateTime.format(new Date(millis));
}

/** A calendar date in the local time zone. */
export function formatLocalDate(millis: number): string {
  return dateOnly.format(new Date(millis));
}
