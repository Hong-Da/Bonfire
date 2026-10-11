import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { confirm } from "../lib/confirm";
import { bytes, relative } from "../lib/format";
import { fuzzyMatch } from "../lib/search";
import { useStore } from "../store";
import { Icon, Menu, Modal, displayName, type IconName } from "../components/ui";
import type { SyncPlan, SyncTarget, TargetStatus } from "../types";

/** 界面上的六种目标；网盘、远程目录、指定目录都是文件夹目标（网盘也可以选 WebDAV）。 */
export const PLACES = ["github", "gitee", "webdav", "cloud", "network", "local"] as const;
export type Place = (typeof PLACES)[number];

const ICONS: Record<Place, IconName> = { github: "branch", gitee: "branch", webdav: "cloud", cloud: "cloud", network: "folder", local: "folder" };

export const placeOf = (t: SyncTarget): Place => (PLACES as readonly string[]).includes(t.place) ? (t.place as Place) : t.kind === "folder" ? "local" : (t.kind as Place);

function kindOf(p: Place): SyncTarget["kind"] {
  return p === "github" || p === "gitee" || p === "webdav" ? p : "folder";
}

/** 新目标的默认值（与后台 SyncTarget::new 一致）。 */
export function newTarget(p: Place): SyncTarget {
  const kind = kindOf(p);
  const git = kind === "github" || kind === "gitee";
  return {
    id: "",
    name: "",
    kind,
    place: p,
    url: "",
    branch: "main",
    user: "",
    content: "repo",
    shots: !git,
    settings: true,
    games: [],
    max_file_mb: kind === "github" ? 100 : kind === "gitee" ? 50 : 0,
    max_total_mb: kind === "github" ? 1024 : kind === "gitee" ? 500 : 0,
    schedule: "manual",
    at: "21:00",
    every_hours: 24,
    enabled: true,
  };
}

type Running = Record<string, { done: number; total: number }>;

/** 同步备份页（REQ-015）：把仓库、最新存档、截图与篝火设置同步到多个远程目标。 */
export default function SyncPage() {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [list, setList] = useState<TargetStatus[] | null>(null);
  const [editing, setEditing] = useState<SyncTarget | null>(null);
  const [preview, setPreview] = useState<SyncTarget | null>(null);
  const [restoring, setRestoring] = useState<SyncTarget | null>(null);
  const [running, setRunning] = useState<Running>({});

  const reload = () => api.syncStatus().then(setList).catch((e) => store.toast("error", errorMessage(e)));
  useEffect(() => {
    void reload();
    const offP = api.onSyncProgress((p) => setRunning((r) => ({ ...r, [p.id]: { done: p.done, total: p.total } })));
    const offD = api.onSyncDone((d) => {
      setRunning((r) => {
        const n = { ...r };
        delete n[d.id];
        return n;
      });
      void reload();
    });
    return () => {
      void offP.then((f) => f());
      void offD.then((f) => f());
    };
  }, []);

  const run = async (target: SyncTarget) => {
    setRunning((r) => ({ ...r, [target.id]: { done: 0, total: 0 } }));
    const d = await api.syncPush(target.id).catch((e) => ({ id: target.id, run: null, error: errorMessage(e) }));
    if (d.error) store.toast("error", t("ui.sync.failed", { name: target.name, msg: d.error }));
    else if (d.run) store.toast("success", t("ui.sync.done", { name: target.name, n: d.run.uploaded, size: bytes(d.run.uploadedBytes) }));
    setRunning((r) => {
      const n = { ...r };
      delete n[target.id];
      return n;
    });
    void reload();
  };

  const runAll = async () => {
    for (const s of list ?? []) if (s.target.enabled) await run(s.target);
  };

  const remove = async (target: SyncTarget) => {
    if (!(await confirm(t("ui.sync.removeConfirm", { name: target.name }), { title: t("ui.sync.remove"), kind: "warning" }))) return;
    try {
      await api.syncRemove(target.id);
      void reload();
    } catch (e) {
      store.toast("error", errorMessage(e));
    }
  };

  const compact = async (target: SyncTarget) => {
    if (!(await confirm(t("ui.sync.compactConfirm", { name: target.name }), { title: t("ui.sync.compact"), kind: "warning", ok: t("ui.sync.compactOk") }))) return;
    try {
      await api.syncCompact(target.id);
      store.toast("success", t("ui.sync.compacted", { name: target.name }));
      void reload();
    } catch (e) {
      store.toast("error", errorMessage(e));
    }
  };

  const games = useMemo(() => store.order.map((id) => store.rows[id]).filter(Boolean), [store.order, store.rows]);

  return (
    <>
      <div className="toolbar">
        <h1>{t("ui.sync.title")}</h1>
        <div className="grow" />
        {list && list.length > 0 && (
          <button className="btn" onClick={() => void runAll()} disabled={Object.keys(running).length > 0}>
            <Icon name="cloud" /> {t("ui.sync.runAll")}
          </button>
        )}
        <button className="btn primary" onClick={() => setEditing(newTarget("local"))}>
          <Icon name="plus" /> {t("ui.sync.add")}
        </button>
      </div>
      <div className="page sync-page">
        {list === null ? (
          <p className="muted">{t("ui.loading")}</p>
        ) : list.length === 0 ? (
          <div className="empty sync-empty">
            <Icon name="cloud" size={40} />
            <h2>{t("ui.sync.emptyTitle")}</h2>
            <p>{t("ui.sync.emptyBody")}</p>
            <button className="btn primary" onClick={() => setEditing(newTarget("local"))}>
              <Icon name="plus" /> {t("ui.sync.add")}
            </button>
          </div>
        ) : (
          <>
            <ul className="sync-list">
              {list.map((s) => {
                const tg = s.target;
                const p = placeOf(tg);
                const r = running[tg.id];
                const last = s.last;
                return (
                  <li key={tg.id} className={`sync-card${!tg.enabled ? " off" : ""}${last?.error ? " failed" : ""}`}>
                    <span className={`sync-kind kind-${p}`} aria-hidden>
                      <Icon name={ICONS[p]} size={20} />
                    </span>
                    <div className="sync-main">
                      <div className="sync-head">
                        <b>{tg.name}</b>
                        <span className="tag">{t(`ui.sync.place.${p}`)}</span>
                        {!tg.enabled && <span className="tag">{t("ui.sync.disabled")}</span>}
                      </div>
                      <span className="sync-url" title={tg.url}>
                        {tg.url}
                      </span>
                      <div className="sync-chips">
                        <span>{t(`ui.sync.content.${tg.content}`)}</span>
                        {tg.content !== "shots" && tg.shots && <span>{t("ui.sync.withShots")}</span>}
                        {tg.settings && <span>{t("ui.sync.withSettings")}</span>}
                        <span>{tg.games.length === 0 ? t("ui.sync.allGames") : t("ui.sync.someGames", { n: tg.games.length })}</span>
                        <span>{t(`ui.sync.schedule.${tg.schedule}`, { at: tg.at, n: tg.every_hours })}</span>
                      </div>
                      {r ? (
                        <div className="sync-progress" role="progressbar" aria-valuemin={0} aria-valuemax={r.total || 1} aria-valuenow={r.done}>
                          <i style={{ width: `${r.total ? Math.min(100, (r.done / r.total) * 100) : 4}%` }} />
                          <span>{r.total ? t("ui.sync.progress", { done: bytes(r.done), total: bytes(r.total) }) : t("ui.sync.preparing")}</span>
                        </div>
                      ) : (
                        <p className={`sync-last${last?.error ? " error" : ""}`}>
                          {!last
                            ? t("ui.sync.never")
                            : last.error
                              ? t("ui.sync.lastFailed", { when: relative(last.at, i18n.language), msg: last.error })
                              : t("ui.sync.lastOk", { when: relative(last.at, i18n.language), n: last.uploaded, size: bytes(last.uploadedBytes) })}
                          {last && last.excluded.length > 0 && !last.error && <span className="warn-hint"> {t("ui.sync.lastExcluded", { n: last.excluded.length })}</span>}
                          {tg.kind !== "folder" && !s.hasSecret && <span className="warn-hint"> {t(tg.kind === "webdav" ? "ui.sync.noPassword" : "ui.sync.noToken")}</span>}
                        </p>
                      )}
                    </div>
                    <div className="sync-actions">
                      <button className="btn primary" onClick={() => setPreview(tg)} disabled={!!r}>
                        {t("ui.sync.syncNow")}
                      </button>
                      <button className="btn" onClick={() => setEditing(tg)} disabled={!!r}>
                        <Icon name="edit" /> {t("ui.sync.edit")}
                      </button>
                      <Menu
                        icon="more"
                        className="btn ghost"
                        align="right"
                        title={t("ui.more")}
                        items={[
                          ...(tg.content === "repo" ? [{ label: t("ui.sync.restore"), hint: t("ui.sync.restoreHint"), onClick: () => setRestoring(tg) }] : []),
                          ...(tg.kind === "github" || tg.kind === "gitee" ? [{ label: t("ui.sync.compact"), hint: t("ui.sync.compactHint"), onClick: () => void compact(tg) }] : []),
                          { label: t("ui.sync.remove"), danger: true, onClick: () => void remove(tg) },
                        ]}
                      />
                    </div>
                  </li>
                );
              })}
            </ul>
            <Coverage list={list} games={games.map((g) => ({ id: g.id, name: displayName(g, i18n.language), names: [g.name, g.nameZh, g.id] }))} />
          </>
        )}
      </div>
      {editing && (
        <TargetDialog
          initial={editing}
          hasSecret={!!list?.find((s) => s.target.id === editing.id)?.hasSecret}
          games={games.map((g) => ({ id: g.id, name: displayName(g, i18n.language) }))}
          onClose={() => setEditing(null)}
          onSaved={async (saved, isNew) => {
            setEditing(null);
            await reload();
            // 新添加的目标默认询问是否立即同步（先看预览）。
            if (isNew && saved.enabled && (await confirm(t("ui.sync.syncNowAsk", { name: saved.name }), { title: t("ui.sync.saved"), ok: t("ui.sync.syncNow"), icon: "cloud" }))) setPreview(saved);
          }}
        />
      )}
      {preview && (
        <PreviewDialog
          target={preview}
          onClose={() => setPreview(null)}
          onRun={() => {
            const tg = preview;
            setPreview(null);
            void run(tg);
          }}
        />
      )}
      {restoring && <RestoreDialog target={restoring} onClose={() => setRestoring(null)} />}
    </>
  );
}

/** 每个游戏的存档与截图同步到了哪些目标（上次同步成功的）。没有任何远程备份的游戏标出来。 */
type CovState = "ok" | "failed" | "pending";
type CovFilter = "todo" | "none" | "problem" | "ok" | "all";
const COV_FILTERS: CovFilter[] = ["todo", "none", "problem", "ok", "all"];
const COV_PAGE = 60;

/** 目标对这个游戏的状态：同步成功 / 最近一次失败 / 还没同步成功过（含缺令牌）。 */
function targetState(s: TargetStatus): CovState {
  if (s.last?.error) return "failed";
  if (!s.last || (s.target.kind !== "folder" && !s.hasSecret)) return "pending";
  return "ok";
}

/**
 * 每个游戏备份到了哪里：默认只列需要处理的游戏（还没同步到任何地方、同步有问题），
 * 可按状态、目标筛选并搜索；游戏多时分页显示。
 */
function Coverage({ list, games }: { list: TargetStatus[]; games: { id: string; name: string; names: (string | null | undefined)[] }[] }) {
  const { t } = useTranslation();
  const [filter, setFilter] = useState<CovFilter>("todo");
  const [target, setTarget] = useState("all");
  const [query, setQuery] = useState("");
  const [shown, setShown] = useState(COV_PAGE);

  const rows = useMemo(() => {
    const active = list.filter((s) => s.target.enabled);
    return games.map((g) => {
      const on = active
        .filter((s) => s.target.content !== "shots" && (s.target.games.length === 0 || s.target.games.includes(g.id)))
        .map((s) => ({ s, state: targetState(s), shots: s.target.shots }));
      const okCount = on.filter((x) => x.state === "ok").length;
      const status: "none" | "problem" | "ok" = okCount === 0 ? "none" : on.some((x) => x.state !== "ok") ? "problem" : "ok";
      return { g, on, status };
    });
  }, [list, games]);

  if (games.length === 0) return null;
  const count = (f: CovFilter) => rows.filter((r) => f === "all" || r.status === f || (f === "todo" && r.status !== "ok")).length;
  const visible = rows.filter(
    (r) =>
      (filter === "all" || r.status === filter || (filter === "todo" && r.status !== "ok")) &&
      (target === "all" || r.on.some((x) => x.s.target.id === target)) &&
      (!query.trim() || fuzzyMatch(query, r.g.names)),
  );
  const todo = count("todo");

  return (
    <section className="sync-coverage">
      <div className="cov-head">
        <h2>{t("ui.sync.coverage")}</h2>
        <p className={todo > 0 ? "warn-hint" : "muted"}>
          {todo > 0
            ? [count("none") > 0 && t("ui.sync.covSumNone", { n: count("none") }), count("problem") > 0 && t("ui.sync.covSumProblem", { n: count("problem") })].filter(Boolean).join(t("ui.sync.covSep"))
            : t("ui.sync.allCovered")}
        </p>
      </div>
      <div className="cov-tools">
        <div className="chips" role="group" aria-label={t("ui.sync.covFilter")}>
          {COV_FILTERS.map((f) => (
            <button key={f} className="chip" aria-pressed={filter === f} onClick={() => { setFilter(f); setShown(COV_PAGE); }}>
              {t(`ui.sync.covFilter.${f}`)}
              <span className="num chip-count">{count(f)}</span>
            </button>
          ))}
        </div>
        <div className="grow" />
        {list.length > 1 && (
          <select className="field" value={target} onChange={(e) => setTarget(e.target.value)} aria-label={t("ui.sync.covTarget")}>
            <option value="all">{t("ui.sync.covAllTargets")}</option>
            {list.map((s) => (
              <option key={s.target.id} value={s.target.id}>
                {s.target.name}
              </option>
            ))}
          </select>
        )}
        <input className="field" type="search" placeholder={t("ui.sync.covSearch")} value={query} onChange={(e) => { setQuery(e.target.value); setShown(COV_PAGE); }} aria-label={t("ui.sync.covSearch")} />
      </div>

      {visible.length === 0 ? (
        <div className="cov-empty">
          <p>{filter === "todo" && !query.trim() && target === "all" ? t("ui.sync.covNothingTodo") : t("ui.sync.covNoMatch")}</p>
          {filter !== "all" && (
            <button className="btn ghost" onClick={() => setFilter("all")}>
              {t("ui.sync.covShowAll", { n: rows.length })}
            </button>
          )}
        </div>
      ) : (
        <ul className="cov-list">
          {visible.slice(0, shown).map(({ g, on, status }) => (
            <li key={g.id} className={`cov-row cov-${status}`}>
              <span className="cov-dot" aria-hidden />
              <span className="cov-name">{g.name}</span>
              <span className="cov-where">
                {on.length === 0 ? (
                  <span className="cov-note">{t("ui.sync.covNoTarget")}</span>
                ) : (
                  on.map(({ s, state, shots }) => (
                    <span key={s.target.id} className={`cov-tag ${state}`} title={`${t(`ui.sync.covState.${state}`)}${shots ? ` · ${t("ui.sync.withShots")}` : ""}`}>
                      {s.target.name}
                      {state !== "ok" && <small>{t(`ui.sync.covState.${state}`)}</small>}
                    </span>
                  ))
                )}
              </span>
            </li>
          ))}
        </ul>
      )}
      {visible.length > shown && (
        <button className="btn ghost cov-more" onClick={() => setShown(shown + COV_PAGE)}>
          {t("ui.sync.covMore", { n: visible.length - shown })}
        </button>
      )}
    </section>
  );
}

function TargetDialog({ initial, hasSecret, games, onClose, onSaved }: { initial: SyncTarget; hasSecret: boolean; games: { id: string; name: string }[]; onClose: () => void; onSaved: (saved: SyncTarget, isNew: boolean) => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [tg, setTg] = useState<SyncTarget>(initial);
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [test, setTest] = useState<{ ok: boolean; msg: string } | null>(null);
  const isNew = !initial.id;
  const p = placeOf(tg);
  const git = tg.kind === "github" || tg.kind === "gitee";
  const needsSecret = tg.kind !== "folder";
  const set = (patch: Partial<SyncTarget>) => {
    setTg({ ...tg, ...patch });
    setTest(null);
  };
  const choose = (np: Place) => {
    const d = newTarget(np);
    setTg({ ...d, id: tg.id, name: tg.name, url: kindOf(np) === tg.kind ? tg.url : "", games: tg.games, schedule: tg.schedule, at: tg.at, every_hours: tg.every_hours });
    setTest(null);
  };

  const pickFolder = async () => {
    const r = await open({ directory: true, multiple: false, title: t("ui.sync.pickFolder") });
    if (typeof r === "string") set({ url: r });
  };

  const doTest = async () => {
    setBusy(true);
    try {
      setTest({ ok: true, msg: await api.syncTest(tg, secret || null) });
    } catch (e) {
      setTest({ ok: false, msg: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    setBusy(true);
    try {
      const saved = await api.syncSave({ ...tg, name: tg.name.trim() || t(`ui.sync.place.${p}`) }, secret || null);
      if (!isNew) store.toast("success", t("ui.sync.saved"));
      onSaved(saved, isNew);
    } catch (e) {
      store.toast("error", errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <Modal
      title={isNew ? t("ui.sync.addTitle") : t("ui.sync.editTitle", { name: initial.name })}
      wide
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <button className="btn ghost" onClick={() => void doTest()} disabled={busy || !tg.url.trim()}>
            {t("ui.sync.test")}
          </button>
          <div className="grow" />
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t("ui.cancel")}
          </button>
          <button className="btn primary" onClick={() => void save()} disabled={busy || !tg.url.trim()}>
            {t("ui.save")}
          </button>
        </>
      }
    >
      <div className="sync-form">
        {isNew && (
          <div className="format-tiles sync-places" role="radiogroup" aria-label={t("ui.sync.type")}>
            {PLACES.map((pl) => (
              <button key={pl} type="button" role="radio" aria-checked={p === pl} className="format-tile" onClick={() => choose(pl)}>
                <span className={`sync-kind kind-${pl}`} aria-hidden>
                  <Icon name={ICONS[pl]} size={18} />
                </span>
                <span>
                  <b>{t(`ui.sync.place.${pl}`)}</b>
                  <small>{t(`ui.sync.placeHint.${pl}`)}</small>
                </span>
              </button>
            ))}
          </div>
        )}

        <label className="sync-field">
          <span>{t("ui.sync.name")}</span>
          <input className="field" value={tg.name} placeholder={t(`ui.sync.place.${p}`)} onChange={(e) => set({ name: e.target.value })} />
        </label>
        <label className="sync-field">
          <span>{t(`ui.sync.url.${tg.kind}`)}</span>
          <span className="inline">
            <input className="field" value={tg.url} placeholder={t(`ui.sync.urlHint.${p}`)} onChange={(e) => set({ url: e.target.value })} />
            {tg.kind === "folder" && (
              <button type="button" className="btn" onClick={() => void pickFolder()}>
                {t("ui.settings.browse")}
              </button>
            )}
          </span>
        </label>
        {git && (
          <label className="sync-field">
            <span>{t("ui.sync.branch")}</span>
            <input className="field" value={tg.branch} onChange={(e) => set({ branch: e.target.value })} style={{ maxWidth: 200 }} />
          </label>
        )}
        {(tg.kind === "gitee" || tg.kind === "webdav") && (
          <label className="sync-field">
            <span>{t(tg.kind === "gitee" ? "ui.sync.userOptional" : "ui.sync.user")}</span>
            <input className="field" value={tg.user} onChange={(e) => set({ user: e.target.value })} style={{ maxWidth: 280 }} />
          </label>
        )}
        {needsSecret && (
          <label className="sync-field">
            <span>{t(git ? "ui.sync.token" : "ui.sync.password")}</span>
            <input className="field" type="password" aria-label={t(git ? "ui.sync.token" : "ui.sync.password")} value={secret} placeholder={hasSecret ? t("ui.sync.secretKept") : ""} onChange={(e) => setSecret(e.target.value)} style={{ maxWidth: 360 }} />
            <small className="hint">{t(git ? `ui.sync.tokenHint.${tg.kind}` : "ui.sync.passwordHint")}</small>
          </label>
        )}
        {test && (
          <p className={`callout${test.ok ? "" : " error"}`} role="status">
            {test.msg}
          </p>
        )}

        <fieldset className="sync-content">
          <legend>{t("ui.sync.contentTitle")}</legend>
          {(["repo", "latest", "shots"] as const).map((c) => (
            <label key={c} className="opt sync-radio">
              <input type="radio" name="sync-content" checked={tg.content === c} onChange={() => set({ content: c })} />
              <span>
                <b>{t(`ui.sync.content.${c}`)}</b>
                <small>{t(`ui.sync.contentHint.${c}`)}</small>
              </span>
            </label>
          ))}
          <div className="inline sync-extras">
            {tg.content !== "shots" && (
              <label className="opt">
                <input type="checkbox" className="check" checked={tg.shots} onChange={(e) => set({ shots: e.target.checked })} />
                {t("ui.sync.withShots")}
              </label>
            )}
            <label className="opt">
              <input type="checkbox" className="check" checked={tg.settings} onChange={(e) => set({ settings: e.target.checked })} />
              {t("ui.sync.withSettings")}
            </label>
          </div>
          {git && tg.shots && <p className="hint warn-hint">{t("ui.sync.gitShotsHint")}</p>}
          {(tg.shots || tg.content === "shots") && <p className="hint">{t("ui.sync.shotsPlainHint")}</p>}
        </fieldset>

        <details className="sync-games">
          <summary>{tg.games.length === 0 ? t("ui.sync.gamesAll") : t("ui.sync.gamesSome", { n: tg.games.length })}</summary>
          <div className="sync-game-list">
            <label className="opt">
              <input type="checkbox" className="check" checked={tg.games.length === 0} onChange={(e) => set({ games: e.target.checked ? [] : games.map((g) => g.id) })} />
              {t("ui.sync.allGames")}
            </label>
            {games.map((g) => (
              <label key={g.id} className="opt">
                <input
                  type="checkbox"
                  className="check"
                  checked={tg.games.length === 0 || tg.games.includes(g.id)}
                  onChange={(e) => {
                    const base = tg.games.length === 0 ? games.map((x) => x.id) : tg.games;
                    const next = e.target.checked ? [...base, g.id] : base.filter((x) => x !== g.id);
                    set({ games: next.length === games.length ? [] : next });
                  }}
                />
                {g.name}
              </label>
            ))}
          </div>
        </details>

        <div className="sync-row2">
          <label className="sync-field">
            <span>{t("ui.sync.maxFile")}</span>
            <span className="inline">
              <input className="field num" type="number" min={0} value={tg.max_file_mb} onChange={(e) => set({ max_file_mb: Math.max(0, Number(e.target.value)) })} style={{ width: 100 }} />
              MB
            </span>
          </label>
          <label className="sync-field">
            <span>{t("ui.sync.maxTotal")}</span>
            <span className="inline">
              <input className="field num" type="number" min={0} value={tg.max_total_mb} onChange={(e) => set({ max_total_mb: Math.max(0, Number(e.target.value)) })} style={{ width: 100 }} />
              MB
            </span>
          </label>
        </div>
        <p className="hint">{t(git ? `ui.sync.limitHint.${tg.kind}` : "ui.sync.limitHint.other")}</p>

        <div className="sync-row2">
          <label className="sync-field">
            <span>{t("ui.sync.when")}</span>
            <select className="field" value={tg.schedule} onChange={(e) => set({ schedule: e.target.value as SyncTarget["schedule"] })}>
              {(["manual", "after-backup", "daily", "hours"] as const).map((v) => (
                <option key={v} value={v}>
                  {t(`ui.sync.scheduleOpt.${v}`)}
                </option>
              ))}
            </select>
          </label>
          {tg.schedule === "daily" && (
            <label className="sync-field">
              <span>{t("ui.sync.at")}</span>
              <input className="field num" type="time" value={tg.at} onChange={(e) => set({ at: e.target.value })} />
            </label>
          )}
          {tg.schedule === "hours" && (
            <label className="sync-field">
              <span>{t("ui.sync.everyHours")}</span>
              <input className="field num" type="number" min={1} max={168} value={tg.every_hours} onChange={(e) => set({ every_hours: Math.max(1, Number(e.target.value)) })} style={{ width: 90 }} />
            </label>
          )}
          <label className="opt" style={{ alignSelf: "end" }}>
            <input type="checkbox" className="check" checked={tg.enabled} onChange={(e) => set({ enabled: e.target.checked })} />
            {t("ui.sync.enabled")}
          </label>
        </div>
      </div>
    </Modal>
  );
}

function PreviewDialog({ target, onClose, onRun }: { target: SyncTarget; onClose: () => void; onRun: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [plan, setPlan] = useState<SyncPlan | null>(null);
  useEffect(() => {
    api
      .syncPlan(target.id)
      .then(setPlan)
      .catch((e) => {
        store.toast("error", errorMessage(e));
        onClose();
      });
  }, []);
  const pct = plan && plan.limitBytes > 0 ? Math.min(100, (plan.remoteBytes / plan.limitBytes) * 100) : 0;
  const git = target.kind === "github" || target.kind === "gitee";
  return (
    <Modal
      title={t("ui.sync.previewTitle", { name: target.name })}
      wide
      onClose={onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose}>
            {t("ui.cancel")}
          </button>
          <button className="btn primary" onClick={onRun} disabled={!plan || !!plan.blocked}>
            <Icon name="cloud" /> {t("ui.sync.start")}
          </button>
        </>
      }
    >
      {!plan ? (
        <p className="muted">{t("ui.sync.planning")}</p>
      ) : (
        <div className="sync-preview">
          <div className="cleanup-figures">
            <div>
              <b className="num">{plan.upload}</b>
              <span>{t("ui.sync.toUpload", { size: bytes(plan.uploadBytes) })}</span>
            </div>
            <div>
              <b className="num">{bytes(plan.remoteBytes)}</b>
              <span>{plan.limitBytes > 0 ? t("ui.sync.remoteOf", { limit: bytes(plan.limitBytes) }) : t("ui.sync.remoteSize")}</span>
            </div>
          </div>
          {plan.limitBytes > 0 && (
            <div className={`size-meter level-${plan.level}`} role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)} aria-label={t("ui.sync.remoteSize")}>
              <i style={{ width: `${pct}%` }} />
            </div>
          )}
          {git && <p className="hint">{t("ui.sync.gitHistoryHint")}</p>}
          <ul className="sync-parts">
            {plan.parts.map(([k, v]) => (
              <li key={k}>
                <span>{t(`ui.sync.part.${k}`)}</span>
                <b className="num">{bytes(v)}</b>
              </li>
            ))}
            {plan.delete > 0 && (
              <li>
                <span>{t("ui.sync.toDelete")}</span>
                <b className="num">{plan.delete}</b>
              </li>
            )}
            <li>
              <span>{t("ui.sync.unchanged")}</span>
              <b className="num">{plan.unchanged}</b>
            </li>
          </ul>
          {plan.level === "warn" && !plan.blocked && <p className="callout">{t("ui.sync.nearLimit")}</p>}
          {plan.blocked && (
            <div className="callout error" role="alert">
              <b>{t("ui.sync.blockedTitle")}</b>
              <p>{plan.blocked}</p>
              <ul className="sync-remedies">
                <li>{t("ui.sync.remedy.shots")}</li>
                <li>{t("ui.sync.remedy.games")}</li>
                <li>{t("ui.sync.remedy.clean")}</li>
                {git && <li>{t("ui.sync.remedy.compact")}</li>}
                <li>{t("ui.sync.remedy.other")}</li>
              </ul>
            </div>
          )}
          {plan.excluded.length > 0 && (
            <section>
              <h3>{t("ui.sync.excludedTitle", { n: plan.excluded.length })}</h3>
              <ul className="bd-list">
                {plan.excluded.map((e) => (
                  <li key={e.path}>
                    <span className="bd-what">{e.path}</span>
                    <span className="num">{bytes(e.size)}</span>
                    <span className="bd-why">{e.reason}</span>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </div>
      )}
    </Modal>
  );
}

function RestoreDialog({ target, onClose }: { target: SyncTarget; onClose: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [dest, setDest] = useState("");
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<{ n: number; settings: boolean } | null>(null);
  const [useRepo, setUseRepo] = useState(true);
  const [applySettings, setApplySettings] = useState(true);

  const pick = async () => {
    const r = await open({ directory: true, multiple: false, title: t("ui.sync.restorePick") });
    if (typeof r === "string") setDest(r);
  };
  const start = async () => {
    setBusy(true);
    try {
      const n = await api.syncRestore(target.id, dest);
      setDone({ n, settings: await api.syncHasSettings(dest) });
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const finish = async () => {
    setBusy(true);
    try {
      if (done?.settings && applySettings) await api.syncApplySettings(dest);
      if (useRepo) {
        const cfg = await api.getSettings();
        const info = await api.saveSettings({ ...cfg, general: { ...cfg.general, repo: dest } });
        store.setInfo(info);
        await store.loadLibrary(false);
      }
      store.toast("success", t("ui.sync.restored"));
      onClose();
    } catch (e) {
      store.toast("error", errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("ui.sync.restoreTitle", { name: target.name })}
      onClose={busy ? () => {} : onClose}
      footer={
        done ? (
          <button className="btn primary" onClick={() => void finish()} disabled={busy}>
            {t("ui.done")}
          </button>
        ) : (
          <>
            <button className="btn ghost" onClick={onClose} disabled={busy}>
              {t("ui.cancel")}
            </button>
            <button className="btn primary" onClick={() => void start()} disabled={busy || !dest}>
              {busy ? t("ui.sync.restoring") : t("ui.sync.restoreStart")}
            </button>
          </>
        )
      }
    >
      {done ? (
        <div className="sync-form">
          <p>{t("ui.sync.restoredFiles", { n: done.n, dest })}</p>
          <label className="opt">
            <input type="checkbox" className="check" checked={useRepo} onChange={(e) => setUseRepo(e.target.checked)} />
            {t("ui.sync.useRepo")}
          </label>
          {done.settings && (
            <label className="opt">
              <input type="checkbox" className="check" checked={applySettings} onChange={(e) => setApplySettings(e.target.checked)} />
              {t("ui.sync.applySettings")}
            </label>
          )}
        </div>
      ) : (
        <div className="sync-form">
          <p className="muted">{t("ui.sync.restoreBody")}</p>
          <span className="inline">
            <input className="field" value={dest} readOnly placeholder={t("ui.sync.restorePick")} aria-label={t("ui.sync.restorePick")} />
            <button className="btn" onClick={() => void pick()} disabled={busy}>
              {t("ui.settings.browse")}
            </button>
          </span>
        </div>
      )}
    </Modal>
  );
}
