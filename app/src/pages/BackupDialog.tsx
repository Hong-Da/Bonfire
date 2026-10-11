import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage, type OnDetached } from "../api";
import { Cover, Icon, Modal, displayName } from "../components/ui";
import { bytes, displayPath } from "../lib/format";
import { useStore } from "../store";
import type { BackupItemResult, FileChange, GameRow } from "../types";

export const MAX_COMMENT = 500;

/**
 * 备份所选游戏，分步进行：
 * 1. 选择要备份的内容：存档、截图，或两者（所选游戏中只有一种可备份时跳过这一步）；
 * 2. 确认备份存档：逐个游戏写备注（留空自动生成），顶部“统一备注”一键填充，可同时打标签（文档 9.1）；
 *    已是最新的游戏单独列出并跳过；
 * 3. 确认收集截图。
 * 确认页中不想备份的游戏可以移除。
 */
type Step = "pick" | "saves" | "shots";

export default function BackupDialog({ rows: picked, onClose }: { rows: GameRow[]; onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [comments, setComments] = useState<Record<string, string>>({});
  const [unified, setUnified] = useState("");
  const [withTag, setWithTag] = useState(false);
  const [tag, setTag] = useState("");
  const [detachedMode, setDetachedMode] = useState<"new" | "move">("new");
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState(0);
  const [results, setResults] = useState<BackupItemResult[] | null>(null);
  // 打开时分好：管理存档的游戏中已是最新的跳过（状态以最近一次检查为准，备份时后端还会再判断一次）；
  // 管理截图的游戏可以收集截图。备份完成后行状态刷新也不影响结果列表。
  const [plan] = useState(() => {
    const saveCand = picked.filter((r) => !r.shotsOnly && r.mode !== "shots");
    return {
      skipped: saveCand.filter((r) => r.state === "up-to-date"),
      saves: saveCand.filter((r) => r.state !== "up-to-date"),
      hasSaves: saveCand.length > 0,
      shots: picked.filter((r) => r.mode !== "saves"),
      // 所选游戏中不参与某一步的：只管理截图的不备份存档，只管理存档的不收集截图。
      shotsOnly: picked.filter((r) => r.shotsOnly || r.mode === "shots"),
      savesOnly: picked.filter((r) => r.mode === "saves"),
    };
  });
  const canPick = plan.hasSaves && plan.shots.length > 0;
  const [wantSaves, setWantSaves] = useState(plan.hasSaves);
  const [wantShots, setWantShots] = useState(plan.shots.length > 0);
  const [removed, setRemoved] = useState<{ saves: Set<string>; shots: Set<string> }>({ saves: new Set(), shots: new Set() });
  const steps: Step[] = [...(canPick ? (["pick"] as const) : []), ...(wantSaves ? (["saves"] as const) : []), ...(wantShots ? (["shots"] as const) : [])];
  const [stepIdx, setStepIdx] = useState(0);
  const step = steps[Math.min(stepIdx, steps.length - 1)] ?? "saves";
  const last = stepIdx >= steps.length - 1;

  const saveRows = wantSaves ? plan.saves.filter((r) => !removed.saves.has(r.id)) : [];
  const shotRows = wantShots ? plan.shots.filter((r) => !removed.shots.has(r.id)) : [];
  // 本次要处理的游戏（按所选顺序）：备份存档和 / 或收集截图。
  const targets = picked.filter((r) => saveRows.includes(r) || shotRows.includes(r));
  const drop = (kind: "saves" | "shots", id: string) => setRemoved((x) => ({ ...x, [kind]: new Set(x[kind]).add(id) }));
  const undrop = (kind: "saves" | "shots", id: string | null) =>
    setRemoved((x) => {
      const next = new Set(x[kind]);
      if (id === null) next.clear();
      else next.delete(id);
      return { ...x, [kind]: next };
    });

  // 游戏正在运行时只提醒，不阻止备份（U-12）。
  const [runningGames, setRunningGames] = useState<{ gameId: string; processes: string[] }[]>([]);
  useEffect(() => {
    if (plan.saves.length === 0) return;
    api
      .runningGames(plan.saves.map((r) => r.id))
      .then(setRunningGames)
      .catch(() => setRunningGames([]));
  }, []);
  // 截图确认页：每个游戏本次会新增的截图数（先用上次检查的结果，打开截图页时按磁盘重新数）。
  const [newShots, setNewShots] = useState<Record<string, number>>(() => Object.fromEntries(plan.shots.map((r) => [r.id, r.newShots ?? 0])));
  useEffect(() => {
    if (step !== "shots") return;
    let off = false;
    for (const r of plan.shots)
      api
        .album(r.id)
        .then((a) => !off && setNewShots((m) => ({ ...m, [r.id]: a.pending })))
        .catch(() => {});
    return () => {
      off = true;
    };
  }, [step]);
  const newTotal = shotRows.reduce((n, r) => n + (newShots[r.id] ?? 0), 0);
  const shownRunning = runningGames.filter((g) => saveRows.some((r) => r.id === g.gameId));
  const detached = saveRows.filter((r) => r.detached);
  const tooLong = saveRows.some((r) => [...(comments[r.id] ?? "")].length > MAX_COMMENT);

  const fillAll = () => setComments(Object.fromEntries(saveRows.map((r) => [r.id, unified])));

  const start = async () => {
    setRunning(true);
    const unlisten = await api.onBackupProgress((p) => setProgress(p.index + 1));
    try {
      const onDetached: OnDetached = { mode: detachedMode };
      const items = targets.map((r) => {
        const saves = saveRows.includes(r);
        const item: { gameId: string; comment?: string; tag?: string; saves?: boolean; shots?: boolean } = {
          gameId: r.id,
          comment: saves ? comments[r.id]?.trim() || undefined : undefined,
          tag: saves && withTag && tag.trim() ? tag.trim() : undefined,
        };
        if (!saves) item.saves = false;
        if (!shotRows.includes(r)) item.shots = false;
        return item;
      });
      const res = await api.runBackup(items, false, onDetached);
      setResults(res);
      await Promise.all(targets.map((r) => store.refreshRow(r.id)));
      const created = res.filter((x) => x.ok && x.result?.result === "created").length;
      const pics = res.reduce((n, x) => n + (x.shots ?? 0), 0);
      store.toast(
        res.every((x) => x.ok) ? "success" : "error",
        saveRows.length > 0 ? t("ui.backup.done", { created, failed: res.filter((x) => !x.ok).length }) : t("ui.backup.shotsDoneAll", { n: pics }),
      );
      saveRows.forEach((r) => store.setUndoable(r.id, null));
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      unlisten();
      setRunning(false);
    }
  };

  const nextDisabled =
    step === "pick" ? !wantSaves && !wantShots : step === "saves" ? tooLong || (withTag && !tag.trim()) : false;

  const footer = results ? (
    <button className="btn primary" onClick={onClose}>
      {t("ui.close")}
    </button>
  ) : (
    <>
      {stepIdx > 0 && (
        <button className="btn step-prev" onClick={() => setStepIdx(stepIdx - 1)} disabled={running}>
          <span className="flip"><Icon name="next" size={14} /></span> {t("ui.backup.prev")}
        </button>
      )}
      {step === "saves" && saveRows.length > 0 && (
        <>
          <label className="inline" style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <input type="checkbox" className="check" checked={withTag} onChange={(e) => setWithTag(e.target.checked)} />
            {t("ui.backup.withTag")}
          </label>
          {withTag && <input className="field" value={tag} onChange={(e) => setTag(e.target.value)} placeholder={t("ui.backup.tagName")} aria-label={t("ui.backup.tagName")} />}
        </>
      )}
      <div className="grow" />
      {running && <span className="muted num">{t("ui.backup.progress", { done: progress, total: targets.length })}</span>}
      <button className="btn ghost" onClick={running ? () => api.cancelTask() : onClose}>
        {t("ui.cancel")}
      </button>
      {last ? (
        <button className="btn primary" onClick={start} disabled={running || targets.length === 0 || tooLong || (withTag && !tag.trim())}>
          {t("ui.backup.confirm", { n: targets.length })}
        </button>
      ) : (
        <button className="btn primary step-next" onClick={() => setStepIdx(stepIdx + 1)} disabled={nextDisabled}>
          {t("ui.backup.next")} <Icon name="next" size={14} />
        </button>
      )}
    </>
  );

  const stepLabel = (st: Step) => t(`ui.backup.step.${st}`);
  const subtitle = results ? undefined : steps.length > 1 ? t("ui.backup.stepOf", { i: stepIdx + 1, n: steps.length, what: stepLabel(step) }) : t(`ui.backup.stepHint.${step}`);

  return (
    <Modal title={t("ui.backup.title")} subtitle={subtitle} footer={footer} onClose={running ? () => {} : onClose} wide>
      {results ? (
        <ul className="note-rows" aria-label={t("ui.backup.results")}>
          {results.map((r) => {
            const row = picked.find((x) => x.id === r.gameId);
            const name = row ? displayName(row, i18n.language) : r.gameId;
            let text: string;
            if (!r.ok) text = t("ui.backup.failed", { msg: r.error?.message ?? "" });
            else if (r.shotsOnly) text = r.shots > 0 ? t("ui.backup.shotsDone", { n: r.shots }) : t("ui.backup.shotsNone");
            else if (r.result?.result === "created") text = t("ui.backup.created", { comment: r.result.comment, size: bytes(r.result.newBytes) });
            else text = t("ui.backup.noChanges");
            return (
              <li key={r.gameId} className="note-row">
                <span />
                <div>
                  <div className="head">
                    <strong>{name}</strong>
                  </div>
                  <div className={r.ok ? "" : "callout error"}>{text}</div>
                  {r.result?.result === "created" && r.result.newBranch && <div className="muted">{t("ui.backup.newBranch", { branch: r.result.newBranch })}</div>}
                  {r.result?.result === "created" && r.result.warnings.map((w) => <div key={w} className="callout warn">{w}</div>)}
                  {r.shots > 0 && !r.shotsOnly && <div className="muted">{t("ui.shots.backupCollected", { n: r.shots })}</div>}
                </div>
              </li>
            );
          })}
        </ul>
      ) : (
        <>
          {steps.length > 1 && (
            <ol className="backup-steps" aria-label={t("ui.backup.steps")}>
              {steps.map((st, i) => (
                <li key={st} className={i === stepIdx ? "current" : i < stepIdx ? "done" : undefined} aria-current={i === stepIdx ? "step" : undefined}>
                  <span className="num">{i + 1}</span> {stepLabel(st)}
                </li>
              ))}
            </ol>
          )}

          {step === "pick" && (
            <div className="backup-kinds" role="group" aria-label={t("ui.backup.step.pick")}>
              <label className={`backup-kind${wantSaves ? " on" : ""}`}>
                <input type="checkbox" className="check" checked={wantSaves} onChange={(e) => setWantSaves(e.target.checked)} />
                <span className="backup-kind-icon mode-saves">
                  <Icon name="backup" size={20} />
                </span>
                <span>
                  <b>{t("ui.backup.kindSaves")}</b>
                  <small>
                    {plan.saves.length > 0 ? t("ui.backup.kindSavesHint", { n: plan.saves.length }) : t("ui.backup.kindSavesNone")}
                    {plan.skipped.length > 0 && t("ui.backup.kindSavesSkip", { n: plan.skipped.length })}
                    {plan.shotsOnly.length > 0 && t("ui.backup.kindSavesShotsOnly", { n: plan.shotsOnly.length })}
                  </small>
                </span>
              </label>
              <label className={`backup-kind${wantShots ? " on" : ""}`}>
                <input type="checkbox" className="check" checked={wantShots} onChange={(e) => setWantShots(e.target.checked)} />
                <span className="backup-kind-icon mode-shots">
                  <Icon name="camera" size={20} />
                </span>
                <span>
                  <b>{t("ui.backup.kindShots")}</b>
                  <small>
                    {t("ui.backup.kindShotsHint", { n: plan.shots.length })}
                    {plan.savesOnly.length > 0 && t("ui.backup.kindShotsSavesOnly", { n: plan.savesOnly.length })}
                  </small>
                </span>
              </label>
            </div>
          )}

          {step === "saves" && (
            <>
              {saveRows.length === 0 && (
                <div className="callout same">
                  <p>{plan.saves.length === 0 ? t("ui.backup.allUpToDate", { n: plan.skipped.length }) : t("ui.backup.noneLeft")}</p>
                </div>
              )}
              {saveRows.length > 1 && (
                <div className="row">
                  <label htmlFor="unified">{t("ui.backup.unified")}</label>
                  <div className="inline">
                    <input id="unified" className="field" style={{ flex: 1 }} value={unified} onChange={(e) => setUnified(e.target.value)} placeholder={t("ui.backup.commentPlaceholder")} />
                    <button className="btn" onClick={fillAll} disabled={!unified.trim()}>
                      {t("ui.backup.fillAll")}
                    </button>
                  </div>
                </div>
              )}
              {shownRunning.length > 0 && (
                <div className="callout warn" role="note">
                  <p style={{ margin: 0 }}>
                    {t("ui.backup.gameRunning", {
                      names: shownRunning
                        .map((g) => {
                          const r = saveRows.find((x) => x.id === g.gameId);
                          return `${r ? displayName(r, i18n.language) : g.gameId}（${g.processes.join("、")}）`;
                        })
                        .join("、"),
                    })}
                  </p>
                </div>
              )}
              {detached.length > 0 && (
                <div className="callout warn">
                  <p style={{ margin: "0 0 6px" }}>{t("ui.backup.detachedIntro", { names: detached.map((r) => displayName(r, i18n.language)).join("、") })}</p>
                  <label style={{ display: "block" }}>
                    <input type="radio" name="detached" checked={detachedMode === "new"} onChange={() => setDetachedMode("new")} /> {t("ui.backup.detachedNew")}
                  </label>
                  <label style={{ display: "block" }}>
                    <input type="radio" name="detached" checked={detachedMode === "move"} onChange={() => setDetachedMode("move")} /> {t("ui.backup.detachedMove")}
                  </label>
                </div>
              )}
              <ul className="note-rows">
                {saveRows.map((r) => (
                  <NoteRow
                    key={r.id}
                    row={r}
                    value={comments[r.id] ?? ""}
                    onChange={(v) => setComments((c) => ({ ...c, [r.id]: v }))}
                    onRemove={() => drop("saves", r.id)}
                  />
                ))}
              </ul>
              <RemovedStrip rows={plan.saves.filter((r) => removed.saves.has(r.id))} onRestore={(id) => undrop("saves", id)} />
              <ExcludedList rows={plan.shotsOnly} title={t("ui.backup.excludedShotsOnly", { n: plan.shotsOnly.length })} hint={t(wantShots ? "ui.backup.excludedShotsOnlyHint" : "ui.backup.excludedShotsOnlyHintNoShots")} tag={t("ui.mode.shots")} />
              {plan.skipped.length > 0 && (
                <section className="skip-list" aria-label={t("ui.backup.skippedTitle", { n: plan.skipped.length })}>
                  <h3>{t("ui.backup.skippedTitle", { n: plan.skipped.length })}</h3>
                  <p className="hint">{t("ui.backup.skippedHint")}</p>
                  <ul>
                    {plan.skipped.map((r) => (
                      <li key={r.id}>
                        <Cover row={r} size="note" />
                        <span>{displayName(r, i18n.language)}</span>
                        <span className="tag">{t("ui.backup.upToDate")}</span>
                      </li>
                    ))}
                  </ul>
                </section>
              )}
            </>
          )}

          {step === "shots" && (
            <>
              <p className="hint">{t("ui.backup.shotsIntro")}</p>
              {shotRows.length > 0 && <p className="shots-total">{newTotal > 0 ? t("ui.backup.shotsNewTotal", { n: newTotal, games: shotRows.length }) : t("ui.backup.shotsNewNone")}</p>}
              {shotRows.length === 0 && (
                <div className="callout same">
                  <p>{t("ui.backup.noneLeft")}</p>
                </div>
              )}
              <ul className="note-rows">
                {shotRows.map((r) => {
                  const name = displayName(r, i18n.language);
                  return (
                    <li key={r.id} className="note-row">
                      <Cover row={r} size="note" />
                      <div>
                        <div className="head">
                          <strong>{name}</strong>
                          <span className="muted">
                            {t("ui.backup.shotsHave", { n: r.shots })}
                            {" · "}
                            {(newShots[r.id] ?? 0) > 0 ? <b className="shots-new">{t("ui.backup.shotsNew", { n: newShots[r.id] })}</b> : t("ui.backup.shotsNoNew")}
                          </span>
                          <RemoveButton name={name} onClick={() => drop("shots", r.id)} />
                        </div>
                      </div>
                    </li>
                  );
                })}
              </ul>
              <RemovedStrip rows={plan.shots.filter((r) => removed.shots.has(r.id))} onRestore={(id) => undrop("shots", id)} />
              <ExcludedList rows={plan.savesOnly} title={t("ui.backup.excludedSavesOnly", { n: plan.savesOnly.length })} hint={t("ui.backup.excludedSavesOnlyHint")} tag={t("ui.mode.saves")} />
            </>
          )}
        </>
      )}
    </Modal>
  );
}

/** 所选游戏中因管理方式不参与这一步的（只管理截图的不备份存档、只管理存档的不收集截图）。 */
function ExcludedList({ rows, title, hint, tag }: { rows: GameRow[]; title: string; hint: string; tag: string }) {
  const { i18n } = useTranslation();
  if (rows.length === 0) return null;
  return (
    <section className="skip-list" aria-label={title}>
      <h3>{title}</h3>
      <p className="hint">{hint}</p>
      <ul>
        {rows.map((r) => (
          <li key={r.id}>
            <Cover row={r} size="note" />
            <span>{displayName(r, i18n.language)}</span>
            <span className="tag">{tag}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** 这一步中移出的游戏：点一下恢复，或全部恢复。 */
function RemovedStrip({ rows, onRestore }: { rows: GameRow[]; onRestore: (id: string | null) => void }) {
  const { t, i18n } = useTranslation();
  if (rows.length === 0) return null;
  return (
    <section className="removed-strip" aria-label={t("ui.backup.removedTitle", { n: rows.length })}>
      <span className="muted">{t("ui.backup.removedTitle", { n: rows.length })}</span>
      {rows.map((r) => {
        const name = displayName(r, i18n.language);
        return (
          <button key={r.id} type="button" className="removed-chip" onClick={() => onRestore(r.id)} title={t("ui.backup.restoreRow", { name })} aria-label={t("ui.backup.restoreRow", { name })}>
            <Icon name="undo" size={12} /> {name}
          </button>
        );
      })}
      {rows.length > 1 && (
        <button type="button" className="btn ghost small" onClick={() => onRestore(null)}>
          {t("ui.backup.restoreAll")}
        </button>
      )}
    </section>
  );
}

/** 确认页中把一个游戏移出本次备份。 */
function RemoveButton({ name, onClick }: { name: string; onClick: () => void }) {
  const { t } = useTranslation();
  const label = t("ui.backup.removeRow", { name });
  return (
    <button type="button" className="row-skip" onClick={onClick} aria-label={label} title={label}>
      <Icon name="close" size={12} /> {t("ui.backup.skipShort")}
    </button>
  );
}

function NoteRow({ row, value, onChange, onRemove }: { row: GameRow; value: string; onChange: (v: string) => void; onRemove: () => void }) {
  const { t, i18n } = useTranslation();
  const [files, setFiles] = useState<FileChange[] | null>(null);
  const [open, setOpen] = useState(false);
  const len = [...value].length;
  const name = displayName(row, i18n.language);
  const changed = row.summary.added + row.summary.modified + row.summary.deleted;

  useEffect(() => {
    if (open && files === null) api.gameChanges(row.id).then(setFiles).catch(() => setFiles([]));
  }, [open, files, row.id]);

  return (
    <li className="note-row">
      <Cover row={row} size="note" />
      <div>
        <div className="head">
          <strong>{name}</strong>
          <span className="muted">
            {row.state === "never-backed-up"
              ? t("ui.backup.firstBackup", { n: row.files })
              : row.state === "up-to-date"
                ? t("ui.backup.upToDate")
                : t("ui.library.changedFiles", { n: changed, size: bytes(row.summary.changedBytes) })}
          </span>
          <RemoveButton name={name} onClick={onRemove} />
        </div>
        <textarea
          className="field comment-edit"
          rows={2}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={t("ui.backup.commentPlaceholder")}
          aria-label={t("ui.backup.commentFor", { name })}
        />
        <div className={`counter num${len > MAX_COMMENT ? " over" : ""}`}>
          {len}/{MAX_COMMENT}
        </div>
        {changed > 0 && (
          <details onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
            <summary>{t("ui.backup.showFiles")}</summary>
            <ul>
              {files === null && <li className="muted">{t("ui.loading")}</li>}
              {files?.map((f) => (
                <li key={f.path}>
                  {t(`ui.file.${f.status}`)} {displayPath(f.path)}
                </li>
              ))}
            </ul>
          </details>
        )}
      </div>
    </li>
  );
}
