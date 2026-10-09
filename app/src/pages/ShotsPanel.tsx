import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, imageSrc } from "../api";
import { Icon, Menu } from "../components/ui";
import { bytes, dateTime, firstLine } from "../lib/format";
import { fileName, groupByDay, shotSnapshots } from "../lib/shots";
import { useStore } from "../store";
import type { AlbumView, ExportFormat, ShotRecord, SnapshotNode } from "../types";

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

/**
 * 详情页“截图”标签页：按拍摄日期分组的相册，可多选导出 / 删除，点开看大图；
 * 顶部是这个游戏的截图开关、来源与截图文件夹。截图只读收集，删除只删备份。
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
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [viewing, setViewing] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);

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

  const groups = useMemo(() => groupByDay(album?.shots ?? []), [album]);
  const ordered = useMemo(() => groups.flatMap((g) => g.shots), [groups]);
  const snapOf = useMemo(() => shotSnapshots(nodes, album?.shots ?? []), [nodes, album]);

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

  const sync = () =>
    run(async () => {
      const [r] = await api.syncShots(gameId);
      store.toast("success", r && r.added > 0 ? t("ui.shots.synced", { n: r.added }) : t("ui.shots.syncedNone"));
      await reload();
      onChanged();
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

  const remove = (blobs: string[]) =>
    run(async () => {
      if (!(await ask(t("ui.shots.deleteConfirm", { n: blobs.length }), { kind: "warning" }))) return;
      const n = await api.deleteShots(gameId, blobs);
      store.toast("success", t("ui.shots.deleted", { n }));
      setPicked(new Set());
      setViewing(null);
      await reload();
      onChanged();
    });

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
  const sel = [...picked];
  const switchValue = album.followsGlobal ? "auto" : album.enabled ? "on" : "off";

  return (
    <div className="shots">
      <aside className="shots-side">
        <h2>{t("ui.shots.title")}</h2>
        <p className="muted num">{t("ui.shots.summary", { n: album.shots.length, size: bytes(album.bytes) })}</p>
        {album.enabled && album.pending > 0 && <div className="callout same"><p>{t("ui.shots.pending", { n: album.pending })}</p></div>}
        <button className="btn primary" onClick={() => void sync()} disabled={busy || !album.enabled}>
          <Icon name="backup" /> {t("ui.shots.sync")}
        </button>

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
        {album.shots.length === 0 ? (
          <div className="empty">
            <h2>{t("ui.shots.emptyTitle")}</h2>
            <p>{t("ui.shots.emptyBody")}</p>
          </div>
        ) : (
          <>
            <div className="shots-bar">
              <button className="btn ghost small" onClick={() => setPicked(new Set(ordered.map((s) => s.blob)))}>
                {t("ui.shots.selectAll")}
              </button>
              <button className="btn ghost small" onClick={() => setPicked(new Set())} disabled={sel.length === 0}>
                {t("ui.shots.selectNone")}
              </button>
              <div className="grow" />
              <span className="count num">{t("ui.shots.selected", { n: sel.length })}</span>
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
            </div>
            {groups.map((g) => (
              <section key={g.day} className="shot-day">
                <h3 className="num">{g.day ? new Intl.DateTimeFormat(i18n.language, { dateStyle: "full" }).format(new Date(g.shots[0].takenAt)) : "—"}</h3>
                <ul className="shot-grid">
                  {g.shots.map((s) => {
                    const i = ordered.indexOf(s);
                    const on = picked.has(s.blob);
                    return (
                      <li key={s.blob} className={on ? "on" : undefined}>
                        <button className="shot-open" onClick={() => setViewing(i)} title={fileName(s.originalPath)}>
                          <ShotThumb gameId={gameId} blob={s.blob} alt={fileName(s.originalPath)} />
                        </button>
                        <input type="checkbox" className="check shot-pick" checked={on} onChange={() => toggle(s.blob)} aria-label={fileName(s.originalPath)} />
                        <span className="shot-time num">{new Date(s.takenAt).toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" })}</span>
                      </li>
                    );
                  })}
                </ul>
              </section>
            ))}
          </>
        )}
      </section>

      {viewing !== null && ordered[viewing] && (
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

/** 大图：← / → 切换，Esc 关闭；显示拍摄时间、来源、原位置与之后的存档。 */
function Lightbox({
  gameId,
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
  shot: ShotRecord;
  index: number;
  total: number;
  snapshot: SnapshotNode | null;
  onPrev: () => void;
  onNext: () => void;
  onClose: () => void;
  onExport: () => void;
  onDelete: () => void;
  onShowSnapshot: (id: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let off = false;
    setSrc(null);
    api
      .shotImage(gameId, shot.blob)
      .then((p) => !off && setSrc(imageSrc(p) ?? null))
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
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, onPrev, onNext]);

  return (
    <div className="lightbox" role="dialog" aria-modal="true" aria-label={fileName(shot.originalPath)} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <button className="lb-nav prev" onClick={onPrev} disabled={index === 0} aria-label={t("ui.shots.prev")}>
        ‹
      </button>
      <figure>
        <div className="lb-image">{src ? <img src={src} alt={fileName(shot.originalPath)} /> : <span className="muted">{t("ui.loading")}</span>}</div>
        <figcaption>
          <div className="lb-meta">
            <b>{fileName(shot.originalPath)}</b>
            <span className="num">
              {index + 1} / {total}
            </span>
          </div>
          <dl>
            <dt>{t("ui.shots.takenAt")}</dt>
            <dd className="num">{dateTime(shot.takenAt, i18n.language)}</dd>
            <dt>{t("ui.shots.sources")}</dt>
            <dd>{t(`ui.shots.source.${shot.source}`)}</dd>
            {shot.width && shot.height && (
              <>
                <dt>{t("ui.shots.size")}</dt>
                <dd className="num">
                  {shot.width} × {shot.height} · {bytes(shot.size)}
                </dd>
              </>
            )}
            <dt>{t("ui.shots.original")}</dt>
            <dd className="path" title={shot.originalPath}>
              {shot.originalPath}
            </dd>
            <dt>{t("ui.shots.savedIn")}</dt>
            <dd>
              {snapshot ? (
                <button className="link" onClick={() => onShowSnapshot(snapshot.id)} title={t("ui.shots.showSave")}>
                  {dateTime(snapshot.createdAt, i18n.language)} · {firstLine(snapshot.comment) || t("ui.history.noComment")}
                </button>
              ) : (
                <span className="muted">{t("ui.shots.noSaveYet")}</span>
              )}
            </dd>
          </dl>
          <div className="actions">
            <button className="btn small" onClick={onExport}>
              <Icon name="export" size={14} /> {t("ui.shots.export")}
            </button>
            <button className="btn small" onClick={onDelete}>
              <Icon name="trash" size={14} /> {t("ui.shots.delete")}
            </button>
            <div className="grow" />
            <button className="btn small primary" onClick={onClose}>
              {t("ui.shots.close")}
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
