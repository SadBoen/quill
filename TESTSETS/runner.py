#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""逐条跑 TESTSETS/tasks.json 里的真实任务，打 quill 的界面与对话链路。

**这个脚本存在的理由**：`TESTSETS/README.md` 的「怎么跑」把「浏览器模拟真实用户」
列为**主要路子**，并指向 `runner.py` —— 而那个文件一直没有（ISSUE-017）。
文档指着一个不存在的东西，比明说「还没做」更坏：它让人以为只是没找到。
本文件就是那个 `runner.py`。

它驱动的是 **HTTP 契约层**（`/api/extensions/*` + `/api/sessions/*`），
不是浏览器。这么选有两条实在的理由，都写在下面而不是藏在代码里：

1. 界面那一层已经有 `ui/web` 的组件测试与 i18n 门禁盯着；**对话链路**这一层
   （技能挂没挂上、工具调没调到、正文吞没吞）只能在真服务上验，而真服务的
   契约就是这些 HTTP 路由。
2. 浏览器那一层需要人在登录页接管（凭据不归脚本管）。把它写进脚本，
   等于让脚本去处理密码 —— 那条线不能越。

**判定口径**（照抄 `TESTSETS/STATUS.md` 的「判定口径」，不自行发明）
每条任务记三件事，**互不替代**：

1. `link`   链路是否通 —— 请求有没有正常往返、界面有没有如实反映状态。
             这条不过就是 quill 的 bug。**这一维度任何时候都可判**。
2. `tools`  工具是否被调用 —— 该调的工具有没有出现在 `tool_calls` 轨迹里。
             **依赖的服务器没配时不可判**，这时必须报 `unjudgeable` 并写清原因，
             **不许**因为「没调工具」就判失败 —— 那会把「没配服务器」说成「quill 不行」。
3. `claims` 答案是否对不上 —— 用 `expected_claims` 粗判。本机是 4B 小模型，
             答不上来**不算 bug**；但「调了工具却把正文吞了」「明明没调工具却
             声称查过了」要算。`expected_claims` 为空时这一维度不判。

用法::

    python3 TESTSETS/runner.py                      # 全部 100 条
    python3 TESTSETS/runner.py --limit 3            # 先跑 3 条看看
    python3 TESTSETS/runner.py --only sb-edit-pdf   # 只跑某一条
    python3 TESTSETS/runner.py --source SkillsBench # 只跑某一段
    python3 TESTSETS/runner.py --dry-run            # 只读，不发任何写请求

**只用标准库。** 这台机器上 WSL 里没有 pip，装依赖不是一条能假设成立的路。
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
TASKS = os.path.join(HERE, "tasks.json")
SKILLS_DIR = os.path.join(HERE, "skills")
DEFAULT_OUT = os.path.join(HERE, "results.jsonl")

# 一条工具名形如 `<服务器>_<工具>`；SkillsBench 那批的 required_tools 是空数组
# 或对象数组，两种形状都要吃得下。
def tool_name(x) -> str:
    if isinstance(x, str):
        return x
    if isinstance(x, dict):
        for k in ("name", "tool", "id", "server"):
            v = x.get(k)
            if isinstance(v, str):
                return v
        return json.dumps(x, ensure_ascii=False)
    return str(x)


def tool_server(name: str) -> str | None:
    """从 `airtable_list_bases` 里取出 `airtable`。

    取不到就返回 None —— **不能**编一个出来。编出来的服务器名会让
    「这台没配」变成「这台配了但失败了」，那是凭空造数据。
    """
    m = re.match(r"^([a-z0-9]+)_", name or "")
    return m.group(1) if m else None


class Quillian:
    """极薄的 HTTP 客户端。只用标准库，且**每个响应都按 UTF-8 解码**。

    PowerShell 的 `Invoke-RestMethod` 会把 UTF-8 当 Latin-1 解，中文全变乱码 ——
    那是客户端的问题，不是服务端的。这里显式指定编码，免得把「界面乱码」
    误记成一条 ISSUE。
    """

    def __init__(self, base: str, token: str, timeout: int = 180):
        self.base = base.rstrip("/")
        self.token = token
        self.timeout = timeout

    def _req(self, method: str, path: str, body=None):
        data = None
        headers = {"Authorization": "Bearer " + self.token, "Accept": "application/json"}
        if body is not None:
            data = json.dumps(body, ensure_ascii=False).encode("utf-8")
            headers["Content-Type"] = "application/json; charset=utf-8"
        req = urllib.request.Request(self.base + path, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                raw = resp.read().decode("utf-8", "replace")
                return resp.status, (json.loads(raw) if raw.strip() else {})
        except urllib.error.HTTPError as e:
            raw = e.read().decode("utf-8", "replace")
            try:
                return e.code, json.loads(raw)
            except json.JSONDecodeError:
                return e.code, {"error": {"code": "non_json", "detail": raw[:500]}}
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            # 连接层失败：原样带出去，不改写。改写过的错误没法用来排查。
            return 0, {"error": {"code": "transport", "detail": str(e)}}

    def get(self, path):
        return self._req("GET", path)

    def post(self, path, body):
        return self._req("POST", path, body)


def err_detail(payload) -> str:
    """把错误信封压成一行**原文**。不重写、不美化 ——
    排查要靠原文，改写过的错误会把人引到错误的分支上。"""
    if not isinstance(payload, dict):
        return str(payload)[:300]
    e = payload.get("error")
    if isinstance(e, dict):
        parts = [str(e.get("code") or ""), str(e.get("detail") or "")]
        nxt = e.get("next_step")
        if nxt:
            parts.append("下一步：" + str(nxt))
        return " / ".join(p for p in parts if p)[:600]
    return json.dumps(payload, ensure_ascii=False)[:300]


# ---------------------------------------------------------------- 技能与服务器

def load_skill(slug: str) -> tuple[str | None, str]:
    """读一份真实 SKILL.md 正文。读不到就返回 (None, 原因) ——
    **不许**拿占位文字顶上：挂一个空正文的技能，模型看到的是「这个技能没内容」，
    而界面上它显示为已启用，那是凭空造数据。"""
    path = os.path.join(SKILLS_DIR, slug + ".md")
    if not os.path.exists(path):
        return None, "TESTSETS/skills/%s.md 不存在" % slug
    try:
        with open(path, encoding="utf-8") as f:
            body = f.read()
    except OSError as e:
        return None, "读 %s 失败：%s" % (path, e)
    if not body.strip():
        return None, "%s 是空的" % path
    return body, ""


def ensure_skill(q: Quillian, slug: str, description: str, dry: bool) -> dict:
    """把技能灌进去，并**核实模型这一轮看不看得见**。

    返回一个 dict 供记结果。`model_can_see` 为假时**不重试** ——
    重试只会把同一个原因再报一遍。
    """
    body, why = load_skill(slug)
    if body is None:
        return {"slug": slug, "installed": False, "model_can_see": False, "problem": why}
    if dry:
        return {"slug": slug, "installed": None, "model_can_see": None, "problem": "dry-run 未发写请求"}

    st, payload = q.post(
        "/api/extensions/skills",
        {"slug": slug, "description": description, "content": body},
    )
    if st not in (200, 201):
        return {
            "slug": slug,
            "installed": False,
            "model_can_see": False,
            "problem": "导入失败 HTTP %s：%s" % (st, err_detail(payload)),
        }

    st, listing = q.get("/api/extensions/skills")
    if st != 200:
        return {
            "slug": slug,
            "installed": True,
            "model_can_see": False,
            "problem": "回读技能列表失败 HTTP %s：%s" % (st, err_detail(listing)),
        }
    for s in listing.get("skills", []):
        if s.get("slug") == slug or s.get("name") == slug:
            can_see = bool(s.get("model_can_see"))
            problem = "" if can_see else ("没挂进对话的工具表：%s" % (s.get("not_mounted_reason") or "（没给原因）"))
            return {"slug": slug, "installed": True, "model_can_see": can_see, "problem": problem}
    return {"slug": slug, "installed": True, "model_can_see": False, "problem": "列表里找不到这一条"}


def configured_servers(q: Quillian, dry: bool) -> dict:
    """当前真的配好且挂上工具的 MCP 服务器：`{名字: 挂上的工具数}`。

    用 `mounted` 而不是 `tool_count` —— 后者是服务器自己报了几个，
    前者才是「模型这一轮真调得到几个」。两者能差很远（能力被关、撞名、
    服务器自报没有 tools 能力），拿错一个就会把「界面说能用」当成事实。
    """
    if dry:
        return {}
    st, listing = q.get("/api/extensions/mcp")
    if st != 200:
        return {}
    out = {}
    for row in listing.get("status", []) or []:
        out[row.get("name")] = row.get("mounted", 0)
    return out


# ---------------------------------------------------------------- 判定

def judge(task: dict, chat: dict, need_servers: dict, servers: dict,
          skills: list, claims) -> dict:
    """给一条任务出结论。**三个维度分开，谁也不替谁说话。**"""
    res = {
        "link": {"ok": False, "why": ""},
        "tools": {"judgeable": False, "called": [], "missing_servers": [], "why": ""},
        "claims": {"judgeable": False, "hit": 0, "total": 0, "why": ""},
        "suspicions": [],
    }

    # 1) 链路
    if chat.get("__http") == 0:
        res["link"]["why"] = "连不上服务：%s" % err_detail(chat)
    elif chat.get("error"):
        res["link"]["why"] = "HTTP %s：%s" % (chat.get("__http"), err_detail(chat))
    else:
        res["link"]["ok"] = True

    # 2) 工具
    called = [c.get("name") for c in (chat.get("tool_calls") or [])]
    res["tools"]["called"] = called
    missing = sorted(s for s in need_servers if s not in servers)
    res["tools"]["missing_servers"] = missing
    if not need_servers:
        res["tools"]["judgeable"] = False
        res["tools"]["why"] = "这条任务不依赖任何外部服务器工具"
    elif missing:
        res["tools"]["judgeable"] = False
        res["tools"]["why"] = "依赖的服务器没配：%s —— 「模型该不该调它」无法判定" % "、".join(missing)
    else:
        res["tools"]["judgeable"] = True
        res["tools"]["why"] = "依赖的服务器都配好了"

    # 3) 答案
    if not claims:
        res["claims"]["why"] = "expected_claims 为空，这一维度不判"
    else:
        res["claims"]["judgeable"] = True
        res["claims"]["total"] = len(claims)
        reply = (chat.get("reply") or "").lower()
        hit = 0
        for c in claims:
            # 粗判：把断言里最像「答案」的数字与专有名词抽出来看有没有出现。
            # 4B 小模型逐字复述断言原文是不现实的，所以只看「关键值」。
            keys = extract_keys(c)
            if not keys:
                hit += 1
                continue
            if any(k in reply for k in keys):
                hit += 1
        res["claims"]["hit"] = hit

    # 4) 这两类是 quill 自己的 bug，与 4B 答不上来无关
    reply = chat.get("reply") or ""
    if called and not reply.strip():
        res["suspicions"].append("调了工具却把正文吞了（tool_calls 有 %d 条，reply 为空）" % len(called))
    if claims and not called and reply.strip() and res["claims"]["hit"] > 0:
        res["suspicions"].append("没调任何工具却把断言里的关键值说对了 —— 要么它在编，要么服务器其实被调过而轨迹没记")
    for s in skills:
        if s.get("installed") and not s.get("model_can_see"):
            res["suspicions"].append("技能 %s 已导入但模型这一轮看不见：%s" % (s.get("slug"), s.get("problem")))
    return res


def extract_keys(claim: str) -> list:
    """从一条标准答案里抽出「答对了就一定会出现」的短串。

    只取**数字**与**像标识符/专名的长词**。取整句话去比对是没意义的 ——
    4B 不会逐字复述标准答案，而它抄对了那个数字才是真的答上了。
    """
    keys = re.findall(r"\d[\d,]*\.?\d*", claim)
    words = re.findall(r"\b[A-Z][A-Za-z0-9_]{3,}\b", claim)
    out = []
    for k in keys + words:
        k = k.strip().lower()
        # 单个数字太短（「0」「1」）容易在正文里到处撞上，不算数
        if len(k) >= 2 and k not in out:
            out.append(k)
    return out[:8]


def verdict(res: dict) -> str:
    """总判定。

    **只有三个维度都判过且都过，才算 PASS。**

    这里最容易写错的一处：把「没有维度判过」当成「没有维度失败」。那样一条
    既不需要外部服务器、`expected_claims` 又为空的任务会直接拿到 PASS ——
    而它其实**什么都没验**。SkillsBench 里有相当一批正是这种任务
    （`sb-3d-scan-calc` 就是），写成那样的话 100 条里会凭空多出一堆 PASS。
    所以分三种情况，一个都不许合并：

    - 一个维度都没判过 → `UNJUDGEABLE`（什么都没验，不是验过了）
    - 判过且有不过的   → `PARTIAL`
    - 命中原项目认定的 bug（正文被吞等）→ `FAIL`
    """
    if not res["link"]["ok"]:
        return "FAIL"
    if res["suspicions"]:
        return "FAIL"

    judged = 0
    failed = []
    if res["tools"]["judgeable"]:
        judged += 1
        if not res["tools"]["called"]:
            failed.append("该调的工具一个都没调")
    if res["claims"]["judgeable"]:
        judged += 1
        if res["claims"]["hit"] < res["claims"]["total"]:
            failed.append("断言 %d/%d 命中" % (res["claims"]["hit"], res["claims"]["total"]))

    if failed:
        return "PARTIAL"
    if judged == 0:
        # 链路通，但没有任何一维可判 —— 报「没验」，不报「过了」。
        return "UNJUDGEABLE"
    return "PASS"


# ---------------------------------------------------------------- 主流程

def run_task(q: Quillian, task: dict, dry: bool) -> dict:
    tid = task["id"]
    need = sorted({s for s in (tool_server(tool_name(x)) for x in (task.get("required_tools") or [])) if s})
    servers = configured_servers(q, dry)

    # 技能：先全灌好，再看它们挂没挂上。挂不上是 quill 的问题，记下来。
    skills = []
    for s in task.get("skills") or []:
        skills.append(ensure_skill(q, s["slug"], s.get("description", ""), dry))
    unseeable = [s["slug"] for s in skills if s.get("installed") and not s.get("model_can_see")]

    if dry:
        chat = {"__http": 0, "error": {"code": "dry_run", "detail": "dry-run 没发这条消息"}}
    else:
        st, session = q.post("/api/sessions", {"title": "runner: " + tid})
        if st not in (200, 201) or not session.get("id"):
            chat = {"__http": st, "error": session.get("error") or {"detail": "建会话没拿到 id"}}
        else:
            st2, chat = q.post(
                "/api/sessions/%s/messages" % session["id"],
                {"content": task["prompt"]},
            )
            chat["__http"] = st2
            chat["__session"] = session["id"]

    res = judge(task, chat, need, servers, skills, task.get("expected_claims") or [])
    return {
        "id": tid,
        "source": task.get("source"),
        "at": int(time.time() * 1000),
        "dry_run": dry,
        "verdict": "DRY-RUN" if dry else verdict(res),
        "need_servers": need,
        "mounted_servers": {k: v for k, v in servers.items() if k in need},
        "skills": skills,
        "skills_unseeable": unseeable,
        "turn_ms": chat.get("turn_ms"),
        "tool_rounds": chat.get("tool_rounds"),
        "tool_calls": [
            {"name": c.get("name"), "ok": c.get("ok"), "arguments": c.get("arguments")}
            for c in (chat.get("tool_calls") or [])
        ],
        "reply_chars": len(chat.get("reply") or ""),
        "judgement": res,
        "error": err_detail(chat) if chat.get("error") else None,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description="逐条跑 TESTSETS/tasks.json 里的真实任务")
    ap.add_argument("--base", default=os.environ.get("QUILL_BASE", "http://127.0.0.1:8848"))
    ap.add_argument("--token", default=os.environ.get("QUILL_TOKEN", "dev-token"))
    ap.add_argument("--out", default=DEFAULT_OUT)
    ap.add_argument("--limit", type=int, default=0, help="只跑前 N 条；0 = 全部")
    ap.add_argument("--only", action="append", default=[], help="只跑这些 id，可重复")
    ap.add_argument("--source", choices=["MCP-Atlas", "SkillsBench"], help="只跑某一段")
    ap.add_argument("--dry-run", action="store_true", help="只读，不发任何写请求")
    args = ap.parse_args()

    with open(TASKS, encoding="utf-8") as f:
        tasks = json.load(f)
    if args.only:
        want = set(args.only)
        tasks = [t for t in tasks if t["id"] in want]
    if args.source:
        tasks = [t for t in tasks if t.get("source") == args.source]
    if args.limit:
        tasks = tasks[: args.limit]
    if not tasks:
        print("没有匹配到任务。", file=sys.stderr)
        return 2

    q = Quillian(args.base, args.token)
    st, health = q.get("/healthz")
    if st != 200:
        print("服务不可达（%s）：%s" % (args.base, err_detail(health)), file=sys.stderr)
        print("下一步：先按 STATUS.md 的「跑之前要知道的三件事」把服务起起来。", file=sys.stderr)
        return 2
    print("服务：%s  status=%s  模型=%s" % (args.base, health.get("status"),
                                          (health.get("llm") or {}).get("base_url")))
    if args.dry_run:
        print("dry-run：不发任何写请求，结论一律记 DRY-RUN。")
    print("要跑 %d 条。\n" % len(tasks))

    results = []
    counts = {}
    t0 = time.time()
    for i, task in enumerate(tasks, 1):
        print("[%d/%d] %s (%s) …" % (i, len(tasks), task["id"], task.get("source")), end=" ", flush=True)
        try:
            r = run_task(q, task, args.dry_run)
        except Exception as e:  # 一条崩了不许带走整轮
            r = {
                "id": task["id"], "source": task.get("source"),
                "at": int(time.time() * 1000), "verdict": "ERROR",
                "error": "runner 自己抛异常：%r" % (e,),
            }
        results.append(r)
        counts[r["verdict"]] = counts.get(r["verdict"], 0) + 1
        j = r.get("judgement") or {}
        tools = j.get("tools") or {}
        claims = j.get("claims") or {}
        bits = ["%s" % r["verdict"]]
        if tools.get("judgeable"):
            bits.append("工具 %d 条" % len(tools.get("called") or []))
        elif tools.get("missing_servers"):
            bits.append("服务器没配 %s" % "、".join(tools["missing_servers"]))
        if claims.get("judgeable"):
            bits.append("断言 %d/%d" % (claims["hit"], claims["total"]))
        for s in (j.get("suspicions") or []):
            bits.append("可疑：" + s)
        if r.get("error"):
            bits.append("err=" + r["error"][:80])
        print("  " + " | ".join(bits))

    mode = "a" if (os.path.exists(args.out) and not args.dry_run) else "w"
    with open(args.out, mode, encoding="utf-8") as f:
        for r in results:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")

    print("\n===== 汇总（%d 条，用了 %.1f 秒）=====" % (len(results), time.time() - t0))
    for k in sorted(counts):
        print("  %-12s %d" % (k, counts[k]))
    print("  结果已写入 %s" % args.out)
    done = counts.get("PASS", 0) + counts.get("PARTIAL", 0) + counts.get("FAIL", 0)
    print(
        "\n判定口径（照抄 STATUS.md，三件事不混为一谈）：\n"
        "  PASS         三个维度都判过且都过。\n"
        "  PARTIAL      链路通，但有维度判过且没过（该调的工具没调、断言没全中）。\n"
        "  FAIL         链路不通，或命中「调了工具却把正文吞了」这类 quill 自己的 bug。\n"
        "  UNJUDGEABLE  链路通，但**没有任何一维可判** —— 什么都没验，不是验过了。\n"
        "\n真的跑完的条数 = PASS + PARTIAL + FAIL = %d（UNJUDGEABLE 不算跑过）\n"
        "依赖的服务器没配时，工具那一维度记 unjudgeable 并写清缺哪几台 ——\n"
        "**那不是 quill 的问题，也不算跑过。**" % done
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
