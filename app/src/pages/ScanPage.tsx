import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { Cover, Icon, displayName } from "../components/ui";
import { bytes, hours } from "../lib/format";
import { useStore } from "../store";
import type { DiscoveredGame, FoundState } from "../types";

type Filter = "all" | FoundState;
const FILTERS: Filter[] = ["new", "managed", "ignored", "all"];

/**
 * 发现游戏：扫描本机平台上的游戏与存档，由用户挑选加入游戏库或忽略。
 * 扫描本身不改动游戏库；默认一个都不勾选。
 */
export default function ScanPage() {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const { scan, scanning, scanPhase, startScan } = store;
  const [filter, setFilter] = useState<Filter>("new");
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);

  const games = scan?.games ?? [];
  const counts = useMemo(() => {
    const c: Record<Filter, number> = { all: games.length, new: 0, managed: 0, ignored: 0 };
    for (const g of games) c[g.state]++;
    return c;
  }, [games]);
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return games
      .filter((g) => filter === "all" || g.state === filter)
      .filter((g) => !q || g.name.toLowerCase().includes(q) || (g.nameZh ?? "").toLowerCase().includes(q));
  }, [games, filter, query]);
  const pickedGames = games.filter((g) => picked.has(g.id));
  const canInclude = pickedGames.some((g) => g.state !== "managed");
  const canIgnore = pickedGames.some((g) => g.state !== "ignored");

  const toggle = (id: string) =>
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const act = async (kind: "include" | "ignore") => {
    const targets = pickedGames.filter((g) => (kind === "include" ? g.state !== "managed" : g.state !== "ignored"));
    if (targets.length === 0) return;
    setBusy(true);
    try {
      const ids = targets.map((g) => g.id);
      if (kind === "include") await api.includeGames(ids);
      else await api.ignoreGames(ids);
      const state: FoundState = kind === "include" ? "managed" : "ignored";
      useStore.setState((s) => ({ scan: s.scan && { ...s.scan, games: s.scan.games.map((g) => (ids.includes(g.id) ? { ...g, state } : g)) } }));
      setPicked(new Set());
      store.toast("success", t(kind === "include" ? "ui.scan.included" : "ui.scan.ignored", { n: ids.length }));
      // 加入后在游戏库中检测状态并联网补全游戏信息。
      if (kind === "include" || targets.some((g) => g.state === "managed")) void store.loadLibrary(true);
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div className="toolbar">
        <h1>{t("ui.scan.title")}</h1>
        {scan && (
          <button className="btn" onClick={() => void startScan()} disabled={scanning}>
            <Icon name="scan" /> {scanning ? t(`ui.scan.phase.${scanPhase}`) : t("ui.scan.rescan")}
          </button>
        )}
        {scan && (
          <div className="chips" role="group" aria-label={t("ui.library.filterState")}>
            {FILTERS.map((f) => (
              <button key={f} className="chip" aria-pressed={filter === f} onClick={() => setFilter(f)}>
                {t(`ui.scan.filter.${f}`)}
                <span className="num chip-count">{counts[f]}</span>
              </button>
            ))}
          </div>
        )}
        <div className="grow" />
        {scan && <input className="field" type="search" placeholder={t("ui.library.search")} value={query} onChange={(e) => setQuery(e.target.value)} aria-label={t("ui.library.search")} />}
      </div>
      <div className="progress-line" aria-hidden>
        <span style={{ width: scanning ? (scanPhase === "rules" ? "30%" : "70%") : "0%" }} />
      </div>

      <div className="library">
        {!scan ? (
          <div className="empty">
            <h2>{t("ui.scan.introTitle")}</h2>
            <p>{t("ui.scan.introBody")}</p>
            <div className="actions">
              <button className="btn primary" onClick={() => void startScan()} disabled={scanning}>
                <Icon name="scan" /> {scanning ? t(`ui.scan.phase.${scanPhase}`) : t("ui.scan.start")}
              </button>
            </div>
          </div>
        ) : (
          <>
            {scan.platforms.length === 0 && <div className="callout warn" style={{ margin: "12px 20px" }}>{t("ui.scan.noPlatforms")}</div>}
            {scan.rules && !scan.rules.path && <div className="callout warn" style={{ margin: "12px 20px" }}>{t("ui.scan.noRules")}</div>}
            <ul className="game-list">
              {visible.map((g) => (
                <FoundLine key={g.id} game={g} lang={i18n.language} picked={picked.has(g.id)} onToggle={() => toggle(g.id)} />
              ))}
              {visible.length === 0 && (
                <li className="empty">
                  <p>{filter === "new" && games.length > 0 ? t("ui.scan.noneNew") : t("ui.library.noMatch")}</p>
                </li>
              )}
            </ul>
            {scan.withoutSaves.length > 0 && (
              <details className="no-saves">
                <summary>{t("ui.scan.withoutSaves", { n: scan.withoutSaves.length })}</summary>
                <ul className="file-list">
                  {scan.withoutSaves.map((g) => (
                    <li key={g.id}>
                      <span />
                      <span className="path">{g.name}</span>
                      <span className="muted">{g.rule ? t("ui.scan.noSaveYet") : t("ui.scan.noRule")}</span>
                    </li>
                  ))}
                </ul>
                <p className="hint">{t("ui.scan.withoutSavesHint")}</p>
              </details>
            )}
          </>
        )}
      </div>

      {scan && games.length > 0 && (
        <div className="actionbar">
          <button className="btn ghost small" onClick={() => setPicked(new Set(games.filter((g) => g.state === "new").map((g) => g.id)))} disabled={counts.new === 0}>
            {t("ui.scan.pickAllNew")}
          </button>
          <button className="btn ghost small" onClick={() => setPicked(new Set())} disabled={picked.size === 0}>
            {t("ui.scan.clearPick")}
          </button>
          <div className="grow" style={{ flex: 1 }} />
          <span className="count num">{t("ui.library.selected", { n: picked.size })}</span>
          <button className="btn" onClick={() => void act("ignore")} disabled={busy || !canIgnore}>
            {t("ui.scan.ignore")}
          </button>
          <button className="btn primary" onClick={() => void act("include")} disabled={busy || !canInclude}>
            <Icon name="plus" /> {t("ui.scan.include")}
          </button>
        </div>
      )}
    </>
  );
}

function FoundLine({ game, lang, picked, onToggle }: { game: DiscoveredGame; lang: string; picked: boolean; onToggle: () => void }) {
  const { t } = useTranslation();
  const name = displayName(game, lang);
  const first = game.locations[0] ?? "";
  const shortPath = first.split(/[/\\]/).slice(-3).join("/");
  return (
    <li className={`game found${picked ? " selected" : ""}`}>
      <input type="checkbox" className="check" checked={picked} onChange={onToggle} aria-label={t("ui.library.selectGame", { name })} />
      <Cover row={game} />
      <div className="title" onClick={onToggle}>
        <strong>{name}</strong>
        <span className="sub">
          <span>{t(`ui.platform.${game.platform}`, { defaultValue: game.platform })}</span>
          {name !== game.name && <span>{game.name}</span>}
          {game.playtimeMin != null && game.playtimeMin > 0 && <span>{t("ui.history.playtime", { h: hours(game.playtimeMin) })}</span>}
        </span>
      </div>
      <div className="meta" title={game.locations.join("\n")}>
        <b className="path-short">{shortPath}</b>
        {game.locations.length > 1 && <div>{t("ui.scan.moreLocations", { n: game.locations.length - 1 })}</div>}
      </div>
      <div className="meta">
        {game.rule ? <span>{t("ui.scan.fromRules")}</span> : <span className="badge pending">{t("ui.scan.guessed")}</span>}
      </div>
      <div className="meta num col-size">
        <b>{bytes(game.bytes)}</b>
        <div>{t("ui.library.files", { n: game.files })}</div>
      </div>
      <span className={`tag found-${game.state}`}>{t(`ui.scan.state.${game.state}`)}</span>
    </li>
  );
}
