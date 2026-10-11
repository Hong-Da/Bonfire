import { useTranslation } from "react-i18next";
import { api } from "../api";
import { bytes } from "../lib/format";
import { Icon } from "./ui";

/** 导出得到的一个文件（或文件夹）。 */
export interface ExportedFile {
  path: string;
  bytes: number;
  /** 内容说明，如“存档 · 4 个游戏” */
  label: string;
}

const baseName = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
const dirName = (p: string) => p.replace(/[\\/][^\\/]*[\\/]?$/, "");

/** 文件标记：带折角的小卡片，写着扩展名（.bonfire / .zip），文件夹显示文件夹图标。 */
export function FileBadge({ ext }: { ext: string | null }) {
  return (
    <span className={`file-badge${ext ? "" : " folder"}`} aria-hidden="true">
      {ext ? `.${ext}` : <Icon name="folder" size={16} />}
    </span>
  );
}

/** 文件的扩展名（只认篝火导出的两种；其余视为文件夹）。 */
export const extOf = (p: string) => {
  const m = /\.(bonfire|zip)$/i.exec(p);
  return m ? m[1].toLowerCase() : null;
};

/**
 * 导出完成的页面（U-49）：导出后对话框换成结果——得到了哪些文件、各自多大、放在哪里，以及接下来怎么用。
 */
export default function ExportDone({ files, hint, skipped }: { files: ExportedFile[]; hint?: string; skipped?: string[] }) {
  const { t } = useTranslation();
  const dirs = [...new Set(files.map((f) => dirName(f.path)))];
  return (
    <div className="export-done" role="status">
      <div className="export-done-head">
        <span className="export-done-mark">
          <Icon name="check" size={22} />
        </span>
        <div>
          <h3>{t("ui.export.doneLead", { n: files.length })}</h3>
          {dirs.length === 1 && (
            <p className="muted" title={dirs[0]}>
              {t("ui.export.savedIn", { dir: dirs[0] })}
            </p>
          )}
        </div>
      </div>
      <ul className="export-files">
        {files.map((f) => (
          <li key={f.path}>
            <FileBadge ext={extOf(f.path)} />
            <span className="export-file-main">
              <b title={f.path}>{baseName(f.path)}</b>
              <small className="num">
                {f.label} · {bytes(f.bytes)}
              </small>
            </span>
            <button className="btn ghost small" onClick={() => void api.openPath(f.path)}>
              <Icon name="folder" size={14} /> {t("ui.transfer.showFile")}
            </button>
          </li>
        ))}
      </ul>
      {skipped && skipped.length > 0 && <p className="hint">{t("ui.export.skipped", { games: skipped.join("、") })}</p>}
      {hint && <p className="hint export-next">{hint}</p>}
    </div>
  );
}
