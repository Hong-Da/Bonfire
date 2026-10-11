import { useState } from "react";
import { NotBonfireHint } from "./DownloadImportDialog";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { Cover, Icon, Modal, displayName } from "../components/ui";
import ExportDone, { FileBadge, type ExportedFile } from "../components/ExportDone";
import { dateTime, shortId } from "../lib/format";
import { useStore } from "../store";
import type { ArchiveImportItem, ArchivePreview, BundlePreview, FilesSource, GameRow } from "../types";
import { MAX_COMMENT } from "./BackupDialog";

/** 导出格式（U-49）：篝火包（完整历史）或篝火的 zip（最新存档文件与截图原图）。 */
type PackageFormat = "bonfire" | "zip";

/** 导出文件名中的时间：年月日-时分（两个文件分开导出时避免与已有文件重名）。 */
const stampOf = (withTime: boolean) => {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  const day = `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}`;
  return withTime ? `${day}-${p(d.getHours())}${p(d.getMinutes())}` : day;
};

/** 在文件夹路径后接文件名（沿用路径中的分隔符）。 */
const joinPath = (dir: string, name: string) => {
  const sep = dir.includes("\\") ? "\\" : "/";
  return dir.endsWith(sep) ? dir + name : dir + sep + name;
};

/**
 * 导出存档和截图（U-16、U-49）：一个或多个游戏，选择内容（存档 / 截图）、格式（篝火包 / zip），
 * 同时导出存档与截图时可合成一个文件或分成两个文件。没有可导出内容的游戏不能选。
 * 游戏库、详情页共用；图库中选中的截图直接导出（见图库页）。导出后换成导出完成的页面。
 */
export function CollectionExportDialog({ rows, initial, onClose }: { rows: GameRow[]; initial: string[]; onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [saves, setSaves] = useState(true);
  const [shots, setShots] = useState(true);
  const [format, setFormat] = useState<PackageFormat>("bonfire");
  const [split, setSplit] = useState(false);
  const has = (r: GameRow) => (saves && r.snapshots > 0) || (shots && r.shots > 0);
  const [picked, setPicked] = useState<Set<string>>(() => new Set(initial));
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<{ files: ExportedFile[]; skipped: string[] } | null>(null);
  const chosen = rows.filter((r) => picked.has(r.id) && has(r));
  const exportable = rows.filter(has);
  const snapshots = saves ? chosen.reduce((n, r) => n + r.snapshots, 0) : 0;
  const latest = saves ? chosen.filter((r) => r.snapshots > 0).length : 0;
  const shotCount = shots ? chosen.reduce((n, r) => n + r.shots, 0) : 0;
  const both = saves && shots;
  // 两个文件：只在同时导出存档与截图、且两样都有内容时。
  const twoFiles = both && split && latest > 0 && shotCount > 0;
  const ext = format === "zip" ? "zip" : "bonfire";

  const toggle = (id: string) =>
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  /** 导出一个文件，返回导出结果的说明。 */
  const exportOne = async (dest: string, part: { saves: boolean; shots: boolean }): Promise<{ file: ExportedFile; skipped: string[] }> => {
    const ids = chosen.filter((r) => (part.saves && r.snapshots > 0) || (part.shots && r.shots > 0)).map((x) => x.id);
    const r = format === "zip" ? await api.exportPackageZip(ids, dest, part) : await api.exportPackage(ids, dest, part);
    const kind = part.saves && part.shots ? "both" : part.saves ? "saves" : "shots";
    const label = [t(`ui.export.kind.${kind}`), t("ui.export.gamesN", { n: r.games.length })].join(" · ");
    return { file: { path: r.dest, bytes: r.bytes, label }, skipped: r.skipped };
  };

  const run = async () => {
    const base = chosen.length === 1 ? displayName(chosen[0], i18n.language) : t("ui.collection.fileName");
    let jobs: { dest: string; part: { saves: boolean; shots: boolean } }[];
    if (twoFiles) {
      const dir = await open({ directory: true, title: t("ui.package.pickFolder") });
      if (!dir || Array.isArray(dir)) return;
      const stamp = stampOf(true);
      jobs = [
        { dest: joinPath(dir, `Bonfire-${base}-${t("ui.export.kind.saves")}-${stamp}.${ext}`), part: { saves: true, shots: false } },
        { dest: joinPath(dir, `Bonfire-${base}-${t("ui.export.kind.shots")}-${stamp}.${ext}`), part: { saves: false, shots: true } },
      ];
    } else {
      const dest = await save({
        defaultPath: `Bonfire-${base}-${stampOf(false)}.${ext}`,
        filters: [format === "zip" ? { name: "Zip", extensions: ["zip"] } : { name: t("ui.transfer.bundle"), extensions: ["bonfire"] }],
      });
      if (!dest) return;
      jobs = [{ dest, part: { saves, shots } }];
    }
    setBusy(true);
    const files: ExportedFile[] = [];
    const skipped = new Set<string>();
    try {
      for (const j of jobs) {
        const r = await exportOne(j.dest, j.part);
        files.push(r.file);
        r.skipped.forEach((g) => skipped.add(g));
      }
      setDone({ files, skipped: [...skipped] });
    } catch (e) {
      store.toast("error", errorMessage(e));
      if (files.length > 0) setDone({ files, skipped: [...skipped] });
    } finally {
      setBusy(false);
    }
  };

  const footer = done ? (
    <button className="btn primary" onClick={onClose}>
      {t("ui.done")}
    </button>
  ) : (
    <>
      <span className="count num">
        <span>{format === "zip" ? t("ui.package.summaryZip", { n: chosen.length, s: latest, p: shotCount }) : t("ui.package.summary", { n: chosen.length, s: snapshots, p: shotCount })}</span>
        {chosen.length > 0 && <span className="muted"> → {t("ui.package.fileCount", { n: twoFiles ? 2 : 1 })}</span>}
      </span>
      <div className="grow" />
      <button className="btn ghost" onClick={onClose}>
        {t("ui.cancel")}
      </button>
      <button className="btn primary" onClick={() => void run()} disabled={busy || chosen.length === 0}>
        <Icon name="export" /> {busy ? t("ui.export.working") : twoFiles ? t("ui.package.exportToFolder") : t("ui.transfer.exportTo")}
      </button>
    </>
  );

  if (done) {
    return (
      <Modal title={t("ui.export.doneTitle")} onClose={onClose} footer={footer}>
        <ExportDone files={done.files} skipped={done.skipped} hint={t(format === "zip" ? "ui.export.nextZip" : "ui.export.nextBonfire")} />
      </Modal>
    );
  }

  return (
    <Modal title={t("ui.package.exportTitle")} subtitle={t("ui.package.exportIntro")} onClose={onClose} footer={footer}>
      <div className="export-opts">
        <span className="export-opt-label" id="pkg-parts">
          {t("ui.package.parts")}
        </span>
        <div className="package-parts" role="group" aria-labelledby="pkg-parts">
          <label className="radio">
            <input type="checkbox" checked={saves} onChange={(e) => setSaves(e.target.checked)} />
            <Icon name="backup" size={14} /> {t(format === "zip" ? "ui.package.savesZip" : "ui.package.saves")}
          </label>
          <label className="radio">
            <input type="checkbox" checked={shots} onChange={(e) => setShots(e.target.checked)} />
            <Icon name="camera" size={14} /> {t(format === "zip" ? "ui.package.shotsZip" : "ui.package.shots")}
          </label>
        </div>

        <span className="export-opt-label" id="pkg-format">
          {t("ui.package.format")}
        </span>
        <div className="format-tiles" role="radiogroup" aria-labelledby="pkg-format">
          {(["bonfire", "zip"] as const).map((f) => (
            <button key={f} type="button" role="radio" aria-checked={format === f} className="format-tile" onClick={() => setFormat(f)}>
              <FileBadge ext={f} />
              <span>
                <b>{t(f === "zip" ? "ui.package.formatZip" : "ui.package.formatBonfire")}</b>
                <small>{t(f === "zip" ? "ui.package.formatZipHint" : "ui.package.formatBonfireHint")}</small>
              </span>
            </button>
          ))}
        </div>

        {both && (
          <>
            <span className="export-opt-label" id="pkg-files">
              {t("ui.package.files")}
            </span>
            <div>
              <div className="seg" role="radiogroup" aria-labelledby="pkg-files">
                <button type="button" role="radio" aria-checked={!split} onClick={() => setSplit(false)}>
                  {t("ui.package.oneFile")}
                </button>
                <button type="button" role="radio" aria-checked={split} onClick={() => setSplit(true)}>
                  {t("ui.package.twoFiles")}
                </button>
              </div>
              {split && <p className="hint">{t("ui.package.twoFilesHint")}</p>}
            </div>
          </>
        )}
      </div>
      <div className="pick-head">
        <label className="radio">
          <input
            type="checkbox"
            checked={chosen.length === exportable.length && exportable.length > 0}
            ref={(el) => {
              if (el) el.indeterminate = chosen.length > 0 && chosen.length < exportable.length;
            }}
            onChange={() => setPicked(chosen.length === exportable.length ? new Set() : new Set(exportable.map((r) => r.id)))}
          />
          {t("ui.library.selectAll")}
        </label>
      </div>
      <ul className="pick-list">
        {rows.map((r) => {
          const disabled = !has(r);
          const savesText = format === "zip" ? t("ui.package.latestSave") : t("ui.library.snapshots", { n: r.snapshots });
          return (
            <li key={r.id} className={disabled ? "disabled" : picked.has(r.id) ? "on" : undefined}>
              <label>
                <input type="checkbox" checked={picked.has(r.id) && !disabled} disabled={disabled} onChange={() => toggle(r.id)} />
                <Cover row={r} />
                <span className="pick-name">
                  <b>{displayName(r, i18n.language)}</b>
                  <small>{t(`ui.platform.${r.platform}`, { defaultValue: r.platform })}</small>
                </span>
                <span className="muted num">
                  {disabled ? t("ui.package.nothing") : [saves && r.snapshots > 0 && savesText, shots && r.shots > 0 && t("ui.shots.countN", { n: r.shots })].filter(Boolean).join(" · ")}
                </span>
              </label>
            </li>
          );
        })}
      </ul>
    </Modal>
  );
}

type Loaded =
  | { kind: "archive"; path: string; preview: ArchivePreview }
  | { kind: "files"; path: string; source: FilesSource }
  // zip 格式的篝火导出中有多个游戏的存档（U-49）：逐个勾选导入。
  | { kind: "multi"; path: string; sources: FilesSource[] };

/** 存档文件能否导入（与本地快照相同或有对不上的文件时不能）。 */
const filesImportable = (s: FilesSource) => !s.preview?.identical && (s.preview?.unmapped.length ?? 0) === 0;

/** 合集包中的游戏能否导入、导入后有没有变化。 */
function importable(p: BundlePreview): boolean {
  if (p.missing.length > 0) return false;
  return p.createGame || p.newSnapshots > 0 || (p.newShots ?? 0) > 0 || p.refs.some((r) => r.action === "create" || r.action === "fast-forward" || r.action === "diverged");
}

/**
 * 在游戏库中导入：
 * - 历史包或合集包（.bonfire）：列出包中的游戏，勾选要导入的；
 * - 别人分享的存档文件（篝火导出的 zip 或文件夹）：按附带的说明找到游戏。
 * 游戏库中没有的游戏自动加入，标为“未安装”。导入只写入仓库，不改动存档目录。
 */
export function CollectionImportDialog({ onClose, onDone, onUseDownload }: { onClose: () => void; onDone: () => void; onUseDownload?: (path: string) => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  // 选到不是篝火导出的存档文件时记下路径，提示改用“导入下载的存档”。
  const [notBonfire, setNotBonfire] = useState<string | null>(null);
  const [modified, setModified] = useState(false);
  const [busy, setBusy] = useState(false);
  const [results, setResults] = useState<ArchiveImportItem[] | null>(null);
  const [filesDone, setFilesDone] = useState<string | null>(null);
  // 多个游戏的存档文件：每个存档文件夹的导入结果。
  const [multiDone, setMultiDone] = useState<Record<string, { gameId?: string; error?: string }> | null>(null);
  const [comment, setComment] = useState("");
  // 每个游戏导入哪些内容（默认包内有的都导入），以及是否把新导入的截图写到磁盘（默认不写）。
  const [parts, setParts] = useState<Record<string, { saves: boolean; shots: boolean }>>({});
  const [toDisk, setToDisk] = useState(false);
  // 包内的 main 与本机不同时：默认作为新路线加入；“这是我旧电脑的存档”必须手动选（U-41h）。
  const [adopt, setAdopt] = useState<Record<string, boolean>>({});
  const hasSaves = (p: BundlePreview) => p.nodes.length > 0 || p.newSnapshots > 0 || p.refs.length > 0;
  const partsOf = (key: string, p: BundlePreview) => parts[key] ?? { saves: hasSaves(p), shots: (p.shots ?? 0) > 0 };

  const choose = async (what: "archive" | "zip" | "folder") => {
    setError(null);
    setNotBonfire(null);
    setResults(null);
    setFilesDone(null);
    setMultiDone(null);
    const path =
      what === "folder"
        ? await open({ directory: true, title: t("ui.transfer.pickFolder") })
        : await open({ filters: what === "archive" ? [{ name: t("ui.transfer.bundle"), extensions: ["bonfire"] }] : [{ name: "Zip", extensions: ["zip"] }] });
    if (!path || Array.isArray(path)) return;
    setBusy(true);
    try {
      if (what === "archive") {
        const preview = await api.inspectArchive(path);
        setLoaded({ kind: "archive", path, preview });
        setPicked(new Set(preview.games.filter((g) => importable(g.preview)).map((g) => g.key)));
      } else {
        const sources = await api.inspectFilesSources(path);
        if (sources.length === 1) setLoaded({ kind: "files", path, source: sources[0] });
        else {
          setLoaded({ kind: "multi", path, sources });
          setPicked(new Set(sources.filter(filesImportable).map((s) => s.dir)));
        }
      }
    } catch (e) {
      setLoaded(null);
      const msg = errorMessage(e);
      const isModified = msg.includes("校验不通过");
      if (what !== "archive" && (isModified || msg.includes("没有篝火的导出说明"))) {
        setNotBonfire(path);
        setModified(isModified);
      } else setError(msg);
    } finally {
      setBusy(false);
    }
  };

  const toggle = (key: string) =>
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(key)) n.delete(key);
      else n.add(key);
      return n;
    });

  const run = async () => {
    if (!loaded) return;
    setBusy(true);
    try {
      if (loaded.kind === "archive") {
        const items = await api.importArchive(
          loaded.path,
          [...picked].map((key) => {
            const g = loaded.preview.games.find((x) => x.key === key)!;
            return { key, target: null, ...partsOf(key, g.preview), adoptMain: !!adopt[key] && !!g.preview.mainConflict };
          }),
        );
        setResults(items);
        // 以旧电脑的路线为本机主存档的游戏：恢复到它的最新快照（恢复前自动做保护快照）。
        for (const it of items.filter((x) => x.adoptedMain)) {
          try {
            await api.restore(it.gameId, it.adoptedMain!, [], true, false);
            store.toast("success", t("ui.primary.adoptRestored", { name: it.gameName }));
          } catch (e) {
            store.toast("error", t("ui.primary.adoptRestoreFailed", { name: it.gameName, msg: errorMessage(e) }));
          }
        }
        // 把新导入的截图写到本机这个游戏的截图文件夹。
        if (toDisk) {
          let written = 0;
          let skipped = 0;
          for (const it of items) {
            const ids = it.report?.newShotIds ?? [];
            if (ids.length === 0) continue;
            try {
              const w = await api.syncShotsToDisk(it.report!.preview.gameId, ids);
              written += w.written;
              skipped += w.exists + w.noDir;
            } catch (e) {
              store.toast("error", `${it.gameName}：${errorMessage(e)}`);
            }
          }
          if (written + skipped > 0) store.toast(written > 0 ? "success" : "info", t("ui.package.toDiskDone", { n: written, skipped }));
        }
        const failed = items.filter((i) => i.error).length;
        if (failed > 0) store.toast("error", t("ui.collection.importFailed", { n: failed }));
        else store.toast("success", t("ui.collection.importDone", { ok: items.length }));
      } else if (loaded.kind === "multi") {
        const out: Record<string, { gameId?: string; error?: string }> = {};
        for (const dir of [...picked]) {
          try {
            out[dir] = { gameId: (await api.importFilesAuto(loaded.path, dir, comment.trim() || null)).gameId };
          } catch (e) {
            out[dir] = { error: errorMessage(e) };
          }
        }
        setMultiDone(out);
        const failed = Object.values(out).filter((x) => x.error).length;
        if (failed > 0) store.toast("error", t("ui.collection.importFailed", { n: failed }));
        else store.toast("success", t("ui.collection.importDone", { ok: Object.keys(out).length }));
      } else {
        const r = await api.importFilesAuto(loaded.path, loaded.source.dir || null, comment.trim() || null);
        setFilesDone(r.gameId);
        store.toast("success", t("ui.collection.filesDone", { name: r.gameName }));
      }
      onDone();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const finished = results !== null || filesDone !== null || multiDone !== null;
  const canImport =
    !busy &&
    !!loaded &&
    !finished &&
    (loaded.kind === "archive"
      ? picked.size > 0 && [...picked].every((k) => { const g = loaded.preview.games.find((x) => x.key === k); return !g || partsOf(k, g.preview).saves || partsOf(k, g.preview).shots; })
      : loaded.kind === "multi"
        ? picked.size > 0 && [...comment].length <= MAX_COMMENT
        : filesImportable(loaded.source) && [...comment].length <= MAX_COMMENT);

  const footer = finished ? (
    <>
      {filesDone && (
        <button
          className="btn"
          onClick={() => {
            onClose();
            store.go({ name: "game", id: filesDone });
          }}
        >
          {t("ui.collection.openGame")}
        </button>
      )}
      <button className="btn primary" onClick={onClose}>
        {t("ui.done")}
      </button>
    </>
  ) : (
    <>
      <button className="btn ghost" onClick={onClose}>
        {t("ui.cancel")}
      </button>
      <button className="btn primary" onClick={() => void run()} disabled={!canImport}>
        <Icon name="import" /> {loaded?.kind === "archive" || loaded?.kind === "multi" ? t("ui.collection.importPicked", { n: picked.size }) : t("ui.transfer.import")}
      </button>
    </>
  );

  const resultOf = (key: string) => results?.find((r) => r.key === key);

  return (
    <Modal title={t("ui.collection.importTitle")} subtitle={t("ui.collection.importIntro")} onClose={onClose} footer={footer}>
      <div className="collection-import">
        <div className={`source-picks${loaded ? " compact" : ""}`}>
          {(
            [
              ["archive", "branch", "ui.collection.chooseArchive", "ui.collection.chooseArchiveHint"],
              ["zip", "import", "ui.transfer.chooseZip", "ui.collection.chooseZipHint"],
              ["folder", "folder", "ui.transfer.chooseFolder", "ui.collection.chooseFolderHint"],
            ] as const
          ).map(([what, icon, key, hint]) => (
            <button key={what} className="source-pick" onClick={() => void choose(what)} disabled={busy}>
              <Icon name={icon} />
              <span>
                <b>{t(key)}</b>
                {!loaded && <small>{t(hint)}</small>}
              </span>
            </button>
          ))}
        </div>
        {error && <div className="callout error">{error}</div>}
        {notBonfire && onUseDownload && (
          <NotBonfireHint
            modified={modified}
            onUse={() => {
              onUseDownload(notBonfire);
              onClose();
            }}
          />
        )}
        {loaded && <p className="source-path" title={loaded.path}>{loaded.path}</p>}

        {loaded?.kind === "archive" && (
          <>
            <p className="hint">
              {t("ui.transfer.bundleFrom", { host: loaded.preview.host, time: dateTime(loaded.preview.createdAt, i18n.language) })}
            </p>
            <div className="pick-head">
              <span className="muted">{t("ui.collection.games", { n: loaded.preview.games.length })}</span>
            </div>
            <ul className="pick-list">
              {loaded.preview.games.map(({ key, preview: p }) => {
                const ok = importable(p);
                const res = resultOf(key);
                return (
                  <li key={key} className={!ok ? "disabled" : picked.has(key) ? "on" : undefined}>
                    <label>
                      <input type="checkbox" checked={picked.has(key)} disabled={!ok || finished} onChange={() => toggle(key)} />
                      <span className="pick-name">
                        <b>{p.gameName}</b>
                        <small>
                          {p.createGame ? <span className="tag found-new">{t("ui.collection.stateNew")}</span> : <span className="tag found-managed">{t("ui.collection.stateLocal")}</span>}
                        </small>
                      </span>
                      <span className="package-parts compact" onClick={(e) => e.preventDefault()}>
                        {hasSaves(p) && (
                          <label className="radio" onClick={(e) => e.stopPropagation()}>
                            <input
                              type="checkbox"
                              checked={partsOf(key, p).saves}
                              disabled={finished}
                              onChange={(e) => setParts((x) => ({ ...x, [key]: { ...partsOf(key, p), saves: e.target.checked } }))}
                              aria-label={t("ui.package.importSavesOf", { name: p.gameName })}
                            />
                            {t("ui.package.savesN", { n: Math.max(p.nodes.length, p.newSnapshots) })}
                          </label>
                        )}
                        {(p.shots ?? 0) > 0 && (
                          <label className="radio" onClick={(e) => e.stopPropagation()}>
                            <input
                              type="checkbox"
                              checked={partsOf(key, p).shots}
                              disabled={finished}
                              onChange={(e) => setParts((x) => ({ ...x, [key]: { ...partsOf(key, p), shots: e.target.checked } }))}
                              aria-label={t("ui.package.importShotsOf", { name: p.gameName })}
                            />
                            {t("ui.package.shotsN", { n: p.shots, fresh: p.newShots })}
                          </label>
                        )}
                      </span>
                      {p.mainConflict && hasSaves(p) && partsOf(key, p).saves && (
                        <span className="main-conflict" onClick={(e) => e.preventDefault()} role="radiogroup" aria-label={t("ui.primary.conflict", { name: p.mainConflict })}>
                          <small>{t("ui.primary.conflict", { name: p.mainConflict })}</small>
                          <label className="radio" onClick={(e) => e.stopPropagation()}>
                            <input type="radio" name={`adopt-${key}`} checked={!adopt[key]} disabled={finished} onChange={() => setAdopt((x) => ({ ...x, [key]: false }))} />
                            {t("ui.primary.asOther")}
                          </label>
                          <label className="radio" onClick={(e) => e.stopPropagation()}>
                            <input type="radio" name={`adopt-${key}`} checked={!!adopt[key]} disabled={finished} onChange={() => setAdopt((x) => ({ ...x, [key]: true }))} />
                            {t("ui.primary.asMine")}
                          </label>
                        </span>
                      )}
                      <span className="pick-state num">
                        {res ? (
                          res.error ? (
                            <span className="danger-text" title={res.error}>
                              {res.error}
                            </span>
                          ) : (
                            <span className="ok-text">
                              <Icon name="check" size={14} /> {t("ui.collection.resultOk", { n: res.report?.importedSnapshots ?? 0 })}
                              {(res.report?.importedShots ?? 0) > 0 && ` · ${t("ui.package.resultShots", { n: res.report?.importedShots })}`}
                              {res.report?.createdGame && ` · ${t("ui.collection.resultAdded")}`}
                            </span>
                          )
                        ) : p.missing.length > 0 ? (
                          <span className="muted">{t("ui.collection.missing")}</span>
                        ) : p.newSnapshots > 0 ? (
                          t("ui.collection.newSnapshots", { n: p.newSnapshots })
                        ) : (
                          <span className="muted">{t("ui.collection.upToDate")}</span>
                        )}
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
            {loaded.preview.games.some((g) => (g.preview.newShots ?? 0) > 0) && (
              <label className="purge-originals to-disk">
                <input type="checkbox" className="check" checked={toDisk} disabled={finished} onChange={(e) => setToDisk(e.target.checked)} />
                <span>
                  <b>{t("ui.package.toDisk")}</b>
                  <small>{t("ui.package.toDiskHint")}</small>
                </span>
              </label>
            )}
          </>
        )}

        {loaded?.kind === "files" && (
          <div className="files-target">
            <p>
              <b>{t("ui.collection.filesFor", { name: loaded.source.manifest.gameName })}</b>
            </p>
            <p className="hint">
              {t("ui.transfer.bundleFrom", { host: loaded.source.manifest.host, time: dateTime(loaded.source.manifest.exportedAt, i18n.language) })}
              {loaded.source.manifest.snapshot && ` · ${shortId(loaded.source.manifest.snapshot)}`}
            </p>
            {loaded.source.preview?.identical ? (
              <div className="callout same">
                <p>{t("ui.collection.identical", { name: loaded.source.manifest.gameName, id: shortId(loaded.source.preview.identical) })}</p>
              </div>
            ) : (
              <p className="hint">{t(loaded.source.gameId ? "ui.collection.filesExisting" : "ui.collection.filesNewGame")}</p>
            )}
            {!loaded.source.preview?.identical && !filesDone && (
              <input className="field" value={comment} onChange={(e) => setComment(e.target.value)} placeholder={t("ui.transfer.commentPlaceholder")} aria-label={t("ui.history.comment")} />
            )}
          </div>
        )}

        {loaded?.kind === "multi" && (
          <>
            <p className="hint">
              {t("ui.transfer.bundleFrom", { host: loaded.sources[0].manifest.host, time: dateTime(loaded.sources[0].manifest.exportedAt, i18n.language) })}
            </p>
            <div className="pick-head">
              <span className="muted">{t("ui.collection.filesGames", { n: loaded.sources.length })}</span>
            </div>
            <ul className="pick-list">
              {loaded.sources.map((s) => {
                const ok = filesImportable(s);
                const res = multiDone?.[s.dir];
                return (
                  <li key={s.dir} className={!ok ? "disabled" : picked.has(s.dir) ? "on" : undefined}>
                    <label>
                      <input type="checkbox" checked={picked.has(s.dir)} disabled={!ok || finished} onChange={() => toggle(s.dir)} aria-label={s.manifest.gameName} />
                      <span className="pick-name">
                        <b>{s.manifest.gameName}</b>
                        <small>
                          {s.gameId ? <span className="tag found-managed">{t("ui.collection.stateLocal")}</span> : <span className="tag found-new">{t("ui.collection.stateNew")}</span>}
                          <span className="muted num"> {s.dir}/</span>
                        </small>
                      </span>
                      <span className="pick-state num">
                        {res ? (
                          res.error ? (
                            <span className="danger-text" title={res.error}>
                              {res.error}
                            </span>
                          ) : (
                            <span className="ok-text">
                              <Icon name="check" size={14} /> {t("ui.collection.filesImported")}
                            </span>
                          )
                        ) : s.preview?.identical ? (
                          <span className="muted">{t("ui.collection.upToDate")}</span>
                        ) : (s.preview?.unmapped.length ?? 0) > 0 ? (
                          <span className="muted">{t("ui.collection.filesUnmapped", { n: s.preview!.unmapped.length })}</span>
                        ) : (
                          t(s.gameId ? "ui.collection.filesNewBranch" : "ui.collection.filesWillAdd")
                        )}
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
            {!finished && <input className="field" value={comment} onChange={(e) => setComment(e.target.value)} placeholder={t("ui.transfer.commentPlaceholder")} aria-label={t("ui.history.comment")} />}
          </>
        )}
      </div>
    </Modal>
  );
}
