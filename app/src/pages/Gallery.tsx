import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { prefGet, prefSet } from "../lib/uiState";
import ShotAutoSwitch from "../components/ShotAutoSwitch";
import SelectSeg from "../components/SelectSeg";
import AssignDialog from "../components/AssignDialog";

/** “未识别的截图”相册的游戏 ID。 */
const UNSORTED = "unsorted";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { Icon, Menu, displayName, type IconName } from "../components/ui";
import { bytes } from "../lib/format";
import { fileName, purgeMessage } from "../lib/shots";
import { useMarquee } from "../lib/marquee";
import PurgeDialog from "../components/PurgeDialog";
import { useStore } from "../store";
import type { GalleryGame, GalleryView, PurgeReport, ShotRecord } from "../types";
import { Lightbox, ShotThumb } from "./ShotsPanel";
import { save } from "@tauri-apps/plugin-dialog";
import { CollectionImportDialog } from "./CollectionDialogs";
import { fuzzyMatch } from "../lib/search";

type Tab = "album" | "trash";
type Group = "game" | "month" | "day" | "source";
type Sort = "new" | "old";
type Look = "small" | "medium" | "large" | "list";

const GROUPS: Group[] = ["game", "month", "day", "source"];
const LOOKS: Look[] = ["small", "medium", "large", "list"];
const LOOK_ICON: Record<Look, IconName> = { small: "grid3", medium: "grid2", large: "square", list: "list" };
const PREF_KEY = "bonfire.gallery";

interface Prefs {
  group: Group;
  sort: Sort;
  /** 查看方式：小 / 中 / 大缩略图，或列表。 */
  look: Look;
}

function readPrefs(): Prefs {
  try {
    const p = JSON.parse(prefGet(PREF_KEY) ?? "{}") as Partial<Prefs>;
    return {
      group: GROUPS.includes(p.group as Group) ? (p.group as Group) : "game",
      sort: p.sort === "old" ? "old" : "new",
      look: LOOKS.includes(p.look as Look) ? (p.look as Look) : "medium",
    };
  } catch {
    return { group: "game", sort: "new", look: "medium" };
  }
}

function writePrefs(p: Prefs) {
  try {
    prefSet(PREF_KEY, JSON.stringify(p));
  } catch {
    // 无法保存偏好时只影响本次。
  }
}

const FOLD_KEY = "bonfire.gallery.folded";

/** 折叠的分组（按“分组方式:分组”记住）。 */
function readFolded(): Set<string> {
  try {
    const v = JSON.parse(prefGet(FOLD_KEY) ?? "[]");
    return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function writeFolded(s: Set<string>) {
  try {
    prefSet(FOLD_KEY, JSON.stringify([...s]));
  } catch {
    // 无法保存时只影响本次。
  }
}

/** 截图的唯一键（不同游戏中可能有内容相同的截图）。 */
const keyOf = (s: ShotRecord) => `${s.gameId}/${s.blob}`;

/**
 * 图库：所有游戏的截图集中查看。按游戏 / 月份 / 日期 / 来源自动分组，可筛选游戏与来源、按时间排序；
 * “回收站”中是所有游戏删除的截图，可恢复或彻底删除。可选缩略图大小或列表查看，按住鼠标拖动框选；
 * 分组、排序与查看方式会记住。
 */
export default function Gallery() {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [data, setData] = useState<GalleryView | null>(null);
  const [tab, setTab] = useState<Tab>("album");
  const [prefs, setPrefs] = useState<Prefs>(readPrefs);
  const [folded, setFoldedState] = useState<Set<string>>(readFolded);
  const setFolded = (next: Set<string>) => {
    setFoldedState(next);
    writeFolded(next);
  };
  const foldKey = (k: string) => `${prefs.group}:${k}`;
  const [game, setGame] = useState("all");
  const [source, setSource] = useState("all");
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [viewing, setViewing] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [purgeAsk, setPurgeAsk] = useState<{ text: string; run: (originals: boolean) => void } | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [assign, setAssign] = useState<string[] | null>(null);
  const marquee = useMarquee(scroller, picked, setPicked);

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
    if (id === UNSORTED) return t("ui.unsorted.name");
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
      // 文件名按包含匹配；游戏名模糊匹配（中文名、英文名、拼音）。
      .filter((s) => !q || fileName(s.originalPath).toLowerCase().includes(q) || fuzzyMatch(q, [games.get(s.gameId)?.name, games.get(s.gameId)?.nameZh, s.gameId]));
    const dir = prefs.sort === "new" ? -1 : 1;
    return [...l].sort((a, b) => dir * a.takenAt.localeCompare(b.takenAt));
  }, [all, game, source, q, prefs.sort, games]);

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
    setPurgeAsk({
      text: empty ? t("ui.gallery.emptyTrashConfirm", { n: shots.length }) : t("ui.shots.purgeConfirm", { n: shots.length }),
      run: (originals) =>
        void run(async () => {
          const reports: PurgeReport[] = [];
          await byGame(shots, async (g, blobs) => {
            const r = await api.purgeShots(g, blobs, originals);
            reports.push(r);
            return r.purged;
          });
          store.toast("success", purgeMessage(t, reports, originals));
          await changed();
        }),
    });

  // 选中的截图（可跨游戏）导出为篝火包，导入到另一台电脑的篝火时合并到相册（U-16）。
  const exportSel = (shots: ShotRecord[]) =>
    run(async () => {
      const ids: Record<string, string[]> = {};
      for (const s of shots) (ids[s.gameId] ??= []).push(s.blob);
      const stamp = new Date().toISOString().slice(0, 10).replaceAll("-", "");
      const dest = await save({ defaultPath: `Bonfire-${t("ui.package.shotsFileName")}-${stamp}.bonfire`, filters: [{ name: t("ui.transfer.bundle"), extensions: ["bonfire"] }] });
      if (!dest) return;
      const r = await api.exportPackage(Object.keys(ids), dest, { saves: false, shots: true, shotIds: ids });
      store.toast("success", t("ui.package.exported", { n: r.games.length, s: 0, p: shots.length, size: bytes(r.bytes) }), { label: t("ui.transfer.showFile"), run: () => void api.openPath(r.dest) });
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
  // 来源与游戏筛选生效时，“整理”按钮显示筛选内容。
  const narrowed = game !== "all" || source !== "all";
  const narrowText = [game !== "all" ? gameName(game) : "", source !== "all" ? t(`ui.shots.source.${source}`) : ""].filter(Boolean).join(" · ");
  const timeOpts: Intl.DateTimeFormatOptions = prefs.group === "day" ? { hour: "2-digit", minute: "2-digit" } : { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" };

  return (
    <>
      <div className="toolbar gallery-toolbar">
        <h1>{t("ui.nav.gallery")}</h1>
        <div className="seg" role="tablist" aria-label={t("ui.nav.gallery")}>
          <button role="tab" aria-selected={!inTrash} onClick={() => switchTab("album")}>
            <Icon name="image" size={14} /> {t("ui.shots.viewAlbum")} <span className="chip-count num">{data.shots.length}</span>
          </button>
          <button role="tab" aria-selected={inTrash} onClick={() => switchTab("trash")}>
            <Icon name="trash" size={14} /> {t("ui.shots.viewTrash")} <span className="chip-count num">{data.trash.length}</span>
          </button>
        </div>
        <Menu
          icon="filter"
          className={`btn${narrowed ? " filtered" : ""}`}
          label={narrowed ? t("ui.library.filterOn", { what: narrowText }) : t("ui.gallery.arrange")}
          title={t("ui.gallery.arrangeHint")}
          items={[
            { label: <b className="menu-head">{t("ui.gallery.groupBy")}</b>, onClick: () => {}, disabled: true },
            ...GROUPS.map((g) => ({ label: t(`ui.gallery.group.${g}`), checked: prefs.group === g, onClick: () => setPref({ group: g }) })),
            "divider" as const,
            { label: <b className="menu-head">{t("ui.gallery.sort")}</b>, onClick: () => {}, disabled: true },
            { label: t("ui.gallery.sortNew"), checked: prefs.sort === "new", onClick: () => setPref({ sort: "new" }) },
            { label: t("ui.gallery.sortOld"), checked: prefs.sort === "old", onClick: () => setPref({ sort: "old" }) },
            "divider" as const,
            { label: <b className="menu-head">{t("ui.gallery.filterSource")}</b>, onClick: () => {}, disabled: true },
            { label: t("ui.gallery.allSources"), checked: source === "all", onClick: () => setSource("all") },
            ...sources.map((x) => ({ label: t(`ui.shots.source.${x}`), checked: source === x, onClick: () => setSource(x) })),
            "divider" as const,
            { label: <b className="menu-head">{t("ui.gallery.filterGame")}</b>, onClick: () => {}, disabled: true },
            { label: t("ui.gallery.allGames"), checked: game === "all", onClick: () => setGame("all") },
            ...gameList.map((g) => ({
              label: `${displayName(g, i18n.language)}${g.inLibrary ? "" : `（${t("ui.gallery.removed")}）`} · ${all.filter((x) => x.gameId === g.id).length}`,
              checked: game === g.id,
              onClick: () => setGame(g.id),
            })),
          ]}
        />
        <div className="seg icon-seg" role="radiogroup" aria-label={t("ui.gallery.look")}>
          {LOOKS.map((l) => (
            <button key={l} role="radio" aria-checked={prefs.look === l} onClick={() => setPref({ look: l })} aria-label={t(`ui.gallery.lookOpt.${l}`)} title={t(`ui.gallery.lookOpt.${l}`)}>
              <Icon name={LOOK_ICON[l]} size={16} />
            </button>
          ))}
        </div>
        {/* 一键全部展开 / 折叠：有展开的分组时全部折叠，否则全部展开。 */}
        {groups.length > 1 && (
          <button
            className="btn icon-only"
            onClick={() => {
              const keys = groups.map((g) => foldKey(g.key));
              const anyOpen = keys.some((k) => !folded.has(k));
              const next = new Set(folded);
              keys.forEach((k) => (anyOpen ? next.add(k) : next.delete(k)));
              setFolded(next);
            }}
            title={groups.some((g) => !folded.has(foldKey(g.key))) ? t("ui.gallery.foldAll") : t("ui.gallery.unfoldAll")}
            aria-label={groups.some((g) => !folded.has(foldKey(g.key))) ? t("ui.gallery.foldAll") : t("ui.gallery.unfoldAll")}
          >
            <Icon name={groups.some((g) => !folded.has(foldKey(g.key))) ? "fold" : "unfold"} size={16} />
          </button>
        )}
        <input className="field fill" type="search" placeholder={t("ui.gallery.search")} value={query} onChange={(e) => setQuery(e.target.value)} aria-label={t("ui.gallery.search")} />
        <button className="btn icon-only" onClick={() => setImportOpen(true)} title={t("ui.package.importHint")} aria-label={t("ui.package.importHint")}>
          <Icon name="import" />
        </button>
        <button
          className="btn icon-only"
          onClick={() => void api.openPath(data.dir).catch((e) => store.toast("error", errorMessage(e)))}
          title={`${t("ui.shots.openDir")}：${data.dir}`}
          aria-label={t("ui.shots.openDir")}
        >
          <Icon name="folder" />
        </button>
      </div>

      <div className={`gallery${marquee.box ? " selecting" : ""}`} ref={scroller} onMouseDown={marquee.onMouseDown}>
        {marquee.box && <div className="marquee" style={{ left: marquee.box.x, top: marquee.box.y, width: marquee.box.w, height: marquee.box.h }} />}
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
            const isFolded = folded.has(foldKey(g.key));
            const toggleFold = () => {
              const next = new Set(folded);
              if (isFolded) next.delete(foldKey(g.key));
              else next.add(foldKey(g.key));
              setFolded(next);
            };
            return (
              <section key={g.key} className={`shot-day${isFolded ? " folded" : ""}`}>
                <h3>
                  <button
                    className="fold"
                    onClick={toggleFold}
                    aria-expanded={!isFolded}
                    aria-label={isFolded ? t("ui.gallery.unfold", { name: g.label }) : t("ui.gallery.fold", { name: g.label })}
                    title={isFolded ? t("ui.gallery.unfold", { name: g.label }) : t("ui.gallery.fold", { name: g.label })}
                  >
                    <Icon name="chevron" size={14} />
                  </button>
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
                  {/* 折叠时：日期范围与最新 3 张叠放的缩略图，点这一条展开。 */}
                  {isFolded && (
                    <button type="button" className="fold-peek" onClick={toggleFold} tabIndex={-1} aria-hidden="true">
                      <span className="fold-range num">{rangeText(g.shots, i18n.language, t)}</span>
                      <span className="fold-stack">
                        {g.shots.slice(0, 3).map((s) => (
                          <span key={keyOf(s)} className="fold-photo">
                            <ShotThumb gameId={s.gameId} blob={s.blob} alt="" />
                          </span>
                        ))}
                      </span>
                    </button>
                  )}
                  {gameInfo && gameInfo.inLibrary && (
                    <button className="btn ghost small" onClick={() => store.go({ name: "game", id: gameInfo.id })}>
                      {t("ui.gallery.openGame")}
                    </button>
                  )}
                  {gameInfo && !gameInfo.inLibrary && g.key !== UNSORTED && <span className="tag">{t("ui.gallery.removed")}</span>}
                  {/* 未识别的截图：说明与“归到游戏…”（有勾选时只归勾选的）。 */}
                  {g.key === UNSORTED && !inTrash && (
                    <button
                      className="btn small"
                      onClick={() => {
                        const own = g.shots.filter((s) => picked.has(keyOf(s)));
                        setAssign((own.length > 0 ? own : g.shots).map((s) => s.blob));
                      }}
                    >
                      {n > 0 ? t("ui.unsorted.assignPicked", { n }) : t("ui.unsorted.assignAll", { n: keys.length })}
                    </button>
                  )}
                </h3>
                {g.key === UNSORTED && !isFolded && <p className="hint unsorted-hint">{t("ui.unsorted.hint")}</p>}
                {isFolded ? null : prefs.look === "list" ? (
                  <ul className="shot-list">
                    {g.shots.map((s) => {
                      const k = keyOf(s);
                      const i = ordered.indexOf(s);
                      const on = picked.has(k);
                      const name = fileName(s.originalPath);
                      return (
                        <li key={k} data-key={k} className={on ? "on" : undefined} onClick={() => toggle(k)}>
                          <input type="checkbox" className="check" checked={on} onChange={() => toggle(k)} onClick={(e) => e.stopPropagation()} aria-label={name} />
                          <button
                            className="shot-open"
                            onClick={(e) => {
                              e.stopPropagation();
                              if (inTrash) toggle(k);
                              else setViewing(i);
                            }}
                            title={name}
                          >
                            <ShotThumb gameId={s.gameId} blob={s.blob} alt={name} />
                          </button>
                          <span className="sl-name" title={s.originalPath}>
                            {name}
                          </span>
                          <span className="sl-game">{gameName(s.gameId)}</span>
                          <span className="num muted">{new Date(s.takenAt).toLocaleString(i18n.language, { dateStyle: "medium", timeStyle: "short" })}</span>
                          <span className="muted">{t(`ui.shots.source.${s.source}`)}</span>
                          <span className="num muted">{s.width && s.height ? `${s.width}×${s.height}` : ""}</span>
                          <span className="num muted">{bytes(s.size)}</span>
                        </li>
                      );
                    })}
                  </ul>
                ) : (
                  <ul className={`shot-grid look-${prefs.look}`}>
                    {g.shots.map((s) => {
                      const k = keyOf(s);
                      const i = ordered.indexOf(s);
                      const on = picked.has(k);
                      const name = fileName(s.originalPath);
                      return (
                        <li key={k} data-key={k} className={on ? "on" : undefined}>
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
                )}
              </section>
            );
          })
        )}
      </div>

      <div className="actionbar">
        {/* 全选框与反选：与游戏库底栏一致。 */}
        <SelectSeg
          total={ordered.length}
          picked={sel.length}
          onAll={() => setPicked(new Set(ordered.map(keyOf)))}
          onNone={() => setPicked(new Set())}
          onInvert={() => setPicked(new Set(ordered.filter((s) => !picked.has(keyOf(s))).map(keyOf)))}
          label={t("ui.shots.selection")}
          hint={t("ui.gallery.selectAllHint")}
        />
        {/* 统计跟随选择：选中截图时显示所选的数量、大小与游戏数，否则显示全部。 */}
        <span className={`num gallery-sum${sel.length > 0 ? " has" : " muted"}`} aria-live="polite">
          {sel.length > 0
            ? inTrash
              ? t("ui.gallery.selTrashSummary", { n: sel.length, size: bytes(sel.reduce((n, x) => n + x.size, 0)) })
              : t("ui.gallery.selSummary", { n: sel.length, size: bytes(sel.reduce((n, x) => n + x.size, 0)), games: new Set(sel.map((x) => x.gameId)).size })
            : inTrash
              ? t("ui.gallery.trashSummary", { n: data.trash.length, size: bytes(data.trashBytes) })
              : t("ui.gallery.summary", { n: data.shots.length, size: bytes(data.bytes), games: data.games.filter((g) => data.shots.some((s) => s.gameId === g.id)).length })}
        </span>
        <div className="grow" style={{ flex: 1 }} />
        {!inTrash && <ShotAutoSwitch onSynced={() => void reload()} />}
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
          <>
            <button className="btn" onClick={() => void exportSel(sel)} disabled={busy || sel.length === 0} title={t("ui.package.galleryExportHint")}>
              <Icon name="export" /> {t("ui.package.galleryExport")}
            </button>
            <button className="btn" onClick={() => void remove(sel)} disabled={busy || sel.length === 0}>
              <Icon name="trash" /> {t("ui.shots.delete")}
            </button>
          </>
        )}
      </div>
      {assign && (
        <AssignDialog
          blobs={assign}
          onClose={() => setAssign(null)}
          onDone={() => {
            setPicked(new Set());
            void reload();
          }}
        />
      )}
      {importOpen && <CollectionImportDialog onClose={() => setImportOpen(false)} onDone={() => void reload()} />}
      {purgeAsk && <PurgeDialog text={purgeAsk.text} onConfirm={purgeAsk.run} onClose={() => setPurgeAsk(null)} />}
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

/** 分组中截图的日期范围：同一天为“10月9日”，否则“10月4日至10月9日”。 */
function rangeText(shots: ShotRecord[], lang: string, t: (k: string, o?: Record<string, unknown>) => string): string {
  if (shots.length === 0) return "";
  const times = shots.map((s) => new Date(s.takenAt).getTime()).filter((x) => !Number.isNaN(x));
  if (times.length === 0) return "";
  const fmt = (ms: number) => new Date(ms).toLocaleDateString(lang, { month: "short", day: "numeric" });
  const from = fmt(Math.min(...times));
  const to = fmt(Math.max(...times));
  return from === to ? from : t("ui.gallery.foldRange", { from, to });
}
