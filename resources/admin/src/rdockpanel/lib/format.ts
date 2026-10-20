// Numbers, sizes and times in the reader's language (docs/ui-guidelines.md §9):
// `Intl` formatters, sizes 1024-based written as Docker writes them.

const numbers = new Map<string, Intl.NumberFormat>();

function numberFormat(locale: string, digits: number): Intl.NumberFormat {
  const key = `${locale}:${digits}`;
  let format = numbers.get(key);
  if (!format) {
    format = new Intl.NumberFormat(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits });
    numbers.set(key, format);
  }
  return format;
}

/** `n` with `digits` decimals (0 by default). */
export function number(n: number, locale: string, digits = 0): string {
  return numberFormat(locale, digits).format(n);
}

/** A percentage of 0–100 with one decimal: `82.0%` (`82,0%` in Vietnamese). */
export function percent(n: number, locale: string): string {
  return `${number(n, locale, 1)}%`;
}

/** A size in bytes: `13 GB`, `5.7 MB` (1024-based, as Docker shows them). */
export function size(bytes: number, locale: string): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${number(value, locale, value < 10 && unit > 0 ? 1 : 0)} ${units[unit]}`;
}

/** A moment in full: date, time and time zone (tooltips, details). */
export function dateTime(at: string | number | Date | null | undefined, locale: string): string {
  if (at == null || at === "") return "";
  const date = new Date(at);
  if (Number.isNaN(date.getTime())) return "";
  // dateStyle/timeStyle cannot be combined with timeZoneName: the fields one by one.
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    timeZoneName: "short",
  }).format(date);
}

/** A moment relative to now: `2 hours ago`, `in 5 minutes`. */
export function relative(at: string | number | Date | null | undefined, locale: string, now = Date.now()): string {
  if (at == null || at === "") return "";
  const time = new Date(at).getTime();
  if (Number.isNaN(time)) return "";
  const seconds = Math.round((time - now) / 1000);
  const format = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const steps: Array<[Intl.RelativeTimeFormatUnit, number]> = [
    ["year", 31536000],
    ["month", 2592000],
    ["day", 86400],
    ["hour", 3600],
    ["minute", 60],
  ];
  for (const [unit, length] of steps) {
    if (Math.abs(seconds) >= length) return format.format(Math.trunc(seconds / length), unit);
  }
  return format.format(seconds, "second");
}
