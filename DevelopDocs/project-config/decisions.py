"""决策记录：追加决策到 DevelopDocs/project-config/decisions.json，并生成 decisions.html。

用法：python decisions.py <项目根> [<新增决策的 JSON 文件>] [--open]
新增文件格式：[{"id": "M2-D1", "stage": "M2", "question": "...", "options": ["...", "..."],
               "chosen": "...", "reason": "...", "impact": "..."}]
同 id 的决策会被覆盖（便于修订）。
"""
import datetime
import html
import json
import os
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1])
cfg = root / "DevelopDocs" / "project-config"
store = cfg / "decisions.json"
data = json.loads(store.read_text(encoding="utf-8")) if store.exists() else []

args = [a for a in sys.argv[2:] if not a.startswith("--")]
if args:
    new = json.loads(Path(args[0]).read_text(encoding="utf-8"))
    today = datetime.date.today().isoformat()
    by_id = {d["id"]: i for i, d in enumerate(data)}
    for d in new:
        d.setdefault("date", today)
        d.setdefault("status", "按默认推荐自动决定，待复核")
        if d["id"] in by_id:
            data[by_id[d["id"]]] = d
        else:
            by_id[d["id"]] = len(data)
            data.append(d)
    store.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")

e = html.escape
stages = []
for d in data:
    if d["stage"] not in stages:
        stages.append(d["stage"])

cards = []
for st in stages:
    items = [d for d in data if d["stage"] == st]
    rows = []
    for d in items:
        opts = "".join(
            f'<li class="{"chosen" if o == d["chosen"] else ""}">{e(o)}{" <b>✓ 采用</b>" if o == d["chosen"] else ""}</li>'
            for o in d.get("options", [])
        )
        if d["chosen"] not in d.get("options", []):
            opts += f'<li class="chosen">{e(d["chosen"])} <b>✓ 采用</b></li>'
        impact = f'<p class="impact"><span>影响</span>{e(d["impact"])}</p>' if d.get("impact") else ""
        rows.append(f"""
      <article id="{e(d['id'])}">
        <header><code>{e(d['id'])}</code><h3>{e(d['question'])}</h3><span class="status">{e(d['status'])}</span></header>
        <ul>{opts}</ul>
        <p class="reason"><span>理由</span>{e(d['reason'])}</p>{impact}
        <footer>{e(d['date'])}</footer>
      </article>""")
    cards.append(f'<section><h2>{e(st)}<small>{len(items)} 项</small></h2>{"".join(rows)}</section>')

nav = "".join(f'<a href="#s-{i}">{e(s)}</a>' for i, s in enumerate(stages))
cards = [c.replace("<section>", f'<section id="s-{i}">', 1) for i, c in enumerate(cards)]
page = f"""<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>篝火 决策记录</title>
<style>
:root {{ --accent:#e8590c; --bg:#faf7f4; --card:#fff; --text:#1f1a17; --muted:#6b625c; --border:#e7dfd8; --ok:#2f7d4f; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#161210; --card:#211b18; --text:#f3ece6; --muted:#a99d94; --border:#3a302b; --ok:#6cc08c; }} }}
* {{ box-sizing:border-box }}
body {{ margin:0; background:var(--bg); color:var(--text); font:15px/1.6 "Segoe UI","Microsoft YaHei UI",system-ui,sans-serif }}
.wrap {{ max-width:960px; margin:0 auto; padding:24px 16px 64px }}
h1 {{ margin:0 0 4px; font-size:24px }} .sub {{ color:var(--muted); margin:0 0 16px }}
nav {{ display:flex; flex-wrap:wrap; gap:8px; margin:0 0 24px; position:sticky; top:0; background:var(--bg); padding:8px 0; z-index:1 }}
nav a {{ text-decoration:none; color:var(--text); border:1px solid var(--border); border-radius:999px; padding:2px 12px; font-size:13px }}
nav a:hover {{ border-color:var(--accent) }}
h2 {{ font-size:18px; border-bottom:2px solid var(--accent); padding-bottom:4px; margin:32px 0 12px }}
h2 small {{ color:var(--muted); font-weight:normal; font-size:13px; margin-left:8px }}
article {{ background:var(--card); border:1px solid var(--border); border-radius:10px; padding:14px 16px; margin:0 0 12px }}
article header {{ display:flex; flex-wrap:wrap; align-items:baseline; gap:8px }}
article h3 {{ margin:0; font-size:16px; flex:1 1 300px }}
code {{ color:var(--accent); font-size:13px }}
.status {{ font-size:12px; color:var(--muted); border:1px dashed var(--border); border-radius:6px; padding:0 6px }}
ul {{ margin:8px 0; padding-left:20px }} li {{ color:var(--muted) }} li.chosen {{ color:var(--text) }} li.chosen b {{ color:var(--ok); font-size:12px }}
.reason span, .impact span {{ display:inline-block; font-size:12px; color:#fff; background:var(--accent); border-radius:4px; padding:0 6px; margin-right:8px }}
.impact span {{ background:var(--muted) }}
p {{ margin:6px 0 }} footer {{ font-size:12px; color:var(--muted) }}
</style></head><body><div class="wrap">
<h1>篝火 Bonfire · 决策记录</h1>
<p class="sub">开发过程中需要确认的事项均按默认推荐方案自动决定，记录于此供复核。共 {len(data)} 项，更新于 {datetime.datetime.now():%Y-%m-%d %H:%M}。需要改动的项请在对话中告诉 Claude（附编号）。</p>
<nav>{nav}</nav>
{"".join(cards)}
</div></body></html>
"""
out = cfg / "decisions.html"
out.write_text(page, encoding="utf-8", newline="\n")
print(f"{len(data)} 项决策 → {out}")
if "--open" in sys.argv and os.name == "nt":
    subprocess.run(["cmd", "/c", "start", "", str(out)], check=False)
