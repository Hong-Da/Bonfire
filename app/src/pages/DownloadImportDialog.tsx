import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { Icon, Modal, displayName } from "../components/ui";
import { bytes } from "../lib/format";
import { useStore } from "../store";
import type { DownloadMode, DownloadPreview } from "../types";

type Step = "pick" | "map" | "finish";

/**
 * 导入下载的存档（U-42）：网上分享的存档大多是“手动把文件复制到存档目录”的方式，这里按步骤引导：
 * 1. 选择下载的 zip 或文件夹（加密 zip 输入密码；7z / rar 说明如何先解压；篝火导出的改用原来的导入）；
 * 2. 确认每个文件放在哪里（按文件名自动对应，对不上的手动指定或跳过；说明、图片已忽略）；
 * 3. 选择叠加或整体替换、路线名与来源备注，导入后可立即换上（先做保护快照，可撤销）。
 */
export default function DownloadImportDialog({
  gameId: fixedGame,
  initialPath,
  onClose,
  onDone,
  onUseBonfireImport,
}: {
  gameId?: string;
  initialPath?: string;
  onClose: () => void;
  /** `restore` 为导入后要换上的快照。 */
  onDone: (gameId: string, restore: string | null) => void;
  /** 篝火导出的存档：改用原来的导入。 */
  onUseBonfireImport?: (path: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const games = store.order.map((id) => store.rows[id]).filter((r) => r && !r.shotsOnly);
  const [gameId, setGameId] = useState<string>(fixedGame ?? games[0]?.id ?? "");
  const [path, setPath] = useState<string | null>(initialPath ?? null);
  const [password, setPassword] = useState("");
  const [preview, setPreview] = useState<DownloadPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [step, setStep] = useState<Step>("pick");
  // 每个文件放到哪里：`null` 为跳过。
  const [targets, setTargets] = useState<Record<string, string | null>>({});
  const [mode, setMode] = useState<DownloadMode>("overlay");
  // 会覆盖现有文件、但选了“不替换”的（不导入，叠加时保留你现有的那份）。
  const [keep, setKeep] = useState<Set<string>>(new Set());
  const [branch, setBranch] = useState(() => `下载-${new Date().toISOString().slice(0, 10).replaceAll("-", "")}`);
  const [comment, setComment] = useState("");
  const [useNow, setUseNow] = useState(true);
  const game = store.rows[gameId];

  const inspect = async (p: string, pw?: string) => {
    if (!gameId) return;
    setBusy(true);
    setError(null);
    try {
      const r = await api.inspectDownload(gameId, p, pw || null);
      setPreview(r);
      setTargets(Object.fromEntries(r.files.filter((f) => !f.ignored).map((f) => [f.rel, f.target ?? null])));
      setMode(r.mode);
      setComment((c) => c || r.sourceName.replace(/\.zip$/i, ""));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (initialPath && gameId) void inspect(initialPath);
  }, []);

  const choose = async (folder: boolean) => {
    const p = await open(folder ? { directory: true, title: t("ui.download.pickFolder") } : { filters: [{ name: t("ui.download.archives"), extensions: ["zip", "7z", "rar", "bonfire"] }] });
    if (!p || Array.isArray(p)) return;
    setPath(p);
    setPassword("");
    setPreview(null);
    await inspect(p);
  };

  const files = preview?.files ?? [];
  const saveFiles = files.filter((f) => !f.ignored);
  const ignored = files.filter((f) => f.ignored);
  const placed = saveFiles.filter((f) => targets[f.rel] && !keep.has(f.rel));
  const unplaced = saveFiles.filter((f) => !targets[f.rel]);
  const locs = preview?.locations ?? [];

  const setKeepOf = (rel: string, on: boolean) =>
    setKeep((k) => {
      const n = new Set(k);
      if (on) n.add(rel);
      else n.delete(rel);
      return n;
    });
  const existing = useMemo(() => new Set(preview?.existing ?? preview?.files.filter((f) => f.replaces && f.target).map((f) => f.target!.toLowerCase()) ?? []), [preview]);
  const existingHas = (target: string) => existing.has(target.toLowerCase());
  const overwrites = saveFiles.filter((f) => targets[f.rel] && existingHas(targets[f.rel]!));
  // 存档位置显示路径的最后两级（完整路径在悬停提示中）。
  const locLabel = (ord: number) => {
    const full = locs.find((l) => l.ord === ord)?.path;
    if (!full) return t("ui.download.location", { n: ord + 1 });
    const parts = full.split(/[\\/]/).filter(Boolean);
    return parts.length > 2 ? `…\\${parts.slice(-2).join("\\")}` : full;
  };
  const ready = preview?.kind === "download" && saveFiles.length > 0;

  const doImport = async () => {
    if (!path || !preview) return;
    setBusy(true);
    try {
      const r = await api.importDownload(gameId, path, password || null, {
        mapping: placed.map((f) => [f.rel, targets[f.rel]!] as [string, string]),
        mode,
        branch: branch.trim() || null,
        comment: comment.trim() || null,
      });
      if (r.identical) {
        store.toast("info", t("ui.download.identical"));
        onDone(gameId, useNow ? r.identical : null);
      } else {
        store.toast("success", t("ui.download.done", { n: r.files, branch: r.branch }));
        onDone(gameId, useNow ? r.snapshot : null);
      }
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const steps: Step[] = ["pick", "map", "finish"];
  const idx = steps.indexOf(step);
  const footer = (
    <>
      {idx > 0 && (
        <button className="btn step-prev" onClick={() => setStep(steps[idx - 1])} disabled={busy}>
          <span className="flip">
            <Icon name="next" size={14} />
          </span>{" "}
          {t("ui.backup.prev")}
        </button>
      )}
      <div className="grow" />
      <button className="btn ghost" onClick={onClose} disabled={busy}>
        {t("ui.cancel")}
      </button>
      {step === "finish" ? (
        <button className="btn primary" onClick={() => void doImport()} disabled={busy || placed.length === 0 || !branch.trim()}>
          <Icon name="import" size={16} /> {useNow ? t("ui.download.importUse") : t("ui.download.import")}
        </button>
      ) : (
        <button
          className="btn primary step-next"
          onClick={() => setStep(steps[idx + 1])}
          disabled={busy || (step === "pick" ? !ready : placed.length === 0)}
        >
          {t("ui.backup.next")} <Icon name="next" size={14} />
        </button>
      )}
    </>
  );

  return (
    <Modal wide title={t("ui.download.title")} subtitle={game ? t("ui.download.subtitle", { name: displayName(game, i18n.language) }) : undefined} onClose={busy ? () => {} : onClose} footer={footer}>
      <ol className="backup-steps" aria-label={t("ui.backup.steps")}>
        {steps.map((st, i) => (
          <li key={st} className={i === idx ? "current" : i < idx ? "done" : undefined} aria-current={i === idx ? "step" : undefined}>
            <span className="num">{i + 1}</span> {t(`ui.download.step.${st}`)}
          </li>
        ))}
      </ol>

      {step === "pick" && (
        <div className="dl-pick">
          {!fixedGame && (
            <div className="row">
              <label htmlFor="dl-game">{t("ui.download.game")}</label>
              <select id="dl-game" className="field" value={gameId} onChange={(e) => { setGameId(e.target.value); setPreview(null); setPath(null); }}>
                {games.map((g) => (
                  <option key={g.id} value={g.id}>
                    {displayName(g, i18n.language)}
                  </option>
                ))}
              </select>
            </div>
          )}
          <div className={`dl-drop${path ? " has" : ""}`}>
            <Icon name="import" size={26} />
            {path ? (
              <p className="dl-file" title={path}>
                <b>{path.split(/[\\/]/).pop()}</b>
                <small>{path}</small>
              </p>
            ) : (
              <p>{t("ui.download.dropHint")}</p>
            )}
            <div className="actions">
              <button className="btn" onClick={() => void choose(false)} disabled={busy || !gameId}>
                {t("ui.download.pickZip")}
              </button>
              <button className="btn ghost" onClick={() => void choose(true)} disabled={busy || !gameId}>
                {t("ui.download.pickFolderBtn")}
              </button>
            </div>
          </div>
          {busy && <p className="muted">{t("ui.download.reading")}</p>}
          {error && <div className="callout error">{error}</div>}

          {preview?.kind === "needs-password" && path && (
            <div className="dl-guide">
              <h3>
                <Icon name="shield" size={16} /> {t("ui.download.pwTitle")}
              </h3>
              <p>{t("ui.download.pwHint")}</p>
              <form
                className="inline"
                onSubmit={(e) => {
                  e.preventDefault();
                  void inspect(path, password);
                }}
              >
                <input className="field" type="password" autoFocus value={password} onChange={(e) => setPassword(e.target.value)} placeholder={t("ui.download.pwPlaceholder")} aria-label={t("ui.download.pwPlaceholder")} />
                <button className="btn primary" type="submit" disabled={!password || busy}>
                  {t("ui.download.pwSubmit")}
                </button>
              </form>
              <small>{t("ui.download.pwPrivacy")}</small>
            </div>
          )}
          {preview?.kind === "needs-extract" && (
            <div className="dl-guide">
              <h3>{t("ui.download.extractTitle")}</h3>
              <ol>
                <li>{t("ui.download.extract1")}</li>
                <li>{t("ui.download.extract2")}</li>
                <li>{t("ui.download.extract3")}</li>
              </ol>
            </div>
          )}
          {preview?.kind === "bonfire" && path && (
            <div className="dl-guide">
              <h3>{t("ui.download.bonfireTitle")}</h3>
              <p>{t("ui.download.bonfireHint")}</p>
              {onUseBonfireImport && (
                <button className="btn primary" onClick={() => { onUseBonfireImport(path); onClose(); }}>
                  {t("ui.download.bonfireUse")}
                </button>
              )}
            </div>
          )}
          {preview?.kind === "download" && saveFiles.length === 0 && (
            <div className="dl-guide">
              <h3>{t("ui.download.emptyTitle")}</h3>
              <p>{t("ui.download.emptyHint")}</p>
            </div>
          )}
          {(error || (preview?.kind === "download" && saveFiles.length === 0)) && gameId && <ManualReplaceGuide gameId={gameId} locations={locs.map((l) => l.ord)} />}
          {ready && (
            <p className="dl-found">
              <Icon name="check" size={16} /> {t("ui.download.found", { n: saveFiles.length, ok: placed.length, ignored: ignored.length })}
            </p>
          )}
          {!preview && !path && (
            <div className="dl-howto">
              <h3>{t("ui.download.howTitle")}</h3>
              <ol>
                <li>{t("ui.download.how1")}</li>
                <li>{t("ui.download.how2")}</li>
                <li>{t("ui.download.how3")}</li>
              </ol>
            </div>
          )}
        </div>
      )}

      {step === "map" && preview && (
        <div className="dl-map-step">
          <p className="hint">{unplaced.length > 0 ? t("ui.download.mapHintSome", { n: unplaced.length }) : t("ui.download.mapHintAll")}</p>
          {placed.length === 0 && (
            <div className="dl-guide">
              <h3>{t("ui.download.noneTitle")}</h3>
              <ol>
                <li>{t("ui.download.none1", { name: game ? displayName(game, i18n.language) : "" })}</li>
                <li>{t("ui.download.none2")}</li>
                <li>{t("ui.download.none3")}</li>
              </ol>
            </div>
          )}
          {placed.length === 0 && <ManualReplaceGuide gameId={gameId} locations={locs.map((l) => l.ord)} />}
          {overwrites.length > 0 && (
            <div className="dl-bulk" role="group" aria-label={t("ui.download.bulkTitle")}>
              <span className="muted">{t("ui.download.bulkTitle", { n: overwrites.length })}</span>
              <button type="button" className="btn small" onClick={() => setKeep(new Set())} disabled={overwrites.every((f) => !keep.has(f.rel))}>
                {t("ui.download.allOverwrite")}
              </button>
              <button type="button" className="btn small" onClick={() => setKeep(new Set(overwrites.map((f) => f.rel)))} disabled={overwrites.every((f) => keep.has(f.rel))}>
                {t("ui.download.allKeep")}
              </button>
            </div>
          )}
          <ul className="dl-map">
            {saveFiles.map((f) => {
              const tg = targets[f.rel];
              const ord = tg ? Number(tg.split("/")[0]) : -1;
              const inner = tg ? tg.slice(tg.indexOf("/") + 1) : "";
              const name = f.rel.split("/").pop() ?? f.rel;
              return (
                <li key={f.rel} className={!tg ? "off" : keep.has(f.rel) ? "kept" : "on"}>
                  <span className="dl-src" title={f.rel}>
                    <b>{name}</b>
                    <small>{f.rel}</small>
                  </span>
                  <Icon name="next" size={14} />
                  <span className="dl-dst">
                    <select
                      className="field"
                      title={ord >= 0 ? locs.find((l) => l.ord === ord)?.path : undefined}
                      value={ord}
                      aria-label={t("ui.download.placeOf", { name })}
                      onChange={(e) => {
                        const o = Number(e.target.value);
                        setTargets((x) => ({ ...x, [f.rel]: o < 0 ? null : `${o}/${inner || name}` }));
                      }}
                    >
                      <option value={-1}>{t("ui.download.skip")}</option>
                      {locs.map((l) => (
                        <option key={l.ord} value={l.ord}>
                          {locLabel(l.ord)}
                        </option>
                      ))}
                    </select>
                    {tg && (
                      <input
                        className="field num"
                        value={inner}
                        aria-label={t("ui.download.innerOf", { name })}
                        onChange={(e) => setTargets((x) => ({ ...x, [f.rel]: e.target.value.trim() ? `${ord}/${e.target.value.trim().replace(/^\/+/, "")}` : null }))}
                      />
                    )}
                    {/* 放到的位置已有同名文件：覆盖；没有：新增。改了位置也实时判断。 */}
                    {tg &&
                      (existing.has(tg.toLowerCase()) ? (
                        // 会覆盖现有文件：可以选“不替换”。
                        <span className="seg dl-choice" role="radiogroup" aria-label={t("ui.download.choiceOf", { name })}>
                          <button type="button" role="radio" aria-checked={!keep.has(f.rel)} className="overwrite" title={t("ui.download.overwriteHint")} onClick={() => setKeepOf(f.rel, false)}>
                            {t("ui.download.overwrite")}
                          </button>
                          <button type="button" role="radio" aria-checked={keep.has(f.rel)} className="keep" title={t("ui.download.keepHint")} onClick={() => setKeepOf(f.rel, true)}>
                            {t("ui.download.keep")}
                          </button>
                        </span>
                      ) : (
                        <span className="dl-fate add" title={t("ui.download.addHint")}>
                          {t("ui.download.add")}
                        </span>
                      ))}
                  </span>
                  <span className="num muted">{bytes(f.size)}</span>
                </li>
              );
            })}
          </ul>
          {ignored.length > 0 && (
            <details className="dl-ignored">
              <summary>{t("ui.download.ignored", { n: ignored.length })}</summary>
              <ul>
                {ignored.map((f) => (
                  <li key={f.rel}>{f.rel}</li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}

      {step === "finish" && preview && (
        <div className="dl-finish">
          <div className="backup-kinds" role="radiogroup" aria-label={t("ui.download.modeTitle")}>
            {(["overlay", "replace"] as DownloadMode[]).map((m) => (
              <label key={m} className={`backup-kind${mode === m ? " on" : ""}`}>
                <input type="radio" className="check" name="dl-mode" checked={mode === m} onChange={() => setMode(m)} />
                <span className="backup-kind-icon mode-saves">
                  <Icon name={m === "overlay" ? "stack" : "restore"} size={20} />
                </span>
                <span>
                  <b>
                    {t(`ui.download.mode.${m}`)}
                    {preview.mode === m && <span className="tag rec">{t("ui.download.recommended")}</span>}
                  </b>
                  <small>{t(`ui.download.modeHint.${m}`, { n: preview.currentFiles })}</small>
                </span>
              </label>
            ))}
          </div>
          <div className="row">
            <label htmlFor="dl-branch">{t("ui.download.branch")}</label>
            <input id="dl-branch" className="field" value={branch} onChange={(e) => setBranch(e.target.value)} />
          </div>
          <div className="row">
            <label htmlFor="dl-note">{t("ui.download.note")}</label>
            <input id="dl-note" className="field" value={comment} onChange={(e) => setComment(e.target.value)} placeholder={t("ui.download.notePlaceholder")} />
          </div>
          <label className="opt">
            <input type="checkbox" className="check" checked={useNow} onChange={(e) => setUseNow(e.target.checked)} />
            <span>
              <b>{t("ui.download.useNow")}</b>
              <small className="muted" style={{ display: "block" }}>
                {t("ui.download.useNowHint")}
              </small>
            </span>
          </label>
        </div>
      )}
    </Modal>
  );
}

/** 原来的导入中选到的不是篝火导出的存档：提示并一键改用“导入下载的存档”（U-42g）。 */
export function NotBonfireHint({ onUse }: { onUse: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="dl-guide not-bonfire" role="note">
      <h3>{t("ui.download.notBonfireTitle")}</h3>
      <p>{t("ui.download.notBonfireHint")}</p>
      <button className="btn primary" onClick={onUse}>
        <Icon name="import" size={16} /> {t("ui.download.notBonfireUse")}
      </button>
    </div>
  );
}

/** 拖放：把文件拖到窗口上时回调（Tauri 拖放事件）。 */
export function useFileDrop(onDrop: (path: string) => void) {
  const stable = useMemo(() => ({ onDrop }), []);
  stable.onDrop = onDrop;
  useEffect(() => {
    let off: (() => void) | undefined;
    let dead = false;
    Promise.resolve(api.onFileDrop?.((paths) => paths[0] && stable.onDrop(paths[0])))
      .then((u) => {
        if (dead) u?.();
        else off = u;
      })
      .catch(() => {});
    return () => {
      dead = true;
      off?.();
    };
  }, []);
}

/**
 * 导入不了时的退路：手工替换文件再备份（U-43）。强调先备份当前存档，替换后备份会默认另开新路线。
 */
export function ManualReplaceGuide({ gameId, locations }: { gameId: string; locations: number[] }) {
  const { t } = useTranslation();
  const store = useStore();
  const [backedUp, setBackedUp] = useState(false);
  const [busy, setBusy] = useState(false);

  const backupNow = async () => {
    setBusy(true);
    try {
      const [r] = await api.runBackup([{ gameId }], false, { mode: "new" });
      if (r && !r.ok) throw new Error(r.error?.message ?? "");
      setBackedUp(true);
      await store.refreshRow(gameId);
      store.toast("success", r?.result?.result === "created" ? t("ui.manual.backedUp") : t("ui.manual.alreadyBacked"));
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const ords = locations.length > 0 ? locations : [0];
  return (
    <div className="dl-guide manual-guide" role="note">
      <h3>{t("ui.manual.title")}</h3>
      <p className="manual-warn">
        <Icon name="shield" size={15} /> {t("ui.manual.warn")}
      </p>
      <ol>
        <li>
          {t("ui.manual.step1")}
          <button className={`btn small${backedUp ? "" : " primary"}`} onClick={() => void backupNow()} disabled={busy || backedUp}>
            {backedUp ? <><Icon name="check" size={14} /> {t("ui.manual.done")}</> : <><Icon name="backup" size={14} /> {t("ui.manual.backup")}</>}
          </button>
        </li>
        <li>{t("ui.manual.step2")}</li>
        <li>
          {t("ui.manual.step3")}
          {ords.map((o) => (
            <button key={o} className="btn small" onClick={() => void api.openLocation(gameId, o).catch((e) => store.toast("error", errorMessage(e)))}>
              <Icon name="folder" size={14} /> {ords.length > 1 ? t("ui.manual.openN", { n: o + 1 }) : t("ui.manual.open")}
            </button>
          ))}
        </li>
        <li>{t("ui.manual.step4")}</li>
        <li>{t("ui.manual.step5")}</li>
      </ol>
    </div>
  );
}
