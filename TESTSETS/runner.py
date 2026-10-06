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
import pathlib
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

    def __init__(self, base: str, token: str, timeout: int = 600):
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


# ---------------------------------------------------------------- 任务隔离
#
# 下面这一段是被 ISSUE-025 逼出来的。
#
# **原来只有「挂」没有「撤」**：每条任务只把自己需要的技能灌进去，
# 上一条留下的一个都不动。于是实测到一条 MCP-Atlas 任务里，
# 模型去调了 SkillsBench 的 `csv-processing` —— 那条任务根本不需要任何技能。
# 双向受害：撑大每条请求的输入（放大 ISSUE-019），以及把模型带偏。
#
# 所以现在每条任务跑之前，把配置**收敛到这条真正需要的集合**，
# 整轮跑完再**原样恢复**。刻意不做「用完就删」：那会毁掉用户真实的配置，
# 这即便是专用测试库也该守住的边界。

def snapshot_enabled(q: Quillian, dry: bool) -> dict:
    """跑之前把当前启用状态记下来，跑完好原样还回去。"""
    if dry:
        return {"skills": {}, "servers": {}}
    skills, servers = {}, {}
    st, payload = q.get("/api/extensions/skills")
    if st == 200:
        for s in payload.get("skills") or []:
            slug = s.get("slug") or s.get("name")
            if slug:
                skills[slug] = bool(s.get("enabled", True))
    st, payload = q.get("/api/extensions/mcp")
    if st == 200:
        for row in payload.get("servers") or []:
            if row.get("name"):
                servers[row["name"]] = bool(row.get("enabled", True))
    return {"skills": skills, "servers": servers}


def set_skill_enabled(q: Quillian, slug: str, enabled: bool, description: str = "") -> tuple[bool, str]:
    """改一个技能的启用状态。

    POST 是**全量 upsert 且 content 不能为空**，所以停用也得把正文带上。
    正文不在磁盘上（不是 `TESTSETS/skills` 里那份）时**如实报失败**，
    不硬塞占位文字 —— 那是凭空造数据。
    """
    body, why = load_skill(slug)
    if body is None:
        return False, why
    st, payload = q.post(
        "/api/extensions/skills",
        {"slug": slug, "description": description, "content": body, "enabled": enabled},
    )
    if st in (200, 201):
        return True, ""
    return False, "HTTP %s：%s" % (st, err_detail(payload))


def set_servers_enabled(q: Quillian, wanted: dict) -> tuple[bool, str]:
    """按 `wanted`（名字 → 要不要启用）调整 MCP 服务器。

    MCP 那边是**全量 POST**：读回 `servers` 整份、按 `wanted` 改 `enabled`、
    再整份发回去 —— 没有 PATCH 可用。
    """
    if not wanted:
        return True, ""
    st, payload = q.get("/api/extensions/mcp")
    if st != 200:
        return False, "读 MCP 列表 HTTP %s：%s" % (st, err_detail(payload))
    servers = payload.get("servers") or []
    changed = False
    for row in servers:
        name = row.get("name")
        if name in wanted and bool(row.get("enabled", True)) != wanted[name]:
            row["enabled"] = wanted[name]
            changed = True
    if not changed:
        return True, ""
    st, payload = q.post("/api/extensions/mcp", {"servers": servers})
    if st in (200, 201):
        return True, ""
    return False, "HTTP %s：%s" % (st, err_detail(payload))


def isolate_for(q: Quillian, task: dict, dry: bool) -> dict:
    """把配置收敛到**这条任务真正需要的集合**，并如实报出收不干净的部分。"""
    if dry:
        return {"skills_off": [], "servers": {}, "problem": "dry-run 未发写请求"}

    want_skills = {s["slug"] for s in (task.get("skills") or []) if s.get("slug")}
    want_servers = {
        s for s in (tool_server(tool_name(x)) for x in (task.get("required_tools") or [])) if s
    }
    problems, off = [], []

    # 技能：不属于这条任务的，一律停用。
    st, payload = q.get("/api/extensions/skills")
    if st == 200:
        for s in payload.get("skills") or []:
            slug = s.get("slug") or s.get("name")
            if not slug or slug in want_skills or not s.get("enabled", True):
                continue
            ok, why = set_skill_enabled(q, slug, False)
            if ok:
                off.append(slug)
            else:
                # 收不干净就**如实记下来**：这条任务的输入被上一条污染了，
                # 它的结果不能当成干净环境下的结论。
                problems.append("停用技能 %s 失败：%s" % (slug, why))
    else:
        problems.append("读技能列表 HTTP %s：%s" % (st, err_detail(payload)))

    # 服务器：这条不需要的，一律停用（ISSUE-026 修好之后这才真正管用）。
    wanted_servers = {}
    st, payload = q.get("/api/extensions/mcp")
    if st == 200:
        for row in payload.get("servers") or []:
            if row.get("name"):
                wanted_servers[row["name"]] = row["name"] in want_servers
    else:
        problems.append("读 MCP 列表 HTTP %s：%s" % (st, err_detail(payload)))
    ok, why = set_servers_enabled(q, wanted_servers)
    if not ok:
        problems.append("调整 MCP 服务器失败：%s" % why)

    return {"skills_off": off, "servers": wanted_servers, "problem": "；".join(problems)}


def restore(q: Quillian, snap: dict, dry: bool) -> list:
    """整轮跑完，把启用状态恢复成跑之前的样子。返回恢复失败的原因列表。"""
    if dry:
        return []
    problems = []
    for slug, was in (snap.get("skills") or {}).items():
        body, why = load_skill(slug)
        if body is None:
            # 磁盘上没有正文就写不回去 —— **如实说**，不假装恢复了。
            problems.append("恢复技能 %s 失败：%s" % (slug, why))
            continue
        st, payload = q.post(
            "/api/extensions/skills",
            {"slug": slug, "content": body, "enabled": was},
        )
        if st not in (200, 201):
            problems.append("恢复技能 %s 失败 HTTP %s：%s" % (slug, st, err_detail(payload)))
    ok, why = set_servers_enabled(q, snap.get("servers") or {})
    if not ok:
        problems.append("恢复 MCP 服务器失败：%s" % why)
    return problems


# ---------------------------------------------------------------- 判定

def judge(task: dict, chat: dict, need_servers: dict, servers: dict,
          skills: list, claims) -> dict:
    """给一条任务出结论。**三个维度分开，谁也不替谁说话。**"""
    res = {
        "link": {"ok": False, "why": ""},
        "tools": {"judgeable": False, "called": [], "missing_servers": [], "why": ""},
        "claims": {"judgeable": False, "hit": 0, "total": 0, "why": ""},
        "suspicions": [],
        # 客户端自己等不下去了 —— 与「链路不通」是两回事，见 ISSUE-028。
        "timed_out": False,
    }

    # 1) 链路
    if chat.get("__http") == 0:
        code = ((chat.get("error") or {}).get("code") or "")
        res["timed_out"] = code == "transport" and "timed out" in err_detail(chat)
        res["link"]["why"] = (
            "客户端超时，没拿到结果（这不是 quill 的错，详见 ISSUE-028）：%s"
            % err_detail(chat)
            if res["timed_out"]
            else "连不上服务：%s" % err_detail(chat)
        )
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
    - **客户端自己等不下去了** → `TIMEOUT`（见下，不是 FAIL）
    """
    # 超时**不是 FAIL**，也不是任何一种「验过了」。见 ISSUE-028。
    if res.get("timed_out"):
        return "TIMEOUT"
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

REPO_TOKEN = "{REPO}"


def render_prompt(task: dict) -> str:
    """把 tasks.json 里的 {REPO} 占位符换成本机仓库根目录。

    tasks.json 是入库文件，里面写死绝对路径会让别的机器 clone 下来就指向一个
    不存在的目录，所以路径一律用占位符。**换不掉就当场报错** —— 带着
    `{REPO}` 字面量发出去的提示，等于告诉模型去读一个不存在的目录，
    那比不发路径更糟。
    """
    root = str(pathlib.Path(__file__).resolve().parents[1])
    text = task.get("prompt") or ""
    if REPO_TOKEN in text:
        text = text.replace(REPO_TOKEN, root)
    if REPO_TOKEN in text:
        raise SystemExit(
            "任务 %s 的提示里还有没换掉的 {REPO} 占位符，拒绝发送。"
            % task.get("id")
        )
    return text


def run_task(q: Quillian, task: dict, dry: bool) -> dict:
    tid = task["id"]
    need = sorted({s for s in (tool_server(tool_name(x)) for x in (task.get("required_tools") or [])) if s})

    # **先隔离，再灌技能**：顺序反了的话，刚停用的又会被自己挂回去。
    iso = isolate_for(q, task, dry)

    servers = configured_servers(q, dry)

    # 技能：先全灌好，再看它们挂没挂上。挂不上是 quill 的问题，记下来。
    skills = []
    for s in task.get("skills") or []:
        skills.append(ensure_skill(q, s["slug"], s.get("description", ""), dry))
    unseeable = [s["slug"] for s in skills if s.get("installed") and not s.get("model_can_see")]

    if dry:
        chat = {"__http": 0, "error": {"code": "dry_run", "detail": "dry-run 没发这条消息"}}
    else:
        content = render_prompt(task)
        st, session = q.post("/api/sessions", {"title": "runner: " + tid})
        if st not in (200, 201) or not session.get("id"):
            chat = {"__http": st, "error": session.get("error") or {"detail": "建会话没拿到 id"}}
        else:
            st2, chat = q.post(
                "/api/sessions/%s/messages" % session["id"],
                {"content": content},
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
        # 隔离记录随结果一起存：日后看这条结论时，能知道当时**排掉了什么**，
        # 以及有没有收不干净的地方（`problem` 非空就是收不干净）。
        "isolation": iso,
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
    ap.add_argument(
        "--timeout",
        type=int,
        default=int(os.environ.get("QUILL_RUNNER_TIMEOUT", "600")),
        help="单条请求的客户端超时秒数，默认 600。"
        "**注意这只是下限**：quill 一轮对话最多 1 + MAX_TOOL_ROUNDS 次串行模型调用，"
        "每次都有自己的超时（默认 300s），最坏能到 1500s。"
        "等不下去会记成 TIMEOUT，**不算跑过**（见 ISSUE-028）。",
    )
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

    q = Quillian(args.base, args.token, timeout=args.timeout)
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

    # 跑之前拍一张启用状态的快照。**整轮跑完原样还回去** ——
    # 这是专用测试库，但它同时也是一个用户的真实配置，不能被这轮跑坏。
    snap = snapshot_enabled(q, args.dry_run)
    if not args.dry_run:
        print("已记下启用状态：技能 %d 个（启用 %d）、MCP 服务器 %d 台（启用 %d）。"
              % (len(snap["skills"]), sum(1 for v in snap["skills"].values() if v),
                 len(snap["servers"]), sum(1 for v in snap["servers"].values() if v)))

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
        iso = r.get("isolation") or {}
        if iso.get("skills_off"):
            bits.append("已隔离技能 %d 个" % len(iso["skills_off"]))
        if iso.get("problem"):
            bits.append("隔离不干净：" + iso["problem"][:70])
        if r.get("error"):
            bits.append("err=" + r["error"][:80])
        print("  " + " | ".join(bits))

    # 恢复启用状态。**恢复失败必须喊出来**，不能默默咽下去 ——
    # 留下一份被这轮跑改坏的配置，比报错难查得多。
    restore_problems = restore(q, snap, args.dry_run)
    if restore_problems:
        print("\n恢复启用状态时出错（%d 条）：" % len(restore_problems), file=sys.stderr)
        for p in restore_problems:
            print("  - " + p, file=sys.stderr)
        print("下一步：照上面逐条处理，或直接用 POST /api/extensions/skills "
              "把 enabled 改回去。", file=sys.stderr)
    elif not args.dry_run:
        print("\n启用状态已恢复成跑之前的样子。")

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
        "  TIMEOUT      **客户端自己等不下去了**（不是 quill 的错），结果没拿到。\n"
        "\n真的跑完的条数 = PASS + PARTIAL + FAIL = %d"
        "（UNJUDGEABLE 与 TIMEOUT 都不算跑过）\n"
        "依赖的服务器没配时，工具那一维度记 unjudgeable 并写清缺哪几台 ——\n"
        "**那不是 quill 的问题，也不算跑过。**\n"
        "客户端超时记 TIMEOUT：quill 一轮最多 1+%d 次串行模型调用，"
        "最坏比这个长得多，等不下去不等于它坏了。" % (done, 4)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
