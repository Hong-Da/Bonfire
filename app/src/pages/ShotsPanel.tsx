import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { prefGet, prefSet } from "../lib/uiState";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, imageSrc } from "../api";
import { Icon, Menu } from "../components/ui";
import { bytes, dateTime, firstLine } from "../lib/format";
import { fileName, groupByDay, purgeMessage, shotSnapshots } from "../lib/shots";
import PurgeDialog from "../components/PurgeDialog";
import { useStore } from "../store";
import type { AlbumView, ExportFormat, ShotRecord, SnapshotNode } from "../types";
import ShotDiffDialog from "./ShotDiffDialog";
import SelectSeg from "../components/SelectSeg";

/** 截图缩略图：进入可视区域后才生成 / 读取。 */
export function ShotThumb({ gameId, blob, alt }: { gameId: string; blob: string; alt: string }) {
  const [src, setSrc] = useState<string | null>(null);
  const ref = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    let off = false;
    const load = () =>
      api
        .shotThumb(gameId, blob)
        .then((p) => !off && setSrc(imageSrc(p) ?? null))
        .catch(() => {});
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      void load();
      return () => {
        off = true;
      };
    }
    const io = new IntersectionObserver(
      (es) => {
        if (es.some((e) => e.isIntersecting)) {
          io.disconnect();
          void load();
        }
      },
      { rootMargin: "200px" },
    );
    io.observe(el);
    return () => {
      off = true;
      io.disconnect();
    };
  }, [gameId, blob]);
  return (
    <span className="shot-thumb" ref={ref}>
      {src && <img src={src} alt={alt} draggable={false} />}
    </span>
  );
}

type View = "album" | "trash";

/**
 * 详情页“截图”页：按拍摄日期分组的相册，可多选导出 / 删除，点开看大图；
 * 删除先进入回收站，可恢复或彻底删除。左侧是这个游戏的截图开关、来源与截图文件夹。
 * 截图只读收集，原图原样保存在仓库的截图文件夹中。
 */
export default function ShotsPanel({
  gameId,
  gameName,
  nodes,
  onChanged,
  onShowSnapshot,
}: {
  gameId: string;
  gameName: string;
  nodes: SnapshotNode[];
  onChanged: () => void;
  onShowSnapshot: (id: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [album, setAlbum] = useState<AlbumView | null>(null);
  const [view, setView] = useState<View>("album");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [viewing, setViewing] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [diffOpen, setDiffOpen] = useState(false);
  const [purgeAsk, setPurgeAsk] = useState<{ text: string; run: (originals: boolean) => void } | null>(null);

  const reload = useCallback(async () => {
    try {
      setAlbum(await api.album(gameId));
    } catch (e) {
      store.toast("error", errorMessage(e));
    }
  }, [gameId]);
  useEffect(() => {
    void reload();
  }, [reload]);

  const list = (view === "album" ? album?.shots : album?.trash) ?? [];
  const groups = useMemo(() => groupByDay(list), [list]);
  const ordered = useMemo(() => groups.flatMap((g) => g.shots), [groups]);
  const snapOf = useMemo(() => shotSnapshots(nodes, album?.shots ?? []), [nodes, album]);

  const switchView = (v: View) => {
    setView(v);
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

  const changed = async () => {
    setPicked(new Set());
    await reload();
    onChanged();
  };

  const sync = () =>
    run(async () => {
      const [r] = await api.syncShots(gameId);
      store.toast("success", r && r.added > 0 ? t("ui.shots.synced", { n: r.added }) : t("ui.shots.syncedNone"));
      await changed();
    });

  const exportShots = (blobs: string[], format: ExportFormat) =>
    run(async () => {
      const dest =
        format === "zip"
          ? await save({ defaultPath: `${gameName}-${t("ui.shots.fileName")}.zip`, filters: [{ name: "Zip", extensions: ["zip"] }] })
          : await open({ directory: true, title: t("ui.transfer.pickParent") });
      if (!dest || Array.isArray(dest)) return;
      const r = await api.exportShots(gameId, blobs, dest, format);
      store.toast("success", t("ui.shots.exported", { n: r.files, size: bytes(r.bytes) }));
      void api.openPath(r.dest);
    });

  const restore = (blobs: string[]) =>
    run(async () => {
      const n = await api.restoreShots(gameId, blobs);
      store.toast("success", t("ui.shots.restored", { n }));
      await changed();
    });

  // 删除只是移入回收站，可以撤销，不再弹确认框。
  const remove = (blobs: string[]) =>
    run(async () => {
      const n = await api.deleteShots(gameId, blobs);
      setViewing(null);
      store.toast("success", t("ui.shots.trashed", { n }), { label: t("ui.shots.undo"), run: () => void restore(blobs) });
      await changed();
    });

  // 彻底删除：先确认，并可选同时删除磁盘上的原图。
  const purge = (blobs: string[]) => {
    const n = blobs.length || (album?.trash.length ?? 0);
    setPurgeAsk({
      text: blobs.length ? t("ui.shots.purgeConfirm", { n }) : t("ui.shots.emptyTrashConfirm", { n }),
      run: (originals) =>
        void run(async () => {
          const r = await api.purgeShots(gameId, blobs, originals);
          store.toast("success", purgeMessage(t, [r], originals));
          await changed();
        }),
    });
  };

  const setEnabled = (v: string) =>
    run(async () => {
      await api.setShotsEnabled(gameId, v === "auto" ? null : v === "on");
      await reload();
      onChanged();
    });

  const addDir = () =>
    run(async () => {
      const d = await open({ directory: true, title: t("ui.shots.addDir") });
      if (!d || Array.isArray(d)) return;
      await api.addShotDir(gameId, d);
      await reload();
      onChanged();
    });

  const removeDir = (d: string) =>
    run(async () => {
      await api.removeShotDir(gameId, d);
      await reload();
      onChanged();
    });

  if (!album) return <div className="empty">{t("ui.loading")}</div>;
  const toggle = (b: string) =>
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(b)) n.delete(b);
      else n.add(b);
      return n;
    });
  const toggleDay = (blobs: string[]) =>
    setPicked((s) => {
      const n = new Set(s);
      const all = blobs.every((b) => n.has(b));
      for (const b of blobs) {
        if (all) n.delete(b);
        else n.add(b);
      }
      return n;
    });
  const sel = ordered.filter((s) => picked.has(s.blob)).map((s) => s.blob);
  const switchValue = album.followsGlobal ? "auto" : album.enabled ? "on" : "off";
  const inTrash = view === "trash";

  return (
    <div className="shots">
      <aside className="shots-side">
        <h2>{t("ui.shots.title")}</h2>
        <p className="muted num">{t("ui.shots.summary", { n: album.shots.length, size: bytes(album.bytes) })}</p>
        {album.enabled && album.pending > 0 && (
          <div className="callout same">
            <p>{t("ui.shots.pending", { n: album.pending })}</p>
          </div>
        )}
        <button className="btn primary" onClick={() => void sync()} disabled={busy || !album.enabled}>
          <Icon name="backup" /> {t("ui.shots.sync")}
        </button>
        <button className="btn" onClick={() => void api.openPath(album.dir).catch((e) => store.toast("error", errorMessage(e)))} title={album.dir}>
          <Icon name="folder" /> {t("ui.shots.openDir")}
        </button>
        <button className="btn" onClick={() => setDiffOpen(true)} title={t("ui.diff.buttonHint")}>
          <Icon name="scan" /> {t("ui.diff.button")}
        </button>
        <p className="hint">{t("ui.shots.storedHint")}</p>

        <label className="field-label" htmlFor="shots-switch">
          {t("ui.shots.collect")}
        </label>
        <select id="shots-switch" className="field" value={switchValue} onChange={(e) => void setEnabled(e.target.value)} disabled={busy}>
          <option value="auto">{t("ui.shots.followGlobal", { state: album.globalEnabled ? t("ui.shots.on") : t("ui.shots.off") })}</option>
          <option value="on">{t("ui.shots.on")}</option>
          <option value="off">{t("ui.shots.off")}</option>
        </select>
        {!album.enabled && <p className="hint">{t("ui.shots.disabledNote")}</p>}

        <h3>{t("ui.shots.sources")}</h3>
        {album.sources.length > 0 ? (
          <ul className="shot-sources">
            {album.sources.map((s) => (
              <li key={s}>{t(`ui.shots.source.${s}`)}</li>
            ))}
          </ul>
        ) : (
          <p className="hint">{t("ui.shots.noSource")}</p>
        )}

        <h3>{t("ui.shots.dirs")}</h3>
        <ul className="shot-dirs">
          {album.dirs.map((d) => (
            <li key={d}>
              <span className="path" title={d}>
                {d}
              </span>
              <button className="btn ghost small" onClick={() => void removeDir(d)} disabled={busy}>
                {t("ui.shots.removeDir")}
              </button>
            </li>
          ))}
        </ul>
        <button className="btn small" onClick={() => void addDir()} disabled={busy}>
          <Icon name="folder" size={14} /> {t("ui.shots.addDir")}
        </button>
        <p className="hint">{t("ui.shots.dirsHint")}</p>
      </aside>

      <section className="shots-main">
        <div className="shots-bar">
          <div className="seg" role="tablist" aria-label={t("ui.shots.title")}>
            <button role="tab" aria-selected={!inTrash} onClick={() => switchView("album")}>
              {t("ui.shots.viewAlbum")} <span className="chip-count num">{album.shots.length}</span>
            </button>
            <button role="tab" aria-selected={inTrash} onClick={() => switchView("trash")}>
              <Icon name="trash" size={14} /> {t("ui.shots.viewTrash")} <span className="chip-count num">{album.trash.length}</span>
            </button>
          </div>
          {ordered.length > 0 && (
            <>
              <SelectSeg
                total={ordered.length}
                picked={sel.length}
                onAll={() => setPicked(new Set(ordered.map((s) => s.blob)))}
                onNone={() => setPicked(new Set())}
                onInvert={() => setPicked(new Set(ordered.filter((s) => !picked.has(s.blob)).map((s) => s.blob)))}
                label={t("ui.shots.selection")}
                hint={t("ui.gallery.selectAllHint")}
              />
              <span className="count num">{t("ui.shots.selected", { n: sel.length })}</span>
            </>
          )}
          <div className="grow" />
          {inTrash ? (
            <>
              <button className="btn" onClick={() => void restore(sel)} disabled={busy || sel.length === 0}>
                <Icon name="undo" /> {t("ui.shots.restore")}
              </button>
              <button className="btn" onClick={() => void purge(sel)} disabled={busy || sel.length === 0}>
                {t("ui.shots.purge")}
              </button>
              <button className="btn ghost" onClick={() => void purge([])} disabled={busy || album.trash.length === 0}>
                {t("ui.shots.emptyTrash")}
              </button>
            </>
          ) : (
            ordered.length > 0 && (
              <>
                <Menu
                  icon="export"
                  label={t("ui.shots.export")}
                  align="right"
                  items={[
                    { label: t("ui.shots.exportZip"), onClick: () => void exportShots(sel, "zip") },
                    { label: t("ui.shots.exportFolder"), onClick: () => void exportShots(sel, "folder") },
                  ]}
                />
                <button className="btn" onClick={() => void remove(sel)} disabled={busy || sel.length === 0}>
                  <Icon name="trash" /> {t("ui.shots.delete")}
                </button>
              </>
            )
          )}
        </div>
        {inTrash && album.trash.length > 0 && <p className="hint trash-hint">{t("ui.shots.trashHint", { size: bytes(album.trashBytes) })}</p>}

        {ordered.length === 0 ? (
          inTrash ? (
            <div className="empty">
              <h2>{t("ui.shots.trashEmpty")}</h2>
              <p>{t("ui.shots.trashEmptyBody")}</p>
            </div>
          ) : (
            <div className="empty">
              <h2>{t("ui.shots.emptyTitle")}</h2>
              <p>{t("ui.shots.emptyBody")}</p>
            </div>
          )
        ) : (
          groups.map((g) => {
            const blobs = g.shots.map((s) => s.blob);
            const n = blobs.filter((b) => picked.has(b)).length;
            const day = g.day ? new Intl.DateTimeFormat(i18n.language, { dateStyle: "full" }).format(new Date(g.shots[0].takenAt)) : "—";
            return (
              <section key={g.day} className="shot-day">
                <h3 className="num">
                  <label className="day-pick">
                    <input
                      type="checkbox"
                      className="check"
                      checked={n === blobs.length}
                      ref={(el) => {
                        if (el) el.indeterminate = n > 0 && n < blobs.length;
                      }}
                      onChange={() => toggleDay(blobs)}
                      aria-label={t("ui.shots.pickDay", { day })}
                    />
                    {day}
                  </label>
                  <span className="muted">{t("ui.shots.dayCount", { n: blobs.length })}</span>
                </h3>
                <ul className="shot-grid">
                  {g.shots.map((s) => {
                    const i = ordered.indexOf(s);
                    const on = picked.has(s.blob);
                    const name = fileName(s.originalPath);
                    return (
                      <li key={s.blob} className={on ? "on" : undefined}>
                        <button className="shot-open" onClick={() => (inTrash ? toggle(s.blob) : setViewing(i))} title={inTrash ? name : t("ui.shots.thumbHint", { name })}>
                          <ShotThumb gameId={gameId} blob={s.blob} alt={name} />
                        </button>
                        <input type="checkbox" className="check shot-pick" checked={on} onChange={() => toggle(s.blob)} aria-label={name} />
                        <span className="shot-time num">{new Date(s.takenAt).toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" })}</span>
                      </li>
                    );
                  })}
                </ul>
              </section>
            );
          })
        )}
      </section>

      {purgeAsk && <PurgeDialog text={purgeAsk.text} onConfirm={purgeAsk.run} onClose={() => setPurgeAsk(null)} />}
      {diffOpen && (
        <ShotDiffDialog
          gameId={gameId}
          gameName={gameName}
          onClose={() => setDiffOpen(false)}
          onChanged={() => {
            void reload();
            onChanged();
          }}
        />
      )}
      {!inTrash && viewing !== null && ordered[viewing] && (
        <Lightbox
          gameId={gameId}
          shot={ordered[viewing]}
          index={viewing}
          total={ordered.length}
          snapshot={nodes.find((n) => n.id === snapOf.get(ordered[viewing].blob)) ?? null}
          onPrev={() => setViewing((v) => (v === null ? v : Math.max(0, v - 1)))}
          onNext={() => setViewing((v) => (v === null ? v : Math.min(ordered.length - 1, v + 1)))}
          onClose={() => setViewing(null)}
          onExport={() => void exportShots([ordered[viewing].blob], "folder")}
          onDelete={() => void remove([ordered[viewing].blob])}
          onShowSnapshot={(id) => {
            setViewing(null);
            onShowSnapshot(id);
          }}
        />
      )}
    </div>
  );
}

const INFO_KEY = "bonfire.shots.info";

function readInfoPref(): boolean {
  try {
    return prefGet(INFO_KEY) === "1";
  } catch {
    return false;
  }
}

/**
 * 大图（详情页截图与图库共用）：图片尽量占满窗口，底部一行精简信息与操作；详情（来源、尺寸、原位置、之后的存档）可展开，
 * 默认收起并记住上次的选择。← / → 切换，I 展开详情，Esc 关闭。
 */
export function Lightbox({
  gameId,
  title,
  shot,
  index,
  total,
  snapshot,
  onPrev,
  onNext,
  onClose,
  onExport,
  onDelete,
  onShowSnapshot,
}: {
  gameId: string;
  /** 图库中显示游戏名。 */
  title?: string;
  shot: ShotRecord;
  index: number;
  total: number;
  /** 拍摄之后的存档；undefined 时不显示这一项（图库）。 */
  snapshot?: SnapshotNode | null;
  onPrev: () => void;
  onNext: () => void;
  onClose: () => void;
  onExport?: () => void;
  onDelete: () => void;
  onShowSnapshot?: (id: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const [path, setPath] = useState<string | null>(null);
  const [info, setInfo] = useState(readInfoPref);
  const toggleInfo = useCallback(() => {
    setInfo((v) => {
      try {
        prefSet(INFO_KEY, v ? "0" : "1");
      } catch {
        // 无法保存偏好时只影响本次。
      }
      return !v;
    });
  }, []);
  useEffect(() => {
    let off = false;
    setPath(null);
    api
      .shotImage(gameId, shot.blob)
      .then((p) => !off && setPath(p))
      .catch(() => {});
    return () => {
      off = true;
    };
  }, [gameId, shot.blob]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowLeft") onPrev();
      else if (e.key === "ArrowRight") onNext();
      else if (e.key === "i" || e.key === "I") toggleInfo();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, onPrev, onNext, toggleInfo]);
  const name = fileName(shot.originalPath);
  const src = imageSrc(path);

  return (
    <div className="lightbox" role="dialog" aria-modal="true" aria-label={name} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <button className="lb-nav prev" onClick={onPrev} disabled={index === 0} aria-label={t("ui.shots.prev")}>
        ‹
      </button>
      <figure>
        <div className="lb-image" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
          {src ? <img src={src} alt={name} /> : <span className="muted">{t("ui.loading")}</span>}
        </div>
        <figcaption>
          {info && (
            <dl className="lb-info">
              <div>
                <dt>{t("ui.shots.sources")}</dt>
                <dd>{t(`ui.shots.source.${shot.source}`)}</dd>
              </div>
              {shot.width && shot.height && (
                <div>
                  <dt>{t("ui.shots.size")}</dt>
                  <dd className="num">
                    {shot.width} × {shot.height} · {bytes(shot.size)}
                  </dd>
                </div>
              )}
              <div className="wide">
                <dt>{t("ui.shots.original")}</dt>
                <dd>
                  <button className="link path" onClick={() => void api.openPath(shot.originalPath)} title={t("ui.shots.openOriginal")}>
                    {shot.originalPath}
                  </button>
                </dd>
              </div>
              {snapshot !== undefined && (
              <div className="wide">
                <dt>{t("ui.shots.savedIn")}</dt>
                <dd>
                  {snapshot ? (
                    <button className="link" onClick={() => onShowSnapshot?.(snapshot.id)} title={t("ui.shots.showSave")}>
                      {dateTime(snapshot.createdAt, i18n.language)} · {firstLine(snapshot.comment) || t("ui.history.noComment")}
                    </button>
                  ) : (
                    <span className="muted">{t("ui.shots.noSaveYet")}</span>
                  )}
                </dd>
              </div>
              )}
            </dl>
          )}
          <div className="lb-bar">
            {title && <span className="lb-game">{title}</span>}
            <b className="lb-name" title={name}>
              {name}
            </b>
            <span className="num muted">{dateTime(shot.takenAt, i18n.language)}</span>
            <span className="num muted">
              {index + 1} / {total}
            </span>
            <div className="grow" />
            <button className="btn small ghost" onClick={toggleInfo} aria-pressed={info} title={t("ui.shots.infoKey")}>
              <Icon name="info" size={14} /> {t("ui.shots.info")}
            </button>
            <button className="btn small ghost" onClick={() => path && void api.openPath(path)} disabled={!path} title={t("ui.shots.revealHint")}>
              <Icon name="folder" size={14} /> {t("ui.shots.reveal")}
            </button>
            {onExport && (
              <button className="btn small ghost" onClick={onExport}>
                <Icon name="export" size={14} /> {t("ui.shots.export")}
              </button>
            )}
            <button className="btn small ghost" onClick={onDelete}>
              <Icon name="trash" size={14} /> {t("ui.shots.delete")}
            </button>
            <button className="btn small" onClick={onClose} aria-label={t("ui.shots.close")}>
              <Icon name="close" size={14} />
            </button>
          </div>
        </figcaption>
      </figure>
      <button className="lb-nav next" onClick={onNext} disabled={index === total - 1} aria-label={t("ui.shots.next")}>
        ›
      </button>
    </div>
  );
}
