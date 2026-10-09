/** 1024 进制的人类可读大小，与 Rust 端 `human_bytes` 一致。 */
export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(1)} ${units[i]}`;
}

/** 相对时间，如“3 分钟前” / “3 minutes ago”。 */
export function relative(date: Date | string | number, locale: string, now: Date = new Date()): string {
  const d = typeof date === "object" ? date : new Date(date);
  const seconds = Math.round((d.getTime() - now.getTime()) / 1000);
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const abs = Math.abs(seconds);
  if (abs < 60) return rtf.format(seconds, "second");
  if (abs < 3600) return rtf.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(seconds / 3600), "hour");
  if (abs < 86400 * 30) return rtf.format(Math.round(seconds / 86400), "day");
  if (abs < 86400 * 365) return rtf.format(Math.round(seconds / (86400 * 30)), "month");
  return rtf.format(Math.round(seconds / (86400 * 365)), "year");
}

/** 日期时间，如“2026/10/09 21:14”。 */
export function dateTime(date: Date | string, locale: string): string {
  const d = typeof date === "string" ? new Date(date) : date;
  return new Intl.DateTimeFormat(locale, { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false }).format(d);
}

/** 游玩时长（分钟）→ 小时，保留一位小数。 */
export function hours(minutes: number): string {
  return (minutes / 60).toFixed(1);
}

/** 快照 ID 的短形式（去掉 b3: 前缀，取前 n 位）。 */
export function shortId(id: string, n = 8): string {
  return id.replace(/^b3:/, "").slice(0, n);
}

/** 清单路径去掉位置序号前缀，便于阅读：`0/slot1.sav` → `slot1.sav`。 */
export function displayPath(path: string): string {
  const i = path.indexOf("/");
  return i >= 0 ? path.slice(i + 1) : path;
}

/** 备注第一行作为摘要（文档 8.3）。 */
export function firstLine(s: string): string {
  return s.split(/\r?\n/)[0] ?? "";
}
