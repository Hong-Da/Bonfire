import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { Cover, Icon, Menu, StatusBadge, displayName } from "../components/ui";
import { bytes, relative } from "../lib/format";
import { checkAndReport } from "../lib/check";
import { useDelayed } from "../lib/hooks";
import { visibleRows, useStore, type ModeFilter, type StateFilter } from "../store";
import type { GameRow, Profile } from "../types";
import BackupDialog from "./BackupDialog";
import AddGameDialog from "./AddGameDialog";
import { CollectionExportDialog, CollectionImportDialog } from "./CollectionDialogs";
import ProtectSwitch from "../components/ProtectSwitch";
import AutoSwitch from "../components/AutoSwitch";
import ModeMenu, { MODES, MODE_ICON, useSetMode } from "../components/ModeMenu";
import RemoveDialog from "../components/RemoveDialog";
import SelectSeg from "../components/SelectSeg";
import { confirm } from "../lib/confirm";

const STATES: StateFilter[] = ["all", "has-changes", "never-backed-up", "up-to-date"];

export default function Library() {
  const { t, i18n } = useTranslation();
  const s = useStore();
  const rows = useMemo(() => visibleRows(s), [s.rows, s.order, s.platformFilter, s.stateFilter, s.modeFilter, s.profileFilter, s.query]);
  const allRows = useMemo(() => s.order.map((id) => s.rows[id]).filter(Boolean), [s.rows, s.order]);
  const platforms = useMemo(() => [...new Set(allRows.map((r) => r.platform))].sort(), [allRows]);
  const selected = allRows.filter((r) => s.selected.has(r.id));
  // 筛选方案、状态与平台合并在“筛选”菜单中，按钮上显示当前生效的筛选；管理方式单独一组按钮。
  const filtered = !!s.profileFilter || s.stateFilter !== "all" || s.platformFilter !== "all";
  const filterText = [
    s.profileFilter ? t("ui.library.profileActive", { name: s.profileFilter.name }) : "",
    s.stateFilter !== "all" ? t(`ui.state.${s.stateFilter}`) : "",
    s.platformFilter !== "all" ? t(`ui.platform.${s.platformFilter}`, { defaultValue: s.platformFilter }) : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const [backupOpen, setBackupOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [profileName, setProfileName] = useState<string | null>(null);
  const [exportIds, setExportIds] = useState<string[] | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const setMode = useSetMode();

  useEffect(() => {
    api.profiles().then(setProfiles).catch(() => setProfiles([]));
  }, [backupOpen]);

  // 筛选方案：一组常一起处理的游戏。选择后只列出并勾选这些游戏，“不使用筛选方案”时恢复原来的勾选。
  const activeProfile = s.profileFilter ? (profiles.find((p) => p.name === s.profileFilter!.name) ?? null) : null;
  const beforeProfile = useRef<Set<string> | null>(null);

  const applyProfile = (name: string) => {
    const p = profiles.find((x) => x.name === name);
    if (!p) return;
    if (!s.profileFilter) beforeProfile.current = new Set(s.selected);
    const ids = p.gameIds.filter((id) => s.rows[id]);
    s.setProfileFilter({ name: p.name, ids });
    s.clearSelection();
    s.selectAll(ids);
  };

  const cancelProfile = () => {
    const prev = beforeProfile.current;
    beforeProfile.current = null;
    s.setProfileFilter(null);
    s.clearSelection();
    if (prev) s.selectAll([...prev].filter((id) => s.rows[id]));
  };

  const deleteProfile = async (name: string) => {
    if (!(await confirm(t("ui.library.profileDeleteConfirm", { name }), { kind: "warning" }))) return;
    try {
      await api.deleteProfile(name);
      if (s.profileFilter?.name === name) cancelProfile();
      setProfiles(await api.profiles());
      s.toast("success", t("ui.library.profileDeleted", { name }));
    } catch (e) {
      s.toast("error", errorMessage(e));
    }
  };

  const saveProfile = async () => {
    const name = profileName?.trim();
    if (!name) return;
    try {
      await api.saveProfile({ name, gameIds: selected.map((r) => r.id), defaultComment: "" });
      setProfiles(await api.profiles());
      setProfileName(null);
      s.toast("success", t("ui.library.profileSaved", { name }));
    } catch (e) {
      s.toast("error", errorMessage(e));
    }
  };

  // 检查更新：按钮图标立即转动，完成后提示结果（很快完成时也有反馈，进度条与文字只在较慢时显示）。
  const check = checkAndReport;

  const phase = s.phase;
  const busy = useDelayed(s.loading);
  const pct = phase.total > 0 ? Math.round((phase.done / phase.total) * 100) : 0;

  return (
    <>
      <div className="toolbar">
        <h1>{t("ui.nav.library")}</h1>
        <button className={`btn${s.loading ? " working" : ""}`} onClick={() => void check()} disabled={s.loading} aria-busy={s.loading}>
          <Icon name="scan" /> {busy ? t(`ui.library.phase.${phase.phase}`, { done: phase.done, total: phase.total }) : t("ui.library.check")}
        </button>
        {busy && (
          <button className="btn ghost" onClick={() => api.cancelTask()}>
            {t("ui.cancel")}
          </button>
        )}
        <Menu
          icon="filter"
          className={`btn${filtered ? " filtered" : ""}`}
          label={filtered ? t("ui.library.filterOn", { what: filterText }) : t("ui.library.filter")}
          title={t("ui.library.filterHint")}
          items={[
            { label: <b className="menu-head">{t("ui.library.profiles")}</b>, onClick: () => {}, disabled: true },
            ...(profiles.length > 0
              ? profiles.map((p) => ({
                  label: (
                    <span className="mode-item">
                      <b>{p.name}</b>
                      <small>{t("ui.library.profileGames", { n: p.gameIds.filter((id) => s.rows[id]).length })}</small>
                    </span>
                  ),
                  checked: s.profileFilter?.name === p.name,
                  onClick: () => applyProfile(p.name),
                }))
              : [{ label: <span className="muted">{t("ui.library.profilesNone")}</span>, onClick: () => {}, disabled: true }]),
            ...(s.profileFilter ? [{ label: t("ui.library.profileCancel"), onClick: cancelProfile }] : []),
            { label: t("ui.library.saveProfile", { n: selected.length }), onClick: () => setProfileName(""), disabled: selected.length === 0 },
            ...(activeProfile ? [{ label: t("ui.library.profileDelete", { name: activeProfile.name }), onClick: () => void deleteProfile(activeProfile.name), danger: true }] : []),
            "divider" as const,
            { label: <b className="menu-head">{t("ui.library.filterState")}</b>, onClick: () => {}, disabled: true },
            ...STATES.map((st) => ({
              label: t("ui.mode.option", { mode: t(st === "all" ? "ui.library.all" : `ui.state.${st}`), n: st === "all" ? allRows.length : allRows.filter((r) => r.state === st).length }),
              checked: s.stateFilter === st,
              onClick: () => s.setStateFilter(st),
            })),
            ...(platforms.length > 1
              ? [
                  "divider" as const,
                  { label: <b className="menu-head">{t("ui.library.filterPlatform")}</b>, onClick: () => {}, disabled: true },
                  { label: t("ui.library.allPlatforms"), checked: s.platformFilter === "all", onClick: () => s.setPlatformFilter("all") },
                  ...platforms.map((p) => ({
                    label: t(`ui.platform.${p}`, { defaultValue: p }),
                    checked: s.platformFilter === p,
                    onClick: () => s.setPlatformFilter(p),
                  })),
                ]
              : []),
          ]}
        />
        {profileName !== null && (
          <form
            className="inline"
            style={{ display: "flex", gap: 6 }}
            onSubmit={(e) => {
              e.preventDefault();
              void saveProfile();
            }}
          >
            <input className="field" autoFocus value={profileName} onChange={(e) => setProfileName(e.target.value)} placeholder={t("ui.library.profileName")} aria-label={t("ui.library.profileName")} />
            <button className="btn small" type="submit" disabled={!profileName.trim()}>
              {t("ui.save")}
            </button>
            <button className="btn ghost small" type="button" onClick={() => setProfileName(null)}>
              {t("ui.cancel")}
            </button>
          </form>
        )}
        {/* 按管理方式筛选：单独一组按钮，每种管理方式一个颜色（与每行的管理方式标签同色）。 */}
        <div className="mode-seg" role="group" aria-label={t("ui.mode.filter")}>
          {(["all", ...MODES] as ModeFilter[]).map((m) => {
            const n = m === "all" ? allRows.length : allRows.filter((r) => r.mode === m).length;
            const name = m === "all" ? t("ui.mode.short.all") : t(`ui.mode.${m}`);
            return (
              <button
                key={m}
                className={`mode-${m}`}
                aria-pressed={s.modeFilter === m}
                aria-label={t("ui.mode.option", { mode: name, n })}
                title={t("ui.mode.option", { mode: name, n })}
                onClick={() => s.setModeFilter(m)}
              >
                {m !== "all" && <Icon name={MODE_ICON[m]} size={14} />}
                <span className="seg-label">{t(`ui.mode.short.${m}`)}</span>
                <span className="chip-count">{n}</span>
              </button>
            );
          })}
        </div>
        <input className="field fill" type="search" placeholder={t("ui.library.search")} value={s.query} onChange={(e) => s.setQuery(e.target.value)} aria-label={t("ui.library.search")} />
      </div>
      <div className="progress-line" aria-hidden>
        <span style={{ width: busy ? `${Math.max(pct, 8)}%` : "0%" }} />
      </div>

      <div className="library">
        {allRows.length === 0 && !s.loading ? (
          <div className="empty">
            <h2>{t("ui.library.emptyTitle")}</h2>
            <p>{t("ui.library.emptyBody")}</p>
            <div className="actions">
              <button className="btn primary" onClick={() => s.go({ name: "scan" })}>
                <Icon name="search" /> {t("ui.library.toScan")}
              </button>
              <button className="btn" onClick={() => setAddOpen(true)}>
                <Icon name="plus" /> {t("ui.library.add")}
              </button>
              <button className="btn" onClick={() => setImportOpen(true)}>
                <Icon name="import" /> {t("ui.library.importSaves")}
              </button>
            </div>
          </div>
        ) : (
          <ul className="game-list">
            {rows.length > 0 && (
              <li className="game list-cols">
                <input
                  type="checkbox"
                  className="check"
                  aria-label={t("ui.scan.pickVisible")}
                  checked={rows.every((r) => s.selected.has(r.id))}
                  ref={(el) => {
                    if (el) el.indeterminate = rows.some((r) => s.selected.has(r.id)) && !rows.every((r) => s.selected.has(r.id));
                  }}
                  onChange={() => (rows.every((r) => s.selected.has(r.id)) ? s.clearSelection() : s.selectAll(rows.map((r) => r.id)))}
                />
                <span />
                <span>{t("ui.library.colGame")}</span>
                <span>{t("ui.library.colState")}</span>
                <span className="col-backup">{t("ui.library.colBackup")}</span>
                <span className="col-size">{t("ui.library.colSize")}</span>
                <span />
              </li>
            )}
            {rows.map((r) => (
              <GameLine key={r.id} row={r} lang={i18n.language} selected={s.selected.has(r.id)} onToggle={() => s.toggle(r.id)} onOpen={() => s.go({ name: "game", id: r.id })} />
            ))}
            {rows.length === 0 && (
              <li className="empty">
                <p>{t("ui.library.noMatch")}</p>
                <div className="actions">
                  <button
                    className="btn"
                    onClick={() => {
                      s.setStateFilter("all");
                      s.setModeFilter("all");
                      if (s.profileFilter) cancelProfile();
                      s.setPlatformFilter("all");
                      s.setQuery("");
                    }}
                  >
                    {t("ui.library.clearFilters")}
                  </button>
                </div>
              </li>
            )}
          </ul>
        )}
      </div>

      {allRows.length > 0 && (
        <div className="actionbar">
          {/* 左端：所选游戏（显示已选数量）→ 全选框 → 添加 / 导入与导出（一组）。 */}
          <Menu
            icon="check"
            label={t("ui.library.selected", { n: selected.length })}
            className={`btn sel-menu${selected.length > 0 ? " has" : ""}`}
            title={t("ui.library.selectedMenu")}
            items={[
              { label: t("ui.library.exportSelected", { n: selected.length }), onClick: () => setExportIds(selected.map((r) => r.id)), disabled: selected.length === 0 },
              "divider",
              { label: <b className="menu-head">{t("ui.mode.batchHead")}</b>, onClick: () => {}, disabled: true },
              ...MODES.map((m) => ({
                label: (
                  <span className="mode-item">
                    <b>
                      <Icon name={MODE_ICON[m]} size={14} /> {t(`ui.mode.${m}`)}
                    </b>
                  </span>
                ),
                onClick: () => void setMode(selected.map((r) => r.id), m),
                disabled: selected.length === 0,
              })),
              "divider",
              { label: t("ui.remove.button", { n: selected.length }), onClick: () => setRemoveOpen(true), disabled: selected.length === 0, danger: true },
            ]}
          />
          <SelectSeg
            total={rows.length}
            picked={rows.filter((r) => s.selected.has(r.id)).length}
            onAll={() => s.selectAll(rows.map((r) => r.id))}
            onNone={() => s.clearSelection()}
            onInvert={() => s.invert(rows.map((r) => r.id))}
            label={t("ui.library.selection")}
            hint={t("ui.library.selectAllHint")}
          />
          <div className="btn-group" role="group" aria-label={t("ui.library.addExport")}>
            <Menu
              icon="plus"
              label={<span className="bar-label">{t("ui.library.addMenu")}</span>}
              title={t("ui.library.addMenu")}
              className="btn"
              items={[
                { label: t("ui.library.add"), onClick: () => setAddOpen(true) },
                { label: t("ui.library.importSaves"), onClick: () => setImportOpen(true) },
              ]}
            />
            <Menu
              icon="export"
              label={<span className="bar-label">{t("ui.library.exportMenu")}</span>}
              title={t("ui.library.exportMenu")}
              className="btn"
              items={[
                { label: t("ui.library.exportSelected", { n: selected.length }), onClick: () => setExportIds(selected.map((r) => r.id)), disabled: selected.length === 0 },
                { label: t("ui.library.exportAll", { n: allRows.length }), onClick: () => setExportIds(allRows.map((r) => r.id)), disabled: allRows.length === 0 },
              ]}
            />
          </div>
          <div className="grow" style={{ flex: 1 }} />
          <AutoSwitch />
          <ProtectSwitch />
          <button className="btn primary" disabled={selected.length === 0 || busy} onClick={() => setBackupOpen(true)}>
            <Icon name="backup" /> {t("ui.library.backupSelected")}
          </button>
        </div>
      )}

      {backupOpen && <BackupDialog rows={selected} onClose={() => setBackupOpen(false)} />}
      {addOpen && <AddGameDialog onClose={() => setAddOpen(false)} />}
      {removeOpen && <RemoveDialog rows={selected} onClose={() => setRemoveOpen(false)} onDone={() => s.clearSelection()} />}
      {exportIds && <CollectionExportDialog rows={allRows} initial={exportIds} onClose={() => setExportIds(null)} />}
      {importOpen && <CollectionImportDialog onClose={() => setImportOpen(false)} onDone={() => void s.loadLibrary(false)} />}
    </>
  );
}

function GameLine({ row, lang, selected, onToggle, onOpen }: { row: GameRow; lang: string; selected: boolean; onToggle: () => void; onOpen: () => void }) {
  const { t } = useTranslation();
  const name = displayName(row, lang);
  const changed = row.summary.added + row.summary.modified + row.summary.deleted;
  return (
    <li className={`game${selected ? " selected" : ""}${row.state === "has-changes" ? " changed" : ""}`}>
      <input type="checkbox" className="check" checked={selected} onChange={onToggle} aria-label={t("ui.library.selectGame", { name })} />
      <Cover row={row} />
      <div className="title" onClick={onOpen} onKeyDown={(e) => e.key === "Enter" && onOpen()} role="button" tabIndex={0}>
        <strong>{name}</strong>
        <span className="sub">
          <span>{t(`ui.platform.${row.platform}`, { defaultValue: row.platform })}</span>
          {!row.installed && !row.shotsOnly && (
            <span className="tag not-installed" title={t("ui.library.notInstalledHint")}>
              {t("ui.library.notInstalled")}
            </span>
          )}
          {name !== row.name && <span>{row.name}</span>}
          {row.cloudSave && <Icon name="cloud" size={12} label={t("ui.library.cloud")} />}
        </span>
      </div>
      <div className="col-state">
        <StatusBadge row={row} />
        <ModeMenu ids={[row.id]} current={row.mode} name={name} />
        {row.state === "has-changes" && <div className="meta num">{t("ui.library.changedFiles", { n: changed, size: bytes(row.summary.changedBytes) })}</div>}
        {row.detached && <div className="meta">{t("ui.library.detached")}</div>}
        {row.newShots > 0 && (
          <div className="meta shot-new">
            <Icon name="camera" size={12} /> {t("ui.shots.newInLibrary", { n: row.newShots })}
          </div>
        )}
      </div>
      <div className="meta col-backup">
        {row.lastBackupAt ? (
          <>
            <b>{relative(row.lastBackupAt, lang)}</b>
            <div>{t("ui.library.snapshots", { n: row.snapshots })}</div>
          </>
        ) : (
          t("ui.library.never")
        )}
      </div>
      <div className="meta col-size num">
        {row.shotsOnly ? (
          <>
            <b>{t("ui.shots.countN", { n: row.shots })}</b>
            <div>{t("ui.shots.tab")}</div>
          </>
        ) : (
          <>
            <b>{bytes(row.totalBytes)}</b>
            <div>{t("ui.library.files", { n: row.files })}</div>
          </>
        )}
      </div>
      <button className="btn ghost small open" onClick={onOpen} aria-label={t("ui.library.openHistory", { name })} title={t("ui.library.openHistory", { name })}>
        <Icon name="next" />
      </button>
    </li>
  );
}
