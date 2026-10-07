#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""从 .octop-ref/octop 的 FastAPI 路由源码生成「迁移功能清单」原始 CSV。

产出的 CSV 是**机器生成的原始事实**（哪个文件有哪些端点），
分类与 quill 完成度由同目录的 FILE_MAP 决定，人可以改 FILE_MAP 再重跑。
不联网、只读上游源码。
"""
import csv
import os
import re
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(ROOT, ".."))
ROUTERS = os.path.join(REPO, ".octop-ref", "octop", "src", "octop", "api", "routers")
OUT = os.path.join(REPO, ".scratch", "octop-endpoints.csv")

# 文件 → (分类, 是否接 goose 核心, quill 现状)
# 分类：前端效果 / goose核心 / 平台 / 外部依赖 / 数据
# goose 核心 = True 表示「必须打通 goose 内核」
# quill 现状：已实现 / 部分 / 桩(501) / 未做
FILE_MAP = {
    "acp.py": ("goose核心", True, "未做", "外部 CLI agent（opencode/claude_code 等）runner 配置"),
    "admin.py": ("平台", False, "未做", "管理台总览/审计/指标"),
    "agent_files.py": ("goose核心", True, "未做", "每 agent 心跳配置 + 每日记忆文件"),
    "agent_tools.py": ("goose核心", True, "部分", "每 agent 工具开关（denylist + 插件工具）"),
    "agents.py": ("goose核心", True, "部分", "agent 生命周期（quill 叫 expert）"),
    "auth.py": ("平台", False, "已实现", "登录/登出/me/改密（quill 无 register，有意）"),
    "auth_ldap.py": ("外部依赖", False, "未做", "LDAP 目录登录"),
    "auth_oauth.py": ("外部依赖", False, "未做", "OAuth SSO（飞书/钉钉/企微）"),
    "auth_oidc.py": ("外部依赖", False, "未做", "OIDC SSO"),
    "backup.py": ("平台", False, "部分", "备份/恢复（quill 有 export/verify/restore，admin-only）"),
    "bridge.py": ("goose核心", True, "未做", "远程 octop 实例协作 + 入站 WS"),
    "channels.py": ("外部依赖", False, "部分", "IM 渠道（quill 只做微信）"),
    "connectors.py": ("goose核心", True, "部分", "Connector/MCP 实例（quill 有 MCP，无目录）"),
    "cron.py": ("goose核心", True, "未做", "定时任务"),
    "envs.py": ("平台", False, "未做", "全局环境变量"),
    "experts.py": ("平台", False, "部分", "专家模板 + 市场（quill 有市场）"),
    "filesystem.py": ("数据", False, "未做", "主机文件系统浏览"),
    "health.py": ("平台", False, "已实现", "健康检查（quill 是 /healthz）"),
    "i18n.py": ("平台", False, "未做", "工具/技能显示名本地化"),
    "internal_mcp.py": ("goose核心", True, "未做", "托管 connector 的内部 MCP 网关"),
    "invites.py": ("平台", False, "部分", "邀请码（quill 存储层有，HTTP 未接）"),
    "knowledge_bases.py": ("数据", False, "未做", "知识库（quill 用 quill-wiki 替代）"),
    "mbti.py": ("平台", False, "已实现", "MBTI（quill 已迁移 0010_mbti）"),
    "media_generation.py": ("外部依赖", False, "未做", "图像/视频生成设置"),
    "memory.py": ("goose核心", True, "未做", "每 agent 记忆看板（原子/实体/片段/候选）"),
    "memory_portable.py": ("goose核心", True, "未做", "跨主机记忆迁移"),
    "observability.py": ("外部依赖", False, "未做", "Langfuse 可观测性"),
    "ollama_download_store.py": ("外部依赖", False, "未做", "Ollama 下载任务存储（无路由）"),
    "ollama_models.py": ("外部依赖", False, "未做", "Ollama 模型管理"),
    "onnx_models.py": ("外部依赖", False, "未做", "本地 ONNX embedding 缓存"),
    "plugins.py": ("平台", False, "桩(501)", "插件安装/市场"),
    "preferences.py": ("平台", False, "未做", "用户偏好（locale 等）"),
    "proactive_care.py": ("goose核心", True, "未做", "主动关怀推送配置"),
    "providers.py": ("goose核心", True, "部分", "Provider（quill 有多供应商 admin）"),
    "search.py": ("外部依赖", False, "未做", "搜索 provider 连通性"),
    "security.py": ("平台", False, "未做", "安全策略 / 命令守卫规则"),
    "settings.py": ("平台", False, "部分", "进程级设置（quill 有部分于 admin/config）"),
    "setup.py": ("平台", False, "部分", "安装向导（quill 有 setup/status + initial-admin）"),
    "skill_packages.py": ("goose核心", True, "部分", "实例级全局 skill 包"),
    "skills.py": ("goose核心", True, "部分", "每 agent SKILL.md 库 + SkillHub"),
    "slash.py": ("goose核心", True, "未做", "斜杠命令发现"),
    "storage_backends.py": ("数据", False, "未做", "存储后端"),
    "subagents.py": ("goose核心", True, "未做", "子 agent 定义与内置目录"),
    "teams.py": ("goose核心", True, "部分", "专家团队（quill 有 CRUD，派工未执行）"),
    "terminal.py": ("goose核心", True, "未做", "每 agent 交互式 PTY 终端（WS）"),
    "tls.py": ("平台", False, "未做", "TLS / Let's Encrypt"),
    "update.py": ("平台", False, "桩(501)", "自更新（quill 只有升级前守卫）"),
    "update_store.py": ("平台", False, "未做", "升级任务跟踪（无路由）"),
    "uploads.py": ("goose核心", True, "未做", "聊天附件上传"),
    "usage.py": ("平台", False, "已实现", "Token 用量账本（quill 有 /api/usage）"),
    "user_roles.py": ("平台", False, "未做", "角色模板 CRUD"),
    "users.py": ("平台", False, "部分", "用户 CRUD（quill 只 list/patch）"),
    "voice.py": ("外部依赖", False, "未做", "语音 STT/TTS"),
    "workspace.py": ("数据", False, "未做", "运行中 agent 工作区读写"),
}

SUB_MAP = {
    "browser/": ("外部依赖", False, "未做", "浏览器自动化（拉起 Chromium，录制/回放）"),
    "chat/": ("goose核心", True, "部分", "看板聊天（WS turn / 线程 / 轨迹 / HITL）"),
    "desktop/": ("外部依赖", False, "未做", "远程桌面"),
    "mobile/": ("外部依赖", False, "未做", "远程 Android"),
    "__init__.py": ("平台", False, "—", "聚合，无端点"),
}

DEC = re.compile(r'@router\.(get|post|put|patch|delete)\(')
STRLIT = re.compile(r'"([^"]*)"')


def endpoints_in(path):
    """返回 [(method, route_path, summary)]。容忍多行装饰器。"""
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        text = f.read()
    out = []
    for m in DEC.finditer(text):
        method = m.group(1).upper()
        # 从装饰器起点往后 300 字内找第一个字符串字面量作为 path
        tail = text[m.end():m.end() + 400]
        # 截到匹配的右括号层（简单起见：截到下一个 '@router.' 或 '\ndef '）
        cut = min([x for x in [tail.find("@router."), tail.find("\ndef "), tail.find("\nasync def "), 400] if x >= 0] or [400])
        tail = tail[:cut]
        s = STRLIT.search(tail)
        route = s.group(1) if s else ""
        sm = re.search(r'summary\s*=\s*"([^"]*)"', tail)
        summary = sm.group(1) if sm else ""
        out.append((method, route, summary))
    return out


def classify(rel):
    for k, v in SUB_MAP.items():
        if rel.startswith(k):
            return v
    return FILE_MAP.get(rel, ("平台", False, "未做", ""))


def main():
    rows = []
    files = []
    for dirpath, _dirs, fnames in os.walk(ROUTERS):
        for fn in sorted(fnames):
            if fn.endswith(".py"):
                files.append(os.path.join(dirpath, fn))
    n = 0
    for p in sorted(files):
        rel = os.path.relpath(p, ROUTERS).replace("\\", "/")
        cat, core, status, note = classify(rel)
        eps = endpoints_in(p)
        if not eps:
            rows.append([n, rel, "(无端点)", "", "", cat, "是" if core else "否", status, note])
            continue
        for method, route, summary in eps:
            n += 1
            rows.append([n, rel, method, route, summary, cat, "是" if core else "否", status, note])

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8-sig", newline="") as f:
        w = csv.writer(f)
        w.writerow(["序号", "octop文件", "方法", "路径", "用途", "分类", "接goose核心", "quill现状", "备注"])
        w.writerows(rows)
    print(f"写出 {len(rows)} 行 → {OUT}")
    # 统计
    from collections import Counter
    c_cat = Counter(r[5] for r in rows)
    c_core = Counter(r[6] for r in rows)
    c_st = Counter(r[7] for r in rows)
    print("分类:", dict(c_cat))
    print("接核心:", dict(c_core))
    print("quill现状:", dict(c_st))


if __name__ == "__main__":
    main()
