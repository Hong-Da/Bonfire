import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { CleanupDialog } from "../components/CleanupDialog";
import { displayName } from "../components/ui";
import { confirm } from "../lib/confirm";
import { bytes, relative } from "../lib/format";
import { prefGet, prefSet } from "../lib/uiState";
import { SEEN_KEY } from "../components/Tour";
import { useStore } from "../store";
import type { Activity, ActivityPage, AttentionItem, DiskCompare, GameRow, StorageStats, TargetStatus } from "../types";
import BackupDialog from "./BackupDialog";

/**
 * 总览（0.3.0，设计稿 docs/design/overview-2026-10-11）：
 * 存档的三道防线（磁盘 → 篝火仓库 → 远程）→ 待处理 → 存储 / 截图与磁盘 → 操作记录。
 */

/** 截图与磁盘的对比结果在 10 分钟内不重复计算（决策 OV-5）。 */
const COMPARE_TTL = 10 * 60 * 1000;
let lastCompare: { at: number; data: DiskCompare } | null = null;
/** 可清理空间达到这个大小时提示。 */
const RECLAIM_MIN = 100 * 1024 * 1024;

type Level = "bad" | "warn" | "info";
interface Todo {
  key: string;
  level: Level;
  title: string;
  detail?: string;
  /** 稍后提醒的依据：变化后重新出现。 */
  print: string;
  /** 后台的“需要处理”（失败）；稍后提醒记在后台。 */
  attention?: AttentionItem;
  actions: { label: string; primary?: boolean; run: () => void }[];
}

const savesManaged = (r: GameRow) => r.state !== "shots-only";
const needsBackup = (r: GameRow) => r.state === "has-changes" || r.state === "never-backed-up";
const targetOk = (s: TargetStatus) => !!s.last && !s.last.error && (s.target.kind === "folder" || s.hasSecret);
const covers = (s: TargetStatus, id: string) => s.target.enabled && s.target.content !== "shots" && (s.target.games.length === 0 || s.target.games.includes(id));

export default function OverviewPage() {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const lang = i18n.language;
  const rows = useMemo(() => store.order.map((id) => store.rows[id]).filter(Boolean), [store.order, store.rows]);
  const saveRows = rows.filter(savesManaged);
  const [targets, setTargets] = useState<TargetStatus[]>([]);
  const [stats, setStats] = useState<StorageStats | null | "error">(null);
  const [reclaim, setReclaim] = useState<number | null>(null);
  const [disk, setDisk] = useState<DiskCompare | null>(lastCompare?.data ?? null);
  const [comparing, setComparing] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [backupRows, setBackupRows] = useState<GameRow[] | null>(null);
  const [cleanOpen, setCleanOpen] = useState(false);
  const [later, setLater] = useState(0);
  const attention = store.attention?.items ?? [];
  const [tourTip, setTourTip] = useState(() => !prefGet(SEEN_KEY) && !prefGet("tour.tipClosed"));

  const refreshAll = () => {
    void api.syncStatus().then(setTargets).catch(() => setTargets([]));
    void api.attention().then(store.setAttention).catch(() => {});
    setStats(null);
    void api
      .storageStats()
      .then(setStats)
      .catch(() => setStats("error"));
    void api
      .cleanHistory(true)
      .then((r) => setReclaim(r.gc.bytes))
      .catch(() => setReclaim(null));
  };

  const compare = async (force: boolean) => {
    if (!force && lastCompare && Date.now() - lastCompare.at < COMPARE_TTL) {
      setDisk(lastCompare.data);
      return;
    }
    setComparing(true);
    try {
      const d = await api.compareDisk();
      lastCompare = { at: Date.now(), data: d };
      setDisk(d);
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setComparing(false);
    }
  };

  useEffect(() => {
    refreshAll();
    void compare(false);
    const off = api.onSyncDone(() => void api.syncStatus().then(setTargets));
    return () => void off.then((f) => f());
  }, []);

  // ---------- 三道防线 ----------
  const okTargets = targets.filter((s) => s.target.enabled && targetOk(s));
  const enabledTargets = targets.filter((s) => s.target.enabled);
  const unbacked = saveRows.filter(needsBackup);
  const backupFailed = attention.filter((a) => a.kind === "backup");
  const syncFailed = attention.filter((a) => a.kind === "sync");
  const uncovered = enabledTargets.length === 0 ? [] : saveRows.filter((r) => r.snapshots > 0 && !okTargets.some((s) => covers(s, r.id)));
  const issueIds = new Set([...unbacked.map((r) => r.id), ...backupFailed.map((a) => a.game ?? ""), ...uncovered.map((r) => r.id)]);
  const issues = saveRows.filter((r) => issueIds.has(r.id)).length;
  const snapshots = saveRows.reduce((n, r) => n + r.snapshots, 0);
  const lastBackup = saveRows.map((r) => r.lastBackupAt).filter((x): x is string => !!x).sort().pop() ?? null;

  const verdict =
    saveRows.length === 0
      ? t("ui.ov.empty")
      : issues === 0
        ? enabledTargets.length > 0
          ? t("ui.ov.allSafe", { n: saveRows.length })
          : t("ui.ov.localSafe", { n: saveRows.length })
        : t("ui.ov.someIssues", { ok: saveRows.length - issues, n: issues });

  const wire1: { cls: string; label: string } =
    backupFailed.length > 0
      ? { cls: "cut", label: t("ui.ov.wireBackupFailed", { n: backupFailed.length }) }
      : unbacked.length > 0
        ? { cls: "gap", label: t("ui.ov.wireUnbacked", { n: unbacked.length }) }
        : { cls: "", label: t("ui.ov.wireBacked") };
  const wire2: { cls: string; label: string } =
    enabledTargets.length === 0
      ? { cls: "gap", label: t("ui.ov.wireNoRemote") }
      : syncFailed.length > 0
        ? { cls: "cut", label: t("ui.ov.wireSyncFailed", { n: syncFailed.length }) }
        : uncovered.length > 0
          ? { cls: "gap", label: t("ui.ov.wireUncovered", { n: uncovered.length }) }
          : { cls: "", label: t("ui.ov.wireSynced") };

  // ---------- 待处理 ----------
  const names = (rs: { name: string }[]) => rs.slice(0, 4).map((r) => r.name).join(t("ui.ov.sep")) + (rs.length > 4 ? t("ui.ov.andMore", { n: rs.length }) : "");
  const rowName = (r: GameRow) => displayName(r, lang);
  const issueText = (k: string | null) => (k ? t(`ui.ov.issue.${k}`, { defaultValue: t("ui.ov.issue.other") }) : t("ui.ov.issue.other"));
  const todos: Todo[] = [];
  for (const a of attention) {
    const retry = a.retry ? t("ui.sync.retryAt", { time: new Date(a.retry.at).toLocaleTimeString(lang, { hour: "2-digit", minute: "2-digit" }), n: a.retry.attempt }) : "";
    todos.push({
      key: a.key,
      level: "bad",
      print: a.at,
      attention: a,
      title:
        a.kind === "sync"
          ? t("ui.ov.todo.syncFailed", { name: a.name, why: issueText(a.issue) })
          : a.kind === "fsck"
            ? t("ui.ov.todo.fsckFailed")
            : t("ui.ov.todo.backupFailed", { name: a.name, why: issueText(a.issue) }),
      detail: [t(`ui.ov.hint.${a.issue ?? "other"}`, { defaultValue: "" }), retry, relative(a.at, lang)].filter(Boolean).join(" "),
      actions:
        a.kind === "sync"
          ? [{ label: t("ui.ov.goSync"), run: () => store.go({ name: "sync" }) }]
          : a.kind === "fsck"
            ? [{ label: t("ui.ov.goSettings"), run: () => store.go({ name: "settings" }) }]
            : a.issue === "locked"
              ? [
                  { label: t("ui.ov.openSettings"), run: () => store.go({ name: "settings", section: "auto" }) },
                  ...(a.game ? [{ label: t("ui.ov.viewGame"), run: () => store.go({ name: "game", id: a.game! }) }] : []),
                ]
              : a.game
                ? [{ label: t("ui.ov.viewGame"), run: () => store.go({ name: "game", id: a.game! }) }]
                : [],
    });
  }
  if (unbacked.length > 0)
    todos.push({
      key: "unbacked",
      level: "warn",
      print: unbacked.map((r) => r.id).sort().join(","),
      title: t("ui.ov.todo.unbacked", { n: unbacked.length }),
      detail: names(unbacked.map((r) => ({ name: rowName(r) }))),
      actions: [{ label: t("ui.ov.backupAll"), primary: true, run: () => setBackupRows(unbacked) }],
    });
  const unconfirmed = rows.filter((r) => r.unconfirmed > 0);
  if (unconfirmed.length > 0)
    todos.push({
      key: "unconfirmed",
      level: "warn",
      print: unconfirmed.map((r) => `${r.id}:${r.unconfirmed}`).join(","),
      title: t("ui.ov.todo.unconfirmed", { n: unconfirmed.length }),
      detail: names(unconfirmed.map((r) => ({ name: rowName(r) }))),
      actions: unconfirmed.length === 1 ? [{ label: t("ui.ov.viewGame"), run: () => store.go({ name: "game", id: unconfirmed[0].id }) }] : [{ label: t("ui.ov.goLibrary"), run: () => store.go({ name: "library" }) }],
    });
  if (uncovered.length > 0)
    todos.push({
      key: "uncovered",
      level: "warn",
      print: uncovered.map((r) => r.id).sort().join(","),
      title: t("ui.ov.todo.uncovered", { n: uncovered.length }),
      detail: names(uncovered.map((r) => ({ name: rowName(r) }))),
      actions: [{ label: t("ui.ov.goSync"), run: () => store.go({ name: "sync" }) }],
    });
  if (enabledTargets.length === 0 && saveRows.some((r) => r.snapshots > 0))
    todos.push({
      key: "no-remote",
      level: "info",
      print: "1",
      title: t("ui.ov.todo.noRemote"),
      detail: t("ui.ov.todo.noRemoteHint"),
      actions: [{ label: t("ui.ov.goSync"), run: () => store.go({ name: "sync" }) }],
    });
  if (reclaim !== null && reclaim >= RECLAIM_MIN)
    todos.push({
      key: "reclaim",
      level: "info",
      print: String(Math.round(reclaim / (50 * 1024 * 1024))),
      title: t("ui.ov.todo.reclaim", { size: bytes(reclaim) }),
      detail: t("ui.ov.todo.reclaimHint"),
      actions: [{ label: t("ui.ov.previewClean"), run: () => setCleanOpen(true) }],
    });
  void later; // 稍后提醒后重新计算
  const shown = todos.filter((x) => x.attention || prefGet(`overview.later.${x.key}`) !== x.print);
  const hidden = todos.length - shown.length + (store.attention?.snoozed ?? 0);
  const snooze = async (x: Todo) => {
    if (x.attention) {
      await api.snoozeAttention(x.attention.key, x.attention.at).catch((e) => store.toast("error", errorMessage(e)));
      void api.attention().then(store.setAttention);
    } else {
      prefSet(`overview.later.${x.key}`, x.print);
      setLater((n) => n + 1);
    }
  };
  const badCount = shown.filter((x) => x.level === "bad").length;

  // ---------- 截图与磁盘 ----------
  const sum = (k: "new" | "missing" | "duplicates" | "ignored" | "toHome" | "noPlace" | "restorable") => (disk?.games ?? []).reduce((n, g) => n + g[k], 0);
  const collectAll = async () => {
    setBusy("collect");
    try {
      const n = await api.collectAllShots();
      store.toast("success", t("ui.ov.collected", { n }));
      void compare(true);
      refreshAll();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const writeBackAll = async () => {
    const n = sum("missing") - sum("noPlace");
    if (!(await confirm(t("ui.ov.writeBackConfirm", { n }), { title: t("ui.ov.writeBackAll"), ok: t("ui.ov.writeBackAll") }))) return;
    setBusy("write");
    try {
      const r = await api.writeBackAll();
      store.toast("success", t("ui.diff.written", { n: r.written, size: bytes(r.bytes) }) + (r.exists + r.noDir > 0 ? t("ui.diff.writtenSkipped", { exists: r.exists, nodir: r.noDir }) : ""));
      void compare(true);
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="page overview">
      <div className="toolbar">
        <h1>{t("ui.ov.title")}</h1>
        <div className="grow" />
        <button
          className="btn"
          onClick={() => {
            refreshAll();
            void compare(true);
            void store.loadLibrary(false);
          }}
        >
          {t("ui.ov.refresh")}
        </button>
      </div>

      {tourTip && (
        <div className="callout ov-tour-tip">
          <span>{t("ui.ov.tourTip")}</span>
          <span className="grow" />
          <button className="btn small primary" onClick={() => store.setGuide({ tour: true })}>
            {t("ui.settings.openTour")}
          </button>
          <button
            className="link"
            onClick={() => {
              prefSet("tour.tipClosed", "1");
              setTourTip(false);
            }}
          >
            {t("ui.ov.tourTipClose")}
          </button>
        </div>
      )}

      <section className="ov-chain" aria-label={t("ui.ov.chain")}>
        <p className="ov-verdict">{verdict}</p>
        <div className="ov-links">
          <div className="ov-stop">
            <small>{t("ui.ov.disk")}</small>
            <b className="num">
              {saveRows.length}
              <em>{t("ui.ov.games")}</em>
            </b>
            <span className={`ov-state ${unbacked.length > 0 ? "warn" : "ok"}`}>{unbacked.length > 0 ? t("ui.ov.diskUnbacked", { n: unbacked.length }) : t("ui.ov.diskOk")}</span>
          </div>
          <div className={`ov-wire ${wire1.cls}`} aria-hidden>
            <span>{wire1.label}</span>
          </div>
          <div className="ov-stop">
            <small>{t("ui.ov.repo")}</small>
            <b className="num">
              {snapshots.toLocaleString(lang)}
              <em>{t("ui.ov.snapshots")}</em>
            </b>
            <span className={`ov-state ${backupFailed.length > 0 ? "bad" : "ok"}`}>
              {backupFailed.length > 0 ? t("ui.ov.repoFailed", { n: backupFailed.length }) : lastBackup ? t("ui.ov.lastBackup", { when: relative(lastBackup, lang) }) : t("ui.ov.noBackup")}
            </span>
          </div>
          <div className={`ov-wire ${wire2.cls}`} aria-hidden>
            <span>{wire2.label}</span>
          </div>
          <div className="ov-stop">
            <small>{t("ui.ov.remote")}</small>
            <b className="num">
              {enabledTargets.length}
              <em>{t("ui.ov.targets")}</em>
            </b>
            <span className={`ov-state ${syncFailed.length > 0 ? "bad" : uncovered.length > 0 || enabledTargets.length === 0 ? "warn" : "ok"}`}>
              {enabledTargets.length === 0
                ? t("ui.ov.remoteNone")
                : syncFailed.length > 0
                  ? t("ui.ov.remoteFailed", { name: syncFailed[0].name, n: syncFailed.length })
                  : uncovered.length > 0
                    ? t("ui.ov.remoteUncovered", { n: uncovered.length })
                    : t("ui.ov.remoteOk")}
            </span>
          </div>
        </div>
      </section>

      <section className="ov-block" aria-label={t("ui.ov.todoTitle")}>
        <h2>
          {t("ui.ov.todoTitle")}
          {shown.length > 0 && <small>{t("ui.ov.todoCount", { bad: badCount, other: shown.length - badCount })}</small>}
        </h2>
        {shown.length === 0 ? (
          <p className="ov-clear">{t("ui.ov.todoNone")}</p>
        ) : (
          <ul className="ov-todo">
            {shown.map((x) => (
              <li key={x.key} className={x.level}>
                <span className="rail-mark" aria-hidden />
                <div className="what">
                  <b>{x.title}</b>
                  {x.detail && <p>{x.detail}</p>}
                </div>
                <div className="acts">
                  {x.actions.map((a) => (
                    <button key={a.label} className={`btn small${a.primary ? " primary" : ""}`} onClick={a.run}>
                      {a.label}
                    </button>
                  ))}
                  <button className="link later" onClick={() => void snooze(x)}>
                    {t("ui.ov.later")}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
        {hidden > 0 && <p className="ov-foot">{t("ui.ov.hiddenTodos", { n: hidden })}</p>}
      </section>

      <section className="ov-panel" aria-label={t("ui.ov.diskTitle")}>
          <div className="ov-panel-head">
            <h2>{t("ui.ov.diskTitle")}</h2>
            <span className="muted ov-small">{comparing ? t("ui.ov.comparing") : disk ? t("ui.ov.compared", { n: disk.compared, when: relative(disk.at, lang) }) : ""}</span>
            <span className="grow" />
            <span className="muted ov-small">{t("ui.ov.noOverwrite")}</span>
            <button className="btn small" disabled={comparing} onClick={() => void compare(true)}>
              {t("ui.ov.compareAgain")}
            </button>
          </div>
          {disk && (
            <div className="ov-disk">
              <div className="row">
                <div>
                  <b className="num">{sum("new")}</b>
                  {t("ui.ov.diskNew")}
                  <p>{sum("new") > 0 ? t("ui.ov.diskNewHint", { games: disk.games.filter((g) => g.new > 0).length, dup: sum("duplicates") }) : t("ui.ov.diskNewNone")}</p>
                </div>
                <button className="btn small primary" disabled={sum("new") === 0 || !!busy} onClick={() => void collectAll()}>
                  {t("ui.ov.collectAll")}
                </button>
              </div>
              <div className="row">
                <div>
                  <b className="num">{sum("missing")}</b>
                  {t("ui.ov.diskMissing")}
                  <p>
                    {sum("missing") > 0
                      ? [
                          sum("restorable") > 0 && t("ui.ov.diskBack", { n: sum("restorable") }),
                          sum("toHome") > 0 && t("ui.ov.diskHome", { n: sum("toHome") }),
                          sum("noPlace") > 0 && t("ui.ov.diskNoPlace", { n: sum("noPlace") }),
                        ]
                          .filter(Boolean)
                          .join(t("ui.ov.sep"))
                      : t("ui.ov.diskMissingNone")}
                  </p>
                  {sum("noPlace") > 0 && (
                    <p className="ov-links-inline">
                      {disk.games
                        .filter((g) => g.noPlace > 0)
                        .map((g) => (
                          <button key={g.id} className="link" onClick={() => store.go({ name: "game", id: g.id })}>
                            {g.name}
                          </button>
                        ))}
                    </p>
                  )}
                </div>
                <button className="btn small" disabled={sum("missing") - sum("noPlace") === 0 || !!busy} onClick={() => void writeBackAll()}>
                  {t("ui.ov.writeBackAll")}
                </button>
              </div>
              <div className="row saves">
                <div>
                  <b className="num">{unbacked.length}</b>
                  {t("ui.ov.diskSaves")}
                  <p>{t("ui.ov.diskSavesHint")}</p>
                </div>
                <button className="btn small" disabled={unbacked.length === 0} onClick={() => setBackupRows(unbacked)}>
                  {t("ui.ov.backupAll")}
                </button>
              </div>
            </div>
          )}
      </section>

      <StoragePanel stats={stats} reclaim={reclaim} targets={targets} onClean={() => setCleanOpen(true)} />

      <ActivityLog rows={rows} />

      {backupRows && (
        <BackupDialog
          rows={backupRows}
          onClose={() => {
            setBackupRows(null);
            void api.attention().then(store.setAttention);
          }}
        />
      )}
      {cleanOpen && (
        <CleanupDialog
          onClose={() => {
            setCleanOpen(false);
            refreshAll();
          }}
        />
      )}
    </div>
  );
}

function StoragePanel({ stats, reclaim, targets, onClean }: { stats: StorageStats | null | "error"; reclaim: number | null; targets: TargetStatus[]; onClean: () => void }) {
  const { t, i18n } = useTranslation();
  if (stats === null || stats === "error")
    return (
      <section className="ov-panel" aria-label={t("ui.ov.storage")}>
        <h2>{t("ui.ov.storage")}</h2>
        <p className="muted">{stats === "error" ? t("ui.ov.statsFailed") : t("ui.ov.counting")}</p>
      </section>
    );
  const pct = (n: number) => `${stats.total > 0 ? (n / stats.total) * 100 : 0}%`;
  const trend = stats.trend;
  const growth = trend.length >= 2 ? trend[trend.length - 1][1] - (trend.find(([d]) => Date.parse(d) >= Date.now() - 30 * 864e5) ?? trend[0])[1] : null;
  const top = stats.games.slice(0, 5);
  const rest = stats.games.slice(5);
  const max = Math.max(1, ...stats.games.map((g) => g.saves + g.shots));
  const spark = (() => {
    if (trend.length < 2) return null;
    const vals = trend.map(([, v]) => v);
    const lo = Math.min(...vals);
    const hi = Math.max(...vals);
    return vals.map((v, i) => `${(i / (vals.length - 1)) * 120},${28 - ((v - lo) / Math.max(1, hi - lo)) * 24}`).join(" ");
  })();
  const remotes = targets.filter((s) => s.target.enabled);
  return (
    <section className="ov-panel" aria-label={t("ui.ov.storage")}>
      <h2>{t("ui.ov.storage")}</h2>
      <div className="ov-storage-grid">
      <div className="ov-storage-main">
      <div className="ov-bigline">
        <b className="num">{bytes(stats.total)}</b>
        <span className="muted">{stats.free !== null ? t("ui.ov.repoAtFree", { dir: stats.repoDir, free: bytes(stats.free) }) : t("ui.ov.repoAt", { dir: stats.repoDir })}</span>
      </div>
      <div className="ov-stack" role="img" aria-label={t("ui.ov.stackLabel", { saves: bytes(stats.saves), shots: bytes(stats.shots), other: bytes(stats.other) })}>
        <i className="seg-saves" style={{ width: pct(stats.saves) }} />
        <i className="seg-shots" style={{ width: pct(stats.shots) }} />
        <i className="seg-other" style={{ width: pct(stats.other) }} />
      </div>
      <div className="ov-legend">
        <span className="seg-saves">{t("ui.ov.partSaves", { size: bytes(stats.saves) })}</span>
        <span className="seg-shots">{t("ui.ov.partShots", { size: bytes(stats.shots) })}</span>
        <span className="seg-other">{t("ui.ov.partOther", { size: bytes(stats.other) })}</span>
      </div>
      <div className="ov-facts">
        <div>
          <small>{t("ui.ov.saved")}</small>
          <b className="num">{bytes(Math.max(0, stats.logical - stats.saves))}</b>
          <span className="muted">{t("ui.ov.logical", { size: bytes(stats.logical), n: stats.snapshots.toLocaleString(i18n.language) })}</span>
        </div>
        <div>
          <small>{t("ui.ov.reclaimable")}</small>
          <b className="num">{reclaim === null ? "…" : bytes(reclaim)}</b>
          {reclaim !== null && reclaim > 0 && (
            <button className="link" onClick={onClean}>
              {t("ui.ov.previewClean")}
            </button>
          )}
        </div>
        <div>
          <small>{t("ui.ov.growth")}</small>
          <b className="num">{growth === null ? "—" : `${growth >= 0 ? "+" : "−"}${bytes(Math.abs(growth))}`}</b>
          {spark ? (
            <svg className="ov-spark" viewBox="0 0 120 30" preserveAspectRatio="none" aria-hidden>
              <polyline points={spark} />
            </svg>
          ) : (
            <span className="muted">{t("ui.ov.growthLater")}</span>
          )}
        </div>
      </div>
      </div>
      <div className="ov-storage-side">
      {top.length > 0 && (
        <>
          <p className="ov-subh">{t("ui.ov.topGames")}</p>
          <ul className="ov-rank">
            {top.map((g) => (
              <li key={g.id} title={t("ui.ov.gameUsage", { saves: bytes(g.saves), shots: bytes(g.shots), n: g.snapshots })}>
                <span className="name">{g.name}</span>
                <span className="bar">
                  <i className="seg-saves" style={{ width: `${(g.saves / max) * 100}%` }} />
                  <i className="seg-shots" style={{ width: `${(g.shots / max) * 100}%` }} />
                </span>
                <span className="size num">{bytes(g.saves + g.shots)}</span>
              </li>
            ))}
            {rest.length > 0 && (
              <li>
                <span className="name muted">{t("ui.ov.restGames", { n: rest.length })}</span>
                <span className="bar">
                  <i className="seg-other" style={{ width: `${(rest.reduce((n, g) => n + g.saves + g.shots, 0) / Math.max(max, rest.reduce((n, g) => n + g.saves + g.shots, 0))) * 100}%` }} />
                </span>
                <span className="size num">{bytes(rest.reduce((n, g) => n + g.saves + g.shots, 0))}</span>
              </li>
            )}
          </ul>
        </>
      )}
      {remotes.length > 0 && (
        <>
          <p className="ov-subh">{t("ui.ov.remote")}</p>
          <ul className="ov-remotes">
            {remotes.map((s) => (
              <li key={s.target.id}>
                <span className="name">{s.target.name}</span>
                <span className={`ov-remote-state ${s.last?.error ? "bad" : s.last ? "ok" : ""}`}>
                  {s.last?.error ? t("ui.ov.remoteStateFailed") : s.last ? t("ui.ov.remoteStateOk", { when: relative(s.last.at, i18n.language), n: s.remoteFiles }) : t("ui.sync.never")}
                </span>
                <span className="muted num">{s.target.max_total_mb > 0 ? t("ui.ov.remoteLimit", { mb: s.target.max_total_mb }) : ""}</span>
              </li>
            ))}
          </ul>
        </>
      )}
      </div>
      </div>
    </section>
  );
}

const CATEGORIES = ["all", "failed", "backup", "restore", "history", "sync", "shots", "transfer", "maintain"] as const;
type Category = (typeof CATEGORIES)[number];
const UNIT: Record<string, string> = {
  backup: "files",
  "backup-auto": "files",
  restore: "files",
  "undo-restore": "files",
  "export-files": "files",
  "import-download": "files",
  "delete-snapshots": "snapshots",
  "branch-delete": "snapshots",
  "export-bundle": "snapshots",
  "import-bundle": "snapshots",
  clean: "snapshots",
  reindex: "snapshots",
  sync: "files",
  "shots-collect": "shots",
  "shots-write-back": "shots",
  "shots-delete": "shots",
  "shots-purge": "shots",
  "export-shots": "shots",
  "export-package": "games",
  scan: "games",
  "rules-update": "games",
  fsck: "problems",
};

function ActivityLog({ rows }: { rows: GameRow[] }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const lang = i18n.language;
  const [cat, setCat] = useState<Category>("all");
  const [game, setGame] = useState("");
  const [text, setText] = useState("");
  /** 默认只看最近一周。 */
  const [days, setDays] = useState(7);
  const [page, setPage] = useState<ActivityPage | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  const query = (before: string | null) => api.activityList({ category: cat === "all" || cat === "failed" ? null : cat, failed: cat === "failed", game: game || null, text: text.trim() || null, limit: 60, before, days });
  useEffect(() => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(
      () =>
        void query(null)
          .then(setPage)
          .catch((e) => store.toast("error", errorMessage(e))),
      text ? 250 : 0,
    );
    return () => window.clearTimeout(timer.current);
  }, [cat, game, text, days]);

  const resolve = async (a: Activity) => {
    try {
      await api.resolveActivity(`${a.at}|${a.action}|${a.name ?? ""}`);
      setPage(await query(null));
      void api.attention().then(store.setAttention);
    } catch (e) {
      store.toast("error", errorMessage(e));
    }
  };

  const more = async () => {
    if (!page || page.items.length === 0) return;
    setLoadingMore(true);
    try {
      const next = await query(page.items[page.items.length - 1].at);
      setPage({ ...next, items: [...page.items, ...next.items] });
    } finally {
      setLoadingMore(false);
    }
  };

  const day = (at: string) => {
    const d = new Date(at);
    const today = new Date();
    const y = new Date(today.getTime() - 864e5);
    const same = (a: Date, b: Date) => a.toDateString() === b.toDateString();
    return same(d, today) ? t("ui.ov.today") : same(d, y) ? t("ui.ov.yesterday") : d.toLocaleDateString(lang, { month: "long", day: "numeric", weekday: "short" });
  };
  const nameOf = (a: Activity) => (a.name ? a.name : a.game ? t("ui.ov.removedGame") : "");
  const describe = (a: Activity) => {
    const parts: string[] = [];
    const [d0, d1] = (a.detail ?? "").split("|");
    if (a.action === "backup-auto" && d0) parts.push(t(`ui.ov.trigger.${d0}`, { defaultValue: d0 }));
    if (a.action === "backup" && d0) parts.push(t("ui.ov.comment", { text: d0 }));
    if ((a.action === "backup" || a.action === "backup-auto") && d1) parts.push(t("ui.ov.onBranch", { name: d1 }));
    if (a.action === "branch-rename" && d1) parts.push(t("ui.ov.renamed", { from: d0, to: d1 }));
    if (["branch-create", "branch-delete", "tag-create", "tag-delete"].includes(a.action) && d0) parts.push(`「${d0}」`);
    if (a.action.startsWith("export") && d0) parts.push(d0.split(/[\\/]/).pop() ?? d0);
    if (a.action.startsWith("import") && d0) parts.push(d0);
    if (a.n && UNIT[a.action]) parts.push(t(`ui.ov.unit.${UNIT[a.action]}`, { n: a.n }));
    if (a.bytes) parts.push(bytes(a.bytes));
    if (a.action === "sync" && a.ok) parts.push(t("ui.ov.uploaded"));
    return parts.join(t("ui.ov.sep"));
  };

  const items = page?.items ?? [];
  let lastDay = "";
  return (
    <section className="ov-block" aria-label={t("ui.ov.logTitle")}>
      <h2>
        {t("ui.ov.logTitle")}
      </h2>
      <div className="ov-log-tools">
        <div className="chips" role="group" aria-label={t("ui.ov.logFilter")}>
          {CATEGORIES.map((c) => (
            <button key={c} className="chip" aria-pressed={cat === c} onClick={() => setCat(c)}>
              {t(`ui.ov.cat.${c}`)}
              {c === "failed" && page && page.failed > 0 && <span className="num chip-count">{page.failed}</span>}
            </button>
          ))}
        </div>
        <div className="grow" />
        <select className="field" value={game} onChange={(e) => setGame(e.target.value)} aria-label={t("ui.ov.logGame")}>
          <option value="">{t("ui.ov.allGames")}</option>
          {rows.map((r) => (
            <option key={r.id} value={r.id}>
              {displayName(r, lang)}
            </option>
          ))}
        </select>
        <select className="field" value={days} onChange={(e) => setDays(Number(e.target.value))} aria-label={t("ui.ov.range")}>
          {[7, 30, 0].map((d) => (
            <option key={d} value={d}>
              {t(`ui.ov.range.${d}`)}
            </option>
          ))}
        </select>
        <input className="field" type="search" placeholder={t("ui.ov.logSearch")} aria-label={t("ui.ov.logSearch")} value={text} onChange={(e) => setText(e.target.value)} />
      </div>
      <div className="ov-log">
        {page && items.length === 0 && (
          <p className="ov-log-empty">
            {cat === "failed" ? t("ui.ov.logNoFailures") : days > 0 ? t("ui.ov.logEmptyRange") : t("ui.ov.logEmpty")}
            {days === 7 && (
              <>
                {" "}
                <button className="link" onClick={() => setDays(30)}>
                  {t("ui.ov.showMonth")}
                </button>
              </>
            )}
          </p>
        )}
        {!page && <p className="ov-log-empty muted">{t("ui.ov.loading")}</p>}
        {items.map((a, i) => {
          const d = day(a.at);
          const head = d !== lastDay;
          lastDay = d;
          const cls = a.ok ? (a.action === "backup-auto" ? "auto" : a.action.startsWith("restore") || a.action === "undo-restore" ? "restore" : a.action === "sync" ? "sync" : a.action.startsWith("shots") ? "pics" : ["clean", "fsck", "reindex", "scan", "rules-update"].includes(a.action) ? "maint" : "") : a.resolved ? "fail resolved" : "fail";
          const failText = t("ui.ov.failedAction", { what: t(`ui.activity.${a.action}`, { name: nameOf(a), defaultValue: a.action }), why: t(`ui.ov.issue.${a.issue ?? "other"}`, { defaultValue: t("ui.ov.issue.other") }) });
          const go = a.action === "sync" ? () => store.go({ name: "sync" }) : a.game && store.rows[a.game] ? () => store.go({ name: "game", id: a.game! }) : null;
          const meta = describe(a);
          return (
            <div key={`${a.at}-${i}`} className="ov-ev-wrap">
              {head && <div className="ov-day">{d}</div>}
              <div className={`ov-ev ${cls}`}>
                <time className="num" dateTime={a.at}>
                  {new Date(a.at).toLocaleTimeString(lang, { hour: "2-digit", minute: "2-digit" })}
                </time>
                <span className="dot" aria-hidden />
                <p>
                  {a.ok ? (
                    <span>{t(`ui.activity.${a.action}`, { name: nameOf(a), defaultValue: a.action })}</span>
                  ) : a.resolved ? (
                    <span>
                      {failText}{" "}
                      <span className="tag ov-resolved" title={t("ui.ov.resolvedHint")}>
                        {t("ui.ov.resolved")}
                      </span>
                    </span>
                  ) : (
                    <b>{failText}</b>
                  )}
                  {meta && <small> {meta}</small>}
                  {a.error && !a.ok && !a.resolved && (
                    <small className="ov-err" title={a.error}>
                      {" "}
                      {a.error}
                    </small>
                  )}
                </p>
                <span className="ov-ev-acts">
                  {go && (
                    <button className="link" onClick={go}>
                      {a.ok || a.resolved ? t("ui.ov.view") : t("ui.ov.handle")}
                    </button>
                  )}
                  {!a.ok && !a.resolved && (
                    <button className="link ov-mark" onClick={() => void resolve(a)}>
                      {t("ui.ov.markResolved")}
                    </button>
                  )}
                </span>
              </div>
            </div>
          );
        })}
        {page?.more && (
          <button className="btn ghost ov-more" disabled={loadingMore} onClick={() => void more()}>
            {t("ui.ov.more")}
          </button>
        )}
      </div>
    </section>
  );
}
