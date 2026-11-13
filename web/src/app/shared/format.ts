const numbers = new Intl.NumberFormat();
const dates = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });

/** `12,345` in the reader's locale. */
export function formatCount(value: number): string {
  return numbers.format(value);
}

/** A Unix time in seconds as a short local date and time. */
export function formatTime(unix: number): string {
  return dates.format(new Date(unix * 1000));
}

/** The base URL Last.fm clients use in place of `https://ws.audioscrobbler.com/2.0/`. */
export function apiBaseUrl(origin: string): string {
  return `${origin}/2.0/`;
}
