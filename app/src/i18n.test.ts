import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import zh from "../../crates/bonfire-app/locales/zh-CN.json";
import en from "../../crates/bonfire-app/locales/en.json";

/** 源码中用到的每个文案键在中英文中都存在；动态键列出所有可能的取值。 */
const DYNAMIC: Record<string, string[]> = {
  "ui.file.": ["added", "modified", "deleted", "removed", "unchanged"],
  "ui.library.phase.": ["discovering", "checking", "info", "idle"],
  "ui.platform.": ["steam", "epic", "gog", "custom", "ubisoft", "ea", "xbox", "battlenet"],
  "ui.state.": ["has-changes", "never-backed-up", "up-to-date", "shots-only", "pending"],
  "ui.theme.": ["auto", "light", "dark"],
  "ui.trigger.": ["manual", "auto-exit", "auto-timer", "auto-change", "pre-restore", "import"],
  "ui.transfer.action.": ["create", "same", "fast-forward", "contained", "diverged", "tag-conflict"],
  "ui.scan.phase.": ["rules", "discovering", "idle"],
  "ui.scan.filter.": ["new", "managed", "ignored", "all"],
  "ui.scan.state.": ["new", "managed", "ignored"],
  "ui.shots.source.": ["steam", "capture", "folder", "import"],
  "ui.settings.schedule.": ["off", "daily", "weekly", "hours"],
  "ui.settings.autoScheduleOpt.": ["off", "daily", "weekly", "hours"],
  "ui.settings.retentionScopeOpt.": ["auto", "all"],
  "ui.bulkDelete.reason.": ["tag", "protected", "undo"],
  "ui.settings.retention": ["LastPre", "LastSuf", "LastLabel", "DailyPre", "DailySuf", "DailyLabel", "WeeklyPre", "WeeklySuf", "WeeklyLabel"],
  "ui.weekday.": ["1", "2", "3", "4", "5", "6", "7"],
  "ui.mode.short.": ["all", "both", "saves", "shots"],
  "ui.backup.step.": ["pick", "saves", "shots"],
  "ui.diff.part.": ["synced", "new", "missing", "ignored"],
  "ui.download.step.": ["pick", "map", "finish"],
  "ui.download.mode.": ["overlay", "replace"],
  "ui.download.modeHint.": ["overlay", "replace"],
  "ui.export.kind.": ["saves", "shots", "both"],
  "ui.backup.stepHint.": ["pick", "saves", "shots"],
  "ui.mode.": ["both", "saves", "shots", "bothHint", "savesHint", "shotsHint"],
  "ui.gallery.group.": ["game", "month", "day", "source"],
  "ui.gallery.lookOpt.": ["small", "medium", "large", "list"],
};

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    if (statSync(p).isDirectory()) return sources(p);
    return /\.tsx?$/.test(f) && !/\.test\./.test(f) ? [p] : [];
  });
}

describe("文案", () => {
  const code = sources(join(__dirname)).map((f) => readFileSync(f, "utf8")).join("\n");
  const literal = [...code.matchAll(/t\(\s*"((?:ui|app)\.[A-Za-z0-9_.-]+)"/g)].map((m) => m[1]);
  const dynamic = Object.entries(DYNAMIC).flatMap(([prefix, vals]) => vals.map((v) => prefix + v));

  it("源码中用到的键都有中英文", () => {
    const missing = [...new Set([...literal, ...dynamic])].filter((k) => !(k in zh) || !(k in en));
    expect(missing).toEqual([]);
  });

  it("动态键的前缀都已登记", () => {
    const prefixes = [...code.matchAll(/t\(\s*`((?:ui|app)\.[A-Za-z0-9_.-]+\.)\$\{/g)].map((m) => m[1]);
    expect(prefixes.filter((p) => !(p in DYNAMIC))).toEqual([]);
  });

  it("中英文键集合一致，占位符一致", () => {
    expect(Object.keys(zh).sort()).toEqual(Object.keys(en).sort());
    const ph = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
    const bad = Object.keys(zh).filter((k) => JSON.stringify(ph((zh as Record<string, string>)[k])) !== JSON.stringify(ph((en as Record<string, string>)[k])));
    expect(bad).toEqual([]);
  });
});
