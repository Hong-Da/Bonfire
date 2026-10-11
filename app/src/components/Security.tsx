import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import logo from "../assets/logo.svg";
import { useStore } from "../store";
import type { SecurityStatus } from "../types";
import { Icon, Modal } from "./ui";

/** 加密仓库启动时的解锁页（M5-2）。 */
export function UnlockScreen({ onUnlocked }: { onUnlocked: () => void }) {
  const { t } = useTranslation();
  const [pw, setPw] = useState("");
  const [remember, setRemember] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [canRemember, setCanRemember] = useState(true);

  useEffect(() => {
    api.repoSecurity().then((s) => setCanRemember(s.canRemember)).catch(() => {});
  }, []);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.repoUnlock(pw, remember && canRemember);
      onUnlocked();
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <div className="wizard">
      <form className="card unlock" onSubmit={(e) => void submit(e)}>
        <div className="brand">
          <img src={logo} alt="" />
          <h1>{t("ui.security.lockedTitle")}</h1>
        </div>
        <p className="muted">{t("ui.security.lockedBody")}</p>
        <input className="field" type="password" autoFocus value={pw} onChange={(e) => setPw(e.target.value)} aria-label={t("ui.security.password")} placeholder={t("ui.security.password")} />
        {canRemember && (
          <label className="opt">
            <input type="checkbox" className="check" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
            {t("ui.security.rememberHint")}
          </label>
        )}
        {error && (
          <p className="callout error" role="alert">
            {error}
          </p>
        )}
        <div className="actions">
          <button className="btn primary" type="submit" disabled={!pw || busy}>
            <Icon name="shield" /> {busy ? t("ui.security.unlocking") : t("ui.security.unlock")}
          </button>
        </div>
        <p className="hint">{t("ui.security.forgotNote")}</p>
      </form>
    </div>
  );
}

type Dialog = "encrypt" | "change" | "decrypt" | null;

/** 设置页“仓库加密”一节。 */
export function SecuritySection() {
  const { t } = useTranslation();
  const store = useStore();
  const [s, setS] = useState<SecurityStatus | null>(null);
  const [dialog, setDialog] = useState<Dialog>(null);
  const reload = () => api.repoSecurity().then(setS).catch(() => setS(null));
  useEffect(() => {
    void reload();
  }, []);
  if (!s) return null;

  const toggleRemember = async (on: boolean) => {
    try {
      await api.repoRemember(on);
      await reload();
    } catch (e) {
      store.toast("error", errorMessage(e));
    }
  };

  return (
    <section id="settings-security">
      <h2>{t("ui.security.title")}</h2>
      <div className="row">
        <label>{t("ui.security.state")}</label>
        <div>
          <p className={`security-state${s.encrypted ? " on" : ""}`}>
            <Icon name="shield" size={16} /> {s.encrypted ? t("ui.security.on") : t("ui.security.off")}
          </p>
          <p className="hint">{s.encrypted ? t("ui.security.onHint") : t("ui.security.offHint")}</p>
        </div>
      </div>
      {s.encrypted && s.canRemember && (
        <div className="row">
          <label>{t("ui.security.remember")}</label>
          <label className="opt">
            <input type="checkbox" className="check" checked={s.remembered} onChange={(e) => void toggleRemember(e.target.checked)} />
            {t("ui.security.rememberHint")}
          </label>
        </div>
      )}
      <div className="inline" style={{ display: "flex", gap: 8 }}>
        {s.encrypted ? (
          <>
            <button className="btn" onClick={() => setDialog("change")}>
              <Icon name="shield" /> {t("ui.security.change")}
            </button>
            <button className="btn danger-ghost" onClick={() => setDialog("decrypt")}>
              {t("ui.security.decrypt")}
            </button>
          </>
        ) : (
          <button className="btn" onClick={() => setDialog("encrypt")}>
            <Icon name="shield" /> {t("ui.security.encrypt")}
          </button>
        )}
      </div>
      {dialog && (
        <PasswordDialog
          kind={dialog}
          canRemember={s.canRemember}
          onClose={() => setDialog(null)}
          onDone={async (msg) => {
            setDialog(null);
            store.toast("success", msg);
            await reload();
          }}
        />
      )}
    </section>
  );
}

function PasswordDialog({ kind, canRemember, onClose, onDone }: { kind: Exclude<Dialog, null>; canRemember: boolean; onClose: () => void; onDone: (msg: string) => void }) {
  const { t } = useTranslation();
  const [old, setOld] = useState("");
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [remember, setRemember] = useState(true);
  const [ack, setAck] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const needsNew = kind !== "decrypt";
  const short = needsNew && pw.length > 0 && pw.length < 6;
  const mismatch = needsNew && pw2.length > 0 && pw !== pw2;
  const ready = kind === "decrypt" ? old.length > 0 && ack : (kind === "change" ? old.length > 0 : ack) && pw.length >= 6 && pw === pw2;

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      if (kind === "encrypt") {
        await api.repoEncrypt(pw, remember && canRemember);
        onDone(t("ui.security.encrypted"));
      } else if (kind === "change") {
        await api.repoChangePassword(old, pw);
        onDone(t("ui.security.changed"));
      } else {
        await api.repoDecrypt(old);
        onDone(t("ui.security.decrypted"));
      }
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t(`ui.security.${kind}Title`)}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t("ui.cancel")}
          </button>
          <button className={`btn ${kind === "decrypt" ? "danger" : "primary"}`} onClick={() => void run()} disabled={!ready || busy}>
            {busy ? t("ui.security.working") : t(`ui.security.${kind}Ok`)}
          </button>
        </>
      }
    >
      <div className="security-form">
        {kind === "encrypt" && (
          <ul className="security-facts">
            <li>{t("ui.security.factWhat")}</li>
            <li>{t("ui.security.factPlain")}</li>
            <li>{t("ui.security.factTime")}</li>
          </ul>
        )}
        {kind === "decrypt" && <p>{t("ui.security.decryptBody")}</p>}
        {(kind === "change" || kind === "decrypt") && (
          <label>
            <span>{t(kind === "change" ? "ui.security.oldPassword" : "ui.security.password")}</span>
            <input className="field" type="password" value={old} onChange={(e) => setOld(e.target.value)} autoFocus />
          </label>
        )}
        {needsNew && (
          <>
            <label>
              <span>{t("ui.security.newPassword")}</span>
              <input className="field" type="password" value={pw} onChange={(e) => setPw(e.target.value)} autoFocus={kind === "encrypt"} />
            </label>
            {short && <p className="hint warn-hint">{t("ui.security.tooShort")}</p>}
            <label>
              <span>{t("ui.security.confirmPassword")}</span>
              <input className="field" type="password" value={pw2} onChange={(e) => setPw2(e.target.value)} />
            </label>
            {mismatch && <p className="hint warn-hint">{t("ui.security.mismatch")}</p>}
          </>
        )}
        {kind === "encrypt" && canRemember && (
          <label className="opt">
            <input type="checkbox" className="check" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
            {t("ui.security.rememberHint")}
          </label>
        )}
        {kind !== "change" && (
          <div className="callout error bd-safety" role="note">
            <b>{t(kind === "encrypt" ? "ui.security.warnTitle" : "ui.security.decryptWarnTitle")}</b>
            <span>{t(kind === "encrypt" ? "ui.security.warn" : "ui.security.decryptWarn")}</span>
            <label className="opt">
              <input type="checkbox" className="check" checked={ack} onChange={(e) => setAck(e.target.checked)} />
              {t(kind === "encrypt" ? "ui.security.ack" : "ui.security.decryptAck")}
            </label>
          </div>
        )}
        {error && (
          <p className="callout error" role="alert">
            {error}
          </p>
        )}
      </div>
    </Modal>
  );
}
