import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { Cover, Icon, Menu, StatusBadge, displayName } from "../components/ui";
import { bytes, relative } from "../lib/format";
import { checkAndReport } from "../lib/check";
import { useDelayed } from "../lib/hooks";
import { visibleRows, useStore, type StateFilter } from "../store";
import type { GameRow, Profile } from "../types";
import BackupDialog from "./BackupDialog";
import AddGameDialog from "./AddGameDialog";
import { CollectionExportDialog, CollectionImportDialog } from "./CollectionDialogs";
import ProtectSwitch from "../components/ProtectSwitch";

const STATES: StateFilter[] = ["all", "has-changes", "never-backed-up", "up-to-date"];

export default function Library() {
  const { t, i18n } = useTranslation();
  const s = useStore();
  const rows = useMemo(() => visibleRows(s), [s.rows, s.order, s.platformFilter, s.stateFilter, s.query]);
  const allRows = useMemo(() => s.order.map((id) => s.rows[id]).filter(Boolean), [s.rows, s.order]);
  const platforms = useMemo(() => [...new Set(allRows.map((r) => r.platform))].sort(), [allRows]);
  const selected = allRows.filter((r) => s.selected.has(r.id));
  const [backupOpen, setBackupOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [profileName, setProfileName] = useState<string | null>(null);
  const [exportIds, setExportIds] = useState<string[] | null>(null);
  const [importOpen, setImportOpen] = useState(false);

  useEffect(() => {
    api.profiles().then(setProfiles).catch(() => setProfiles([]));
  }, [backupOpen]);

  const applyProfile = (name: string) => {
    const p = profiles.find((x) => x.name === name);
    if (!p) return;
    s.clearSelection();
    s.selectAll(p.gameIds.filter((id) => s.rows[id]));
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
        <div className="chips" role="group" aria-label={t("ui.library.filterState")}>
          {STATES.map((st) => (
            <button key={st} className="chip" aria-pressed={s.stateFilter === st} onClick={() => s.setStateFilter(st)}>
              {t(st === "all" ? "ui.library.all" : `ui.state.${st}`)}
              <span className="num chip-count">{st === "all" ? allRows.length : allRows.filter((r) => r.state === st).length}</span>
            </button>
          ))}
        </div>
        {platforms.length > 1 && (
          <select className="field" value={s.platformFilter} onChange={(e) => s.setPlatformFilter(e.target.value)} aria-label={t("ui.library.filterPlatform")}>
            <option value="all">{t("ui.library.allPlatforms")}</option>
            {platforms.map((p) => (
              <option key={p} value={p}>
                {t(`ui.platform.${p}`, { defaultValue: p })}
              </option>
            ))}
          </select>
        )}
        <div className="grow" />
        <input className="field" type="search" placeholder={t("ui.library.search")} value={s.query} onChange={(e) => s.setQuery(e.target.value)} aria-label={t("ui.library.search")} />
        <Menu
          icon="import"
          label={t("ui.library.transfer")}
          align="right"
          items={[
            { label: t("ui.library.importSaves"), onClick: () => setImportOpen(true) },
            { label: t("ui.library.exportSelected", { n: selected.length }), onClick: () => setExportIds(selected.map((r) => r.id)), disabled: selected.length === 0 },
            { label: t("ui.library.exportAll", { n: allRows.length }), onClick: () => setExportIds(allRows.map((r) => r.id)), disabled: allRows.length === 0 },
          ]}
        />
        <button className="btn" onClick={() => setAddOpen(true)}>
          <Icon name="plus" /> {t("ui.library.add")}
        </button>
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
          <button className="btn ghost small" onClick={() => s.selectAll(rows.map((r) => r.id))}>
            {t("ui.library.selectAll")}
          </button>
          <button className="btn ghost small" onClick={() => s.clearSelection()} disabled={selected.length === 0}>
            {t("ui.library.selectNone")}
          </button>
          <button className="btn ghost small" onClick={() => s.invert(rows.map((r) => r.id))}>
            {t("ui.library.invert")}
          </button>
          {profiles.length > 0 && (
            <select className="field" defaultValue="" onChange={(e) => applyProfile(e.target.value)} aria-label={t("ui.library.profiles")}>
              <option value="" disabled>
                {t("ui.library.profiles")}
              </option>
              {profiles.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
            </select>
          )}
          {profileName === null ? (
            <button className="btn ghost small" onClick={() => setProfileName("")} disabled={selected.length === 0}>
              {t("ui.library.saveProfile")}
            </button>
          ) : (
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
          <div className="grow" style={{ flex: 1 }} />
          <ProtectSwitch />
          <span className="count num">{t("ui.library.selected", { n: selected.length })}</span>
          <button className="btn primary" disabled={selected.length === 0 || busy} onClick={() => setBackupOpen(true)}>
            <Icon name="backup" /> {t("ui.library.backupSelected")}
          </button>
        </div>
      )}

      {backupOpen && <BackupDialog rows={selected} onClose={() => setBackupOpen(false)} />}
      {addOpen && <AddGameDialog onClose={() => setAddOpen(false)} />}
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
      <div>
        <StatusBadge row={row} />
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
