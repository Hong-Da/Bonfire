import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { useStore } from "../store";
import { Icon, Modal } from "./ui";

/**
 * 设为本机主存档（U-41）：主存档只是“家”，不改动磁盘上的存档；可选同时恢复到它的最新快照，
 * 以及整理名称（选的不是 main 且 main 存在时：选中的改为 main，原来的 main 改为 main-本机电脑名）。
 */
export function PrimaryDialog({
  gameId,
  branch,
  branches,
  host,
  onClose,
  onDone,
}: {
  gameId: string;
  branch: string;
  branches: string[];
  host: string;
  onClose: () => void;
  /** `restore` 为需要恢复到的路线名（选了“同时恢复”时）。 */
  onDone: (restore: string | null) => void;
}) {
  const { t } = useTranslation();
  const store = useStore();
  const canTidy = branch !== "main" && branches.includes("main");
  // 导入时改名的路线（main-旧电脑）默认整理名称；普通路线默认只移动标记。
  const [tidy, setTidy] = useState(canTidy && branch.startsWith("main-"));
  const [restore, setRestore] = useState(false);
  const [busy, setBusy] = useState(false);
  const aside = `main-${host.replace(/[^\p{L}\p{N}_-]+/gu, "-").replace(/^-+|-+$/g, "") || "PC"}`;

  const confirm = async () => {
    setBusy(true);
    try {
      const r = await api.setPrimaryBranch(gameId, branch, canTidy && tidy);
      store.toast("success", t("ui.primary.done", { name: r.branch }));
      onDone(restore ? r.branch : null);
      onClose();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("ui.primary.title", { name: branch })}
      subtitle={t("ui.primary.subtitle")}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t("ui.cancel")}
          </button>
          <button className="btn primary" onClick={() => void confirm()} disabled={busy}>
            <Icon name="home" size={16} /> {t("ui.primary.confirm")}
          </button>
        </>
      }
    >
      <div className="primary-options">
        {canTidy && (
          <label className="opt">
            <input type="checkbox" className="check" checked={tidy} onChange={(e) => setTidy(e.target.checked)} />
            <span>
              <b>{t("ui.primary.tidy")}</b>
              <span className="rename-preview num">
                <code>{branch}</code> → <code>main</code>
                <br />
                <code>main</code> → <code>{aside}</code>
              </span>
              <small>{t("ui.primary.tidyHint")}</small>
            </span>
          </label>
        )}
        <label className="opt">
          <input type="checkbox" className="check" checked={restore} onChange={(e) => setRestore(e.target.checked)} />
          <span>
            <b>{t("ui.primary.restore")}</b>
            <small>{t("ui.primary.restoreHint")}</small>
          </span>
        </label>
      </div>
    </Modal>
  );
}

/** 路线改名：只换名字，快照不动。 */
export function RenameBranchDialog({ gameId, branch, onClose, onDone }: { gameId: string; branch: string; onClose: () => void; onDone: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [name, setName] = useState(branch);
  const [busy, setBusy] = useState(false);
  const next = name.trim();

  const confirm = async () => {
    setBusy(true);
    try {
      await api.renameBranch(gameId, branch, next);
      store.toast("success", t("ui.branch.renamed", { from: branch, to: next }));
      onDone();
      onClose();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("ui.branch.renameTitle", { name: branch })}
      subtitle={t("ui.branch.renameHint")}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t("ui.cancel")}
          </button>
          <button className="btn primary" onClick={() => void confirm()} disabled={busy || !next || next === branch}>
            {t("ui.branch.rename")}
          </button>
        </>
      }
    >
      <div className="row">
        <label htmlFor="branch-name">{t("ui.branch.newName")}</label>
        <input
          id="branch-name"
          className="field"
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && next && next !== branch && void confirm()}
        />
      </div>
    </Modal>
  );
}
