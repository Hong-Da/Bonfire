import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { Icon, displayName } from "../components/ui";
import { bytes } from "../lib/format";
import { fileName } from "../lib/shots";
import { useStore } from "../store";
import type { GalleryGame, GalleryView, ShotRecord } from "../types";
import { Lightbox, ShotThumb } from "./ShotsPanel";

type Tab = "album" | "trash";
type Group = "game" | "month" | "day" | "source";
type Sort = "new" | "old";

const GROUPS: Group[] = ["game", "month", "day", "source"];
const PREF_KEY = "bonfire.gallery";

interface Prefs {
  group: Group;
  sort: Sort;
}

function readPrefs(): Prefs {
  try {
    const p = JSON.parse(localStorage.getItem(PREF_KEY) ?? "{}") as Partial<Prefs>;
    return { group: GROUPS.includes(p.group as Group) ? (p.group as Group) : "game", sort: p.sort === "old" ? "old" : "new" };
  } catch {
    return { group: "game", sort: "new" };
  }
}

function writePrefs(p: Prefs) {
  try {
    localStorage.setItem(PREF_KEY, JSON.stringify(p));
  } catch {
    // 无法保存偏好时只影响本次。
  }
}

/** 截图的唯一键（不同游戏中可能有内容相同的截图）。 */
const keyOf = (s: ShotRecord) => `${s.gameId}/${s.blob}`;

/**
 * 图库：所有游戏的截图集中查看。按游戏 / 月份 / 日期 / 来源自动分组，可筛选游戏与来源、按时间排序；
 * “回收站”中是所有游戏删除的截图，可恢复或彻底删除。分组与排序方式会记住。
 */
export default function Gallery() {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [data, setData] = useState<GalleryView | null>(null);
  const [tab, setTab] = useState<Tab>("album");
  const [prefs, setPrefs] = useState<Prefs>(readPrefs);
  const [game, setGame] = useState("all");
  const [source, setSource] = useState("all");
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [viewing, setViewing] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    try {
      setData(await api.gallery());
    } catch (e) {
      store.toast("error", errorMessage(e));
    }
  }, []);
  useEffect(() => {
    void reload();
  }, [reload]);

  const games = useMemo(() => new Map((data?.games ?? []).map((g) => [g.id, g])), [data]);
  const gameName = useCallback((id: string) => {
    const g = games.get(id);
    return g ? displayName(g, i18n.language) : id;
  }, [games, i18n.language]);

  const setPref = (p: Partial<Prefs>) =>
    setPrefs((old) => {
      const n = { ...old, ...p };
      writePrefs(n);
      return n;
    });

  const all = (tab === "album" ? data?.shots : data?.trash) ?? [];
  const sources = useMemo(() => [...new Set(all.map((s) => s.source))].sort(), [all]);
  const q = query.trim().toLowerCase();
  const list = useMemo(() => {
    const l = all
      .filter((s) => game === "all" || s.gameId === game)
      .filter((s) => source === "all" || s.source === source)
      .filter((s) => !q || fileName(s.originalPath).toLowerCase().includes(q) || gameName(s.gameId).toLowerCase().includes(q));
    const dir = prefs.sort === "new" ? -1 : 1;
    return [...l].sort((a, b) => dir * a.takenAt.localeCompare(b.takenAt));
  }, [all, game, source, q, prefs.sort, gameName]);

  // 按选定方式分组：组的顺序跟随排序（最新在前时，最近有截图的游戏排在前面）。
  const groups = useMemo(() => {
    const label = (s: ShotRecord): [string, string] => {
      const d = new Date(s.takenAt);
      switch (prefs.group) {
        case "game":
          return [s.gameId, gameName(s.gameId)];
        case "month":
          return [s.takenAt.slice(0, 7), Number.isNaN(d.getTime()) ? "—" : new Intl.DateTimeFormat(i18n.language, { year: "numeric", month: "long" }).format(d)];
        case "day":
          return [s.takenAt.slice(0, 10), Number.isNaN(d.getTime()) ? "—" : new Intl.DateTimeFormat(i18n.language, { dateStyle: "full" }).format(d)];
        case "source":
          return [s.source, t(`ui.shots.source.${s.source}`)];
      }
    };
    const map = new Map<string, { key: string; label: string; shots: ShotRecord[] }>();
    for (const s of list) {
      const [key, text] = label(s);
      let g = map.get(key);
      if (!g) map.set(key, (g = { key, label: text, shots: [] }));
      g.shots.push(s);
    }
    return [...map.values()];
  }, [list, prefs.group, gameName, i18n.language, t]);
  const ordered = useMemo(() => groups.flatMap((g) => g.shots), [groups]);
  const sel = ordered.filter((s) => picked.has(keyOf(s)));

  const switchTab = (v: Tab) => {
    setTab(v);
    setPicked(new Set());
    setViewing(null);
  };

  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    try {
      await f();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  /** 按游戏分批调用（后端的截图操作以游戏为单位）。 */
  const byGame = async (shots: ShotRecord[], f: (gameId: string, blobs: string[]) => Promise<number>) => {
    const m = new Map<string, string[]>();
    for (const s of shots) m.set(s.gameId, [...(m.get(s.gameId) ?? []), s.blob]);
    let n = 0;
    for (const [g, blobs] of m) n += await f(g, blobs);
    return n;
  };

  const changed = async () => {
    setPicked(new Set());
    await reload();
    void store.loadLibrary(false);
  };

  const restore = (shots: ShotRecord[]) =>
    run(async () => {
      const n = await byGame(shots, api.restoreShots);
      store.toast("success", t("ui.shots.restored", { n }));
      await changed();
    });

  const remove = (shots: ShotRecord[]) =>
    run(async () => {
      const n = await byGame(shots, api.deleteShots);
      setViewing(null);
      store.toast("success", t("ui.shots.trashed", { n }), { label: t("ui.shots.undo"), run: () => void restore(shots) });
      await changed();
    });

  const purge = (shots: ShotRecord[], empty: boolean) =>
    run(async () => {
      const text = empty ? t("ui.gallery.emptyTrashConfirm", { n: shots.length }) : t("ui.shots.purgeConfirm", { n: shots.length });
      if (!(await ask(text, { kind: "warning" }))) return;
      const n = await byGame(shots, api.purgeShots);
      store.toast("success", t("ui.shots.purged", { n }));
      await changed();
    });

  const toggle = (k: string) =>
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(k)) n.delete(k);
      else n.add(k);
      return n;
    });
  const toggleGroup = (keys: string[]) =>
    setPicked((s) => {
      const n = new Set(s);
      const every = keys.every((k) => n.has(k));
      for (const k of keys) {
        if (every) n.delete(k);
        else n.add(k);
      }
      return n;
    });

  if (!data) return <div className="empty">{t("ui.loading")}</div>;
  const inTrash = tab === "trash";
  const gameList: GalleryGame[] = [...data.games].filter((g) => all.some((s) => s.gameId === g.id)).sort((a, b) => displayName(a, i18n.language).localeCompare(displayName(b, i18n.language)));
  const timeOpts: Intl.DateTimeFormatOptions = prefs.group === "day" ? { hour: "2-digit", minute: "2-digit" } : { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" };

  return (
    <>
      <div className="toolbar">
        <h1>{t("ui.nav.gallery")}</h1>
        <span className="muted num">{t("ui.gallery.summary", { n: data.shots.length, size: bytes(data.bytes), games: data.games.filter((g) => data.shots.some((s) => s.gameId === g.id)).length })}</span>
        <div className="seg" role="tablist" aria-label={t("ui.nav.gallery")}>
          <button role="tab" aria-selected={!inTrash} onClick={() => switchTab("album")}>
            <Icon name="image" size={14} /> {t("ui.shots.viewAlbum")} <span className="chip-count num">{data.shots.length}</span>
          </button>
          <button role="tab" aria-selected={inTrash} onClick={() => switchTab("trash")}>
            <Icon name="trash" size={14} /> {t("ui.shots.viewTrash")} <span className="chip-count num">{data.trash.length}</span>
          </button>
        </div>
        <div className="grow" />
        <input className="field" type="search" placeholder={t("ui.gallery.search")} value={query} onChange={(e) => setQuery(e.target.value)} aria-label={t("ui.gallery.search")} />
        <button className="btn" onClick={() => void api.openPath(data.dir).catch((e) => store.toast("error", errorMessage(e)))} title={data.dir}>
          <Icon name="folder" /> {t("ui.shots.openDir")}
        </button>
      </div>

      <div className="gallery-bar">
        <label className="inline">
          {t("ui.gallery.groupBy")}
          <select className="field" value={prefs.group} onChange={(e) => setPref({ group: e.target.value as Group })}>
            {GROUPS.map((g) => (
              <option key={g} value={g}>
                {t(`ui.gallery.group.${g}`)}
              </option>
            ))}
          </select>
        </label>
        <label className="inline">
          {t("ui.gallery.sort")}
          <select className="field" value={prefs.sort} onChange={(e) => setPref({ sort: e.target.value as Sort })}>
            <option value="new">{t("ui.gallery.sortNew")}</option>
            <option value="old">{t("ui.gallery.sortOld")}</option>
          </select>
        </label>
        <select className="field" value={game} onChange={(e) => setGame(e.target.value)} aria-label={t("ui.gallery.filterGame")}>
          <option value="all">{t("ui.gallery.allGames")}</option>
          {gameList.map((g) => (
            <option key={g.id} value={g.id}>
              {displayName(g, i18n.language)}
              {g.inLibrary ? "" : ` (${t("ui.gallery.removed")})`} · {all.filter((s) => s.gameId === g.id).length}
            </option>
          ))}
        </select>
        <select className="field" value={source} onChange={(e) => setSource(e.target.value)} aria-label={t("ui.gallery.filterSource")}>
          <option value="all">{t("ui.gallery.allSources")}</option>
          {sources.map((s) => (
            <option key={s} value={s}>
              {t(`ui.shots.source.${s}`)}
            </option>
          ))}
        </select>
        {ordered.length > 0 && (
          <>
            <div className="seg" role="group" aria-label={t("ui.shots.selection")}>
              <button onClick={() => setPicked(new Set(ordered.map(keyOf)))}>{t("ui.shots.selectAll")}</button>
              <button onClick={() => setPicked(new Set())} disabled={sel.length === 0}>
                {t("ui.shots.selectNone")}
              </button>
              <button onClick={() => setPicked(new Set(ordered.filter((s) => !picked.has(keyOf(s))).map(keyOf)))}>{t("ui.shots.invert")}</button>
            </div>
            <span className="count num">{t("ui.shots.selected", { n: sel.length })}</span>
          </>
        )}
        <div className="grow" />
        {inTrash ? (
          <>
            <button className="btn" onClick={() => void restore(sel)} disabled={busy || sel.length === 0}>
              <Icon name="undo" /> {t("ui.shots.restore")}
            </button>
            <button className="btn" onClick={() => void purge(sel, false)} disabled={busy || sel.length === 0}>
              {t("ui.shots.purge")}
            </button>
            <button className="btn ghost" onClick={() => void purge(data.trash, true)} disabled={busy || data.trash.length === 0}>
              {t("ui.shots.emptyTrash")}
            </button>
          </>
        ) : (
          <button className="btn" onClick={() => void remove(sel)} disabled={busy || sel.length === 0}>
            <Icon name="trash" /> {t("ui.shots.delete")}
          </button>
        )}
      </div>

      <div className="gallery">
        {inTrash && data.trash.length > 0 && <p className="hint trash-hint">{t("ui.shots.trashHint", { size: bytes(data.trashBytes) })}</p>}
        {ordered.length === 0 ? (
          <div className="empty">
            <h2>{inTrash ? t("ui.shots.trashEmpty") : all.length > 0 ? t("ui.gallery.noMatch") : t("ui.gallery.emptyTitle")}</h2>
            <p>{inTrash ? t("ui.shots.trashEmptyBody") : all.length > 0 ? "" : t("ui.gallery.emptyBody")}</p>
            {!inTrash && all.length === 0 && (
              <div className="actions">
                <button className="btn primary" onClick={() => store.go({ name: "scan" })}>
                  <Icon name="search" /> {t("ui.gallery.toScan")}
                </button>
              </div>
            )}
          </div>
        ) : (
          groups.map((g) => {
            const keys = g.shots.map(keyOf);
            const n = keys.filter((k) => picked.has(k)).length;
            const gameInfo = prefs.group === "game" ? games.get(g.key) : undefined;
            return (
              <section key={g.key} className="shot-day">
                <h3>
                  <label className="day-pick">
                    <input
                      type="checkbox"
                      className="check"
                      checked={n === keys.length}
                      ref={(el) => {
                        if (el) el.indeterminate = n > 0 && n < keys.length;
                      }}
                      onChange={() => toggleGroup(keys)}
                      aria-label={t("ui.gallery.pickGroup", { name: g.label })}
                    />
                    {g.label}
                  </label>
                  <span className="muted num">{t("ui.shots.dayCount", { n: keys.length })}</span>
                  {gameInfo && gameInfo.inLibrary && (
                    <button className="btn ghost small" onClick={() => store.go({ name: "game", id: gameInfo.id })}>
                      {t("ui.gallery.openGame")}
                    </button>
                  )}
                  {gameInfo && !gameInfo.inLibrary && <span className="tag">{t("ui.gallery.removed")}</span>}
                </h3>
                <ul className="shot-grid">
                  {g.shots.map((s) => {
                    const k = keyOf(s);
                    const i = ordered.indexOf(s);
                    const on = picked.has(k);
                    const name = fileName(s.originalPath);
                    return (
                      <li key={k} className={on ? "on" : undefined}>
                        <button className="shot-open" onClick={() => (inTrash ? toggle(k) : setViewing(i))} title={`${gameName(s.gameId)} · ${name}`}>
                          <ShotThumb gameId={s.gameId} blob={s.blob} alt={name} />
                        </button>
                        <input type="checkbox" className="check shot-pick" checked={on} onChange={() => toggle(k)} aria-label={name} />
                        <span className="shot-time num">
                          {prefs.group !== "game" && <span className="shot-game">{gameName(s.gameId)}</span>}
                          {new Date(s.takenAt).toLocaleString(i18n.language, timeOpts)}
                        </span>
                      </li>
                    );
                  })}
                </ul>
              </section>
            );
          })
        )}
      </div>

      {!inTrash && viewing !== null && ordered[viewing] && (
        <Lightbox
          gameId={ordered[viewing].gameId}
          title={gameName(ordered[viewing].gameId)}
          shot={ordered[viewing]}
          index={viewing}
          total={ordered.length}
          onPrev={() => setViewing((v) => (v === null ? v : Math.max(0, v - 1)))}
          onNext={() => setViewing((v) => (v === null ? v : Math.min(ordered.length - 1, v + 1)))}
          onClose={() => setViewing(null)}
          onDelete={() => void remove([ordered[viewing]])}
        />
      )}
    </>
  );
}
