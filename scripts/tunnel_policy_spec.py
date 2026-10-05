#!/usr/bin/env python3
# =============================================================================
# tunnel_policy_spec.py —— HTTP 隧道三项防护的**可执行策略规范**（单一真相源）
# =============================================================================
# 归属（07_实例间委派设计.md §责任边界）：
#   · 防护策略实现 → 共享 crate `quill-bridge`（**两端都要用**，物理位置由主理人指定）
#   · 本文件 = **策略的可执行定义 + 部署侧配置**，供三方对齐：
#       ① Rust 实现（quill-bridge）必须与本规范逐条一致
#       ② 运维闸门 check-tunnel-protection-boundary.sh 校验之
#       ③ 自检 test-tunnel-policy.sh 用它跑「恶意必拒 / 合法必放」两侧
#
# ⚠️ 为什么需要「可执行规范」而不是文档里的文字描述：
#   文档写「默认拒绝」无法验证；写成正则/网段后，**两侧用例可以机械比对**。
#   **状态必须由工具给出，不能靠人转述。**
#
# 依据：07_实例间委派设计.md §3.5（agent-app-engineer 实测 Octop 后的约束）
# 参照：Octop `infra/bridge/{tunnel_policy.py, peer_auth.py, http_tunnel.py}`
# =============================================================================
"""
三项防护：
  ① SSRF 防护      —— 校验**DNS 解析后**的每个 IP（防 DNS rebinding），非仅字面 IP
  ② 隧道路径白名单 —— **方法级**正则 + **默认拒绝**（未匹配即拒）
  ③ 敏感头剥离     —— hop-by-hop + authorization/cookie，auth 由隧道层强制注入

安全模型（07 §3.5 攻击面 3）：
  对端实例 B 用**连接所有者（B 的用户）**的凭据执行，**不是** A 的凭据。
  → 所以必须剥离对端发来的 authorization，改为注入连接所有者自己的。
"""
from __future__ import annotations

import ipaddress
import re
import socket
from typing import Iterable

# =============================================================================
# ① SSRF 防护
# =============================================================================

# 🔴 元数据地址：**永远禁止**，无任何授权可放开（07 §3.5 / Q-2 裁决）
BLOCKED_NETWORKS: tuple[str, ...] = (
    "169.254.0.0/16",       # 云元数据（AWS/GCP/Azure/Alibaba）
    "169.254.169.254/32",   # 显式单点，冗余声明防误删
    "fe80::/10",            # IPv6 链路本地
    "::ffff:169.254.0.0/112",  # IPv4-mapped IPv6 形式（解析器可能返回这种）
    "fd00::/8",             # IPv6 唯一本地地址
    "::1/128",              # IPv6 localhost
    "127.0.0.0/8",          # IPv4 localhost
    "0.0.0.0/8",            # 「本机」
)

# ⚠️ 私网网段：**默认封 + 知情授权**（Q-2 裁决）
#   目标场景是家用 NAS + 内网 VPS，B 很可能在家庭 LAN
#   → 完全封死 = 功能在目标场景不可用 = 等于没做
#   → 但必须由用户显式开启（allow_private_network），且**永不**放开元数据
PRIVATE_NETWORKS: tuple[str, ...] = (
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "fc00::/7",
)


class SsrfDecision:
    """SSRF 判定的结果。`allowed` 为 False 时 `reason` 必非空。"""

    __slots__ = ("allowed", "reason", "resolved")

    def __init__(self, allowed: bool, reason: str = "", resolved: tuple[str, ...] = ()) -> None:
        self.allowed = allowed
        self.reason = reason
        self.resolved = resolved

    def __repr__(self) -> str:  # pragma: no cover - 调试用
        return f"SsrfDecision(allowed={self.allowed}, reason={self.reason!r}, resolved={self.resolved})"


def _parse_ip(value: str) -> ipaddress._BaseAddress | None:
    try:
        return ipaddress.ip_address(value)
    except ValueError:
        return None


def classify_ip(ip_text: str, *, allow_private: bool = False) -> tuple[bool, str]:
    """对**单个已解析的 IP** 判定。返回 (是否放行, 原因)。

    ⚠️ 调用方必须先做 DNS 解析再逐个调用本函数 —— 只校验字面 IP 会被
       `evil.com → 169.254.169.254`（DNS rebinding）绕过。Octop 做对了这一步。
    """
    ip = _parse_ip(ip_text)
    if ip is None:
        return False, f"无法解析为 IP：{ip_text}"

    # 关键：IPv4-mapped IPv6 必须**折叠成 IPv4** 再判，否则 ::ffff:169.254.169.254
    # 会绕过 169.254.0.0/16 的检查（这是最常见的绕过手法）
    if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped is not None:
        ip = ip.ipv4_mapped

    for net in BLOCKED_NETWORKS:
        if ip in ipaddress.ip_network(net):
            # 元数据与 loopback 永远禁止，不接受 allow_private 影响
            return False, f"命中永远禁止网段 {net}（元数据/loopback 不授权）"

    if not allow_private:
        for net in PRIVATE_NETWORKS:
            if ip in ipaddress.ip_network(net):
                return False, f"命中默认封禁私网 {net}（需知情授权 allow_private_network）"

    return True, ""


# =============================================================================
# ★ 契约硬要求（backend v2.7 §11.1，2026-10-04）
# =============================================================================
# 原文：「判据：判定的输入必须与连接的输入是【同一个对象】。
#        "判了 A、连了 B"不是安全漏洞，是【根本没判】。」
#
# 本模块的调用契约（**调用方必须遵守，否则本模块的安全保证形同虚设**）：
#
#   ① SSRF 判定【只解析一次】，返回 `SsrfDecision.resolved`（已判定的全部 IP）
#   ② 连接器 dial 的 IP **必须 ∈ resolved**          ← 由调用方断言，本模块无法保证
#   ③ 连接器【禁止】再次解析 hostname
#      （⚠️ `reqwest` 默认就是「getaddrinfo + 自选第一个 IP」→ 违反本条）
#   ④ 若连接器**无法复用** `resolved`（reqwest 默认如此），
#      则【禁止】启用 `allow_private_network` —— 宁可关闭私网支持，
#      也不要「判了但连的不是那个」：
#      后者会给用户**虚假的安全感**（allowed=true 但实际连到别处）。
#
# ⚠️ 违反了会怎样：本模块返回 `allowed=true`（因为 DNS 解析结果确实干净），
#    但连接器连了**另一个**未被判定的 IP → 攻击者用 DNS rebinding
#    在「判定之后、连接之前」把 A 记录换成 169.254.169.254 → 绕过全部防护。
#    → 这是**先检查后使用（TOCTOU）** 类漏洞，必须由调用方消除时间窗。
#
# 参考实现要求：自定义 `dns_resolver`，或 dial 前校验 `resolved` 集合。


def check_peer_url(
    url: str,
    *,
    allow_private: bool = False,
    resolver: Iterable[str] | None = None,
) -> SsrfDecision:
    """校验对端 base_url 的 SSRF 安全性。

    `resolver` 仅供测试注入（避免自检依赖真实 DNS）。**生产不得传入。**
    真实实现必须走 `socket.getaddrinfo()` 取**全部** A/AAAA 记录并逐个判定 ——
    只判第一个解析结果会被多 A 记录绕过。
    """
    m = re.match(r"^https?://([^/:?#]+)(?::(\d+))?", url or "", re.IGNORECASE)
    if not m:
        return SsrfDecision(False, "URL 格式非法或 scheme 非 http(s)")
    host = m.group(1)

    # 字面 IP 直接判
    if _parse_ip(host) is not None:
        ok, reason = classify_ip(host, allow_private=allow_private)
        return SsrfDecision(ok, reason, (host,))

    if resolver is not None:
        resolved = tuple(resolver)
    else:
        # ⚠️ 生产路径：真实 DNS 解析
        try:
            infos = socket.getaddrinfo(host, None)
        except socket.gaierror as e:
            return SsrfDecision(False, f"DNS 解析失败：{e}")
        # 去重且保留全部记录 —— 多 A 记录绕过是真实存在的
        resolved = tuple(dict.fromkeys(i[4][0] for i in infos))

    if not resolved:
        return SsrfDecision(False, "DNS 未返回任何地址")

    for ip_text in resolved:
        ok, reason = classify_ip(ip_text, allow_private=allow_private)
        if not ok:
            # 🔴 关键：**任一**解析结果违规即整体拒绝
            return SsrfDecision(False, f"DNS 解析结果 {ip_text} 不可用：{reason}", resolved)

    return SsrfDecision(True, "", resolved)


# =============================================================================
# ② 隧道路径白名单（方法级 + 默认拒绝）
# =============================================================================

# ⚠️ 只列路径不列方法是常见漏洞（07 §3.5 攻击面 2 明确指出）
#   例：把 /api/agents/x 列入白名单但不限方法 → POST 即可写操作
#
# ★ 入站委派**比出站控制更窄**（07 §3.5 + backend v2.5 采纳我的论证）：
#   Octop 的 _AGENT_RESOURCE 允许 6 种方法，那是「操作我自己的对端」的需求；
#   而「接收别人派来的任务」是相反方向 —— 我无法确认 A 给 B 开的授权范围，
#   所以入站必须更严：**委派路径只允许 POST**。
#
# ★ 白名单已定死 5 条（backend-engineer v2.5 裁定，每条都能说清"为什么必须有"）：
#   backend 从 6 条收到 5 条，删掉了 3 条以避免超配：
#     × GET /api/sessions/{id}      —— V1 不做「列出 B 的会话」（属编排，v1.1）
#     × GET /api/experts            —— A 侧用户不需要看 B 的专家列表
#     × POST /api/wiki/search       —— 检索是 B 的 Agent 内部行为，不该由 A 穿透调用
TUNNEL_PATH_RULES: tuple[tuple[str, re.Pattern[str]], ...] = (
    # ① 在 B 上建会话（委派起点）
    ("POST ^/api/sessions$", re.compile(r"^/api/sessions$")),
    # ② 发消息 = 任务本体
    ("POST ^/api/sessions/[^/]+/messages$", re.compile(r"^/api/sessions/[^/]+/messages$")),
    # ③ 断线补齐（唯一必要的 GET）
    ("GET ^/api/sessions/[^/]+/messages$", re.compile(r"^/api/sessions/[^/]+/messages$")),
    # ④ 对端身份/状态（「我连的是谁」）
    ("GET ^/api/bridge/agent$", re.compile(r"^/api/bridge/agent$")),
    # ⑤ 查在途进度
    ("GET ^/api/bridge/dispatch/[^/]+/status$", re.compile(r"^/api/bridge/dispatch/[^/]+/status$")),
)

# 规则 → 允许的方法（由规则名里的方法前缀决定，**方法与路径同源声明**，
#   杜绝「列了路径忘限方法」这个漏洞）
RULE_METHODS: dict[str, frozenset[str]] = {
    "^/api/sessions$": frozenset({"POST"}),
    "^/api/sessions/[^/]+/messages$": frozenset({"GET", "POST"}),
    "^/api/bridge/agent$": frozenset({"GET"}),
    "^/api/bridge/dispatch/[^/]+/status$": frozenset({"GET"}),
}

# ⚠️⚠️ 2026-10-04 已删除 DISPATCH_METHODS / READONLY_METHODS（backend 指出）
#
#   这两个常量**曾经定义但零引用** —— 判定的唯一真相是 RULE_METHODS。
#   backend 的原话（我完全同意）：
#     「留着比删掉更危险 —— 它看起来是权威定义，
#       下一个人会以为『改它能放开方法』—— 改了零作用且不报错。」
#
#   → 这正是本项目「看起来在工作、实际什么都不做」的同类：
#     一个**看起来可配置、实际无作用**的旋钮，比没有旋钮更坏。
#
#   🔴 防漂移约定（backend §5.3）：方法的**唯一**真相是 RULE_METHODS。
#      下面两条断言保证「若有人重新引入这两个死常量，自检会判红」。
DEAD_CONSTANTS_MUST_NOT_EXIST: tuple[str, ...] = ("DISPATCH_METHODS", "READONLY_METHODS")

# 🔴 永远 local-only 的面前缀（Octop 注释原文：
#   "Management / auth / bridge control planes stay local-only"）
# ⚠️ /api/vault 是 backend v2.5 追加的**最高优先级禁止项**：
#   若不禁，A 派任务时可读 B 的凭据 → P0-9 的核心被直接绕过。
#   凭据只在 data/{uid}/secrets.enc，绝不可经隧道读取。
LOCAL_ONLY_PREFIXES: tuple[str, ...] = (
    "/api/vault",         # 🔴 凭据库：泄露即等于泄露 B 的登录口令
    "/api/auth",          # 登录/鉴权
    "/api/admin",         # 管理面
    "/api/bridge/control",  # 隧道自身控制面
    "/metrics",
    "/healthz",
    "/readyz",
)

# 部署侧配置：请求体上限（07 责任边界表「部署配置」归 devops）
# ⚠️ 1 MiB，而 backend 的 context 校验是 ≤256KB（08 §4.5）→ 4 倍余量，已确认不冲突
MAX_TUNNEL_BODY_BYTES = 1 * 1024 * 1024  # 1 MiB


def is_tunnel_path_allowed(method: str, path: str) -> bool:
    """方法级路径白名单。**默认拒绝** —— 未匹配即拒（抄 Octop 的 `return False`）。

    这是 B 侧（入站）axum handler 必须调用的函数。
    """
    if not method or not path:
        return False

    # 归一化：去掉 query/fragment（防 `?` 绕过）
    clean = path.split("?", 1)[0].split("#", 1)[0]
    # 归一化：拒绝非常规编码的路径（防 %2e%2e 之类绕过）
    if "%" in clean:
        return False
    # 路径必须以 / 开头且不含换行（防 header 注入 / 路径走私）
    if not clean.startswith("/") or "\n" in clean or "\r" in clean:
        return False
    # 禁止路径穿越
    if ".." in clean:
        return False

    upper = method.upper()

    # 🔴 local-only 面永远拒绝，无论方法
    for prefix in LOCAL_ONLY_PREFIXES:
        if clean == prefix or clean.startswith(prefix + "/"):
            return False

    # 规则匹配：方法与路径**同源声明**（规则名形如 "POST ^/path$"）
    # 🔴 关键：先按「方法+路径」精确匹配，再单独处理
    #    「路径匹配但方法不匹配」的情况 —— 明确拒绝，绝不回落到其他规则。
    path_matched_but_method_rejected = False
    for rule_name, pattern in TUNNEL_PATH_RULES:
        if not pattern.match(clean):
            continue
        want_method, _, path_pat = rule_name.partition(" ")
        # 规则名里的路径可能与正则不同源，用它查方法表
        allowed = RULE_METHODS.get(path_pat, frozenset())
        if upper in allowed:
            return True
        path_matched_but_method_rejected = True
    # 路径在白名单里但方法不对 → 拒（这正是「只列路径不列方法」漏洞的拦截点）
    if path_matched_but_method_rejected:
        return False

    # ★ 默认拒绝：走到这里说明没有任何规则匹配
    return False


# =============================================================================
# ③ 敏感头剥离
# =============================================================================

# ⚠️ hop-by-hop 头**不能**被代理转发（RFC 7230 §6.1）
# ⚠️ authorization/cookie 必须剥离：隧道层用**连接所有者**的凭据重新注入
#   （07 §3.5 攻击面 3：对端实例 B 用 B 的用户身份执行，不是 A 的）
DROP_REQUEST_HEADERS: frozenset[str] = frozenset({
    # hop-by-hop
    "connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
    "te", "trailer", "transfer-encoding", "upgrade",
    # 由隧道层重算
    "host", "content-length",
    # ★ 敏感：必须剥离并由隧道层强制注入连接所有者的凭据
    "authorization", "cookie", "proxy-authorization",
    # 避免上游看到对端内部拓扑
    "x-forwarded-for", "x-forwarded-host", "x-real-ip",
    "x-bridge-peer", "x-bridge-node",
})

# 隧道层必须强制注入的头（07 §3.5：auth is injected as the connection owner）
INJECT_REQUEST_HEADERS: tuple[str, ...] = ("authorization",)


def strip_request_headers(headers: dict[str, str] | list[tuple[str, str]]) -> dict[str, str]:
    """剥离 hop-by-hop / 敏感头。大小写不敏感。返回**新**字典（不改入参）。"""
    items = headers.items() if isinstance(headers, dict) else headers
    out: dict[str, str] = {}
    for k, v in items:
        if k.lower() in DROP_REQUEST_HEADERS:
            continue
        out[k] = v
    return out


def inject_connection_owner_auth(headers: dict[str, str], owner_token: str) -> dict[str, str]:
    """注入连接所有者的凭据（覆盖任何残留）。**这是唯一允许写 authorization 的地方。**"""
    out = {k: v for k, v in headers.items() if k.lower() != "authorization"}
    out["authorization"] = f"Bearer {owner_token}"
    return out


if __name__ == "__main__":  # pragma: no cover - CLI 便于人工核对
    import sys

    # ⚠️ 下面两个 CLI 子命令仅供**人工核对**。
    #    Windows/Git Bash(MSYS2) 会把 `/api/...` 这类 argv 转换成 Windows 路径
    #    （`/api/x` → `C:/Program Files/.../api/x`），因此**自检绝不可走这里**——
    #    自检直接 import 本模块调函数（见 test_tunnel_policy.py）。
    if len(sys.argv) >= 3 and sys.argv[1] == "path":
        ok = is_tunnel_path_allowed(sys.argv[2], sys.argv[3])
        print("ALLOW" if ok else "DENY")
        sys.exit(0 if ok else 1)
    if len(sys.argv) >= 2 and sys.argv[1] == "ssrf":
        allow_private = "--allow-private" in sys.argv
        d = check_peer_url(sys.argv[2], allow_private=allow_private)
        print(f"{'ALLOW' if d.allowed else 'DENY'} {d.reason} resolved={d.resolved}")
        sys.exit(0 if d.allowed else 1)
    print(__doc__)
    print("用法：")
    print("  tunnel_policy_spec.py path <METHOD> <PATH>")
    print("  tunnel_policy_spec.py ssrf <URL> [--allow-private]")
    sys.exit(2)
