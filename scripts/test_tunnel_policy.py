#!/usr/bin/env python3
# =============================================================================
# test_tunnel_policy.py —— tunnel_policy_spec.py 的自检（恶意必拒 / 合法必放）
# =============================================================================
# ★ 任务要求：「每项须配自检 + 绕过用例（**合法用例必须放行**）」
#
# 🔴🔴 为什么这个自检是 Python 而不是 shell（本项目实测踩坑）：
#   初版用 bash harness，经 argv 传路径给 python：
#       python3 spec.py path POST /api/bridge/dispatch
#   → **MSYS2/Git Bash 会把以 `/` 开头的 argv 转换成 Windows 路径**：
#       /api/bridge/dispatch  →  C:/Users/.../api/bridge/dispatch
#   → 于是**所有合法用例都被判 DENY**，19 个用例假红。
#   而闸门（策略本身）**完全正确**。
#   **教训：先怀疑 harness，再怀疑闸门**；且 Windows 上测路径类逻辑必须避开 argv。
#
# 双向纪律（不可只测一侧）：
#   · 只测「恶意被拒」→ 可能「什么都拒」而恒绿（假闸门）
#   · 只测「合法被放」→ 可能「什么都放」而恒绿
#   → **两侧都测，且合法用例失败同样算失败**。
#
# 退出码：0=全部通过  1=有用例失败
# =============================================================================
from __future__ import annotations

import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
# ★ 允许用环境变量指向被测对象 —— 故障注入器（falsify_tunnel_policy.py）靠它
#   把「注入故障的 spec」喂给本自检，从而验证自检**有鉴别力**。
#   正常用法不设此变量。
SPEC_PATH = os.environ.get("TUNNEL_POLICY_SPEC") or os.path.join(HERE, "tunnel_policy_spec.py")

_pass = 0
_failed = 0
_section = ""


def section(title: str) -> None:
    global _section
    _section = title
    print(f"\n{'=' * 4} {title} {'=' * 4}")


def ok(msg: str) -> None:
    global _pass
    _pass += 1
    print(f"  o {msg}")


def bad(msg: str) -> None:
    global _failed
    _failed += 1
    print(f"  x {msg}")


def expect(label: str, expected: str, got: str) -> None:
    if got == expected:
        ok(f"{label} → {got}")
    else:
        bad(f"{label} (期望 {expected}，实得 {got})")


def load_spec():
    spec = importlib.util.spec_from_file_location("tunnel_policy_spec", SPEC_PATH)
    if spec is None or spec.loader is None:
        print(f"  x 无法加载规范文件：{SPEC_PATH}", file=sys.stderr)
        sys.exit(2)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main() -> int:
    m = load_spec()

    # ═══════════════════════════════════════════════════════════════════════
    section("① SSRF 防护（恶意必拒 / 合法必放）")

    # ── 恶意必拒 ──────────────────────────────────────────────────────────
    deny = [
        ("元数据地址（字面 IP）", "http://169.254.169.254/latest/meta-data/", None, False),
        ("🔴 IPv4-mapped 元数据（绕过手法）", "http://[::ffff:169.254.169.254]/", None, False),
        ("🔴 DNS rebinding → 元数据", "http://evil.example/", ["169.254.169.254"], False),
        ("🔴 多 A 记录其一为元数据", "http://evil.example/", ["93.184.216.34", "169.254.169.254"], False),
        ("loopback", "http://127.0.0.1:8080/", None, False),
        ("IPv6 loopback", "http://[::1]:8080/", None, False),
        ("IPv6 链路本地", "http://[fe80::1]:8080/", None, False),
        ("私网 192.168（未授权）", "http://192.168.1.10:3000/", None, False),
        ("私网 10.x（未授权）", "http://10.0.0.5:3000/", None, False),
        ("私网 172.16（未授权）", "http://172.16.0.1:3000/", None, False),
        ("非 http(s) scheme", "file:///etc/passwd", None, False),
        ("URL 格式非法", "not-a-url", None, False),
        ("🔴 授权私网也不放开元数据", "http://169.254.169.254/", None, True),
    ]
    for label, url, resolver, allow_private in deny:
        d = m.check_peer_url(url, allow_private=allow_private, resolver=resolver)
        expect(label, "DENY", "ALLOW" if d.allowed else "DENY")

    # ── 合法必放（防止「什么都拒」的假闸门）──────────────────────────────
    allow = [
        ("公网 IP", "https://203.0.113.10:3000/", None, False),
        ("公网域名（解析到公网）", "https://peer.example.com/", ["93.184.216.34"], False),
        ("公网多 A 记录（全部合法）", "https://peer.example.com/", ["93.184.216.34", "93.184.216.35"], False),
        ("私网 + 知情授权", "http://192.168.1.10:3000/", None, True),
        ("内网 VPS + 知情授权", "http://10.0.0.5:3000/", None, True),
    ]
    for label, url, resolver, allow_private in allow:
        d = m.check_peer_url(url, allow_private=allow_private, resolver=resolver)
        expect(label, "ALLOW", "ALLOW" if d.allowed else "DENY")

    # DNS 解析失败必须 DENY 而非崩溃
    d = m.check_peer_url("http://nonexistent.invalid/", resolver=[])
    expect("DNS 无结果 → 拒（不得放行）", "DENY", "ALLOW" if d.allowed else "DENY")

    # ── G4 的鉴别力用例（防止「IPv4-mapped 折叠」被删掉而无人发现）──────────
    # 🔴 关键：不折叠 ::ffff:169.254.169.254 时，它**仍会**命中
    #    BLOCKED 里的 ::ffff:169.254.0.0/112 —— 即那条网段是**纵深防御**。
    #    所以「折叠」与「显式 IPv4-mapped 网段」是两道独立防线，
    #    删掉任一道都**不应该**放行元数据。两条都断言，才能测出「删折叠」这个动作。
    mapped_variants = [
        "::ffff:169.254.169.254",
        "::FFFF:169.254.169.254",     # 大写
        "::ffff:a9fe:a9fe",           # 十六进制等价的元数据地址
    ]
    for ip in mapped_variants:
        d = m.classify_ip(ip)[0] if hasattr(m, "classify_ip") else None
        got = m.check_peer_url(f"http://[{ip}]/", resolver=[ip])
        expect(f"IPv4-mapped 元数据被拒 {ip}", "DENY", "ALLOW" if got.allowed else "DENY")

    # 纵深防御断言：显式的 IPv4-mapped 网段必须存在于 BLOCKED_NETWORKS
    has_mapped_net = any("ffff:169.254" in n for n in m.BLOCKED_NETWORKS)
    expect("BLOCKED 含显式 IPv4-mapped 网段（纵深防御）", "YES", "YES" if has_mapped_net else "NO")

    # ═══════════════════════════════════════════════════════════════════════
    section("② 隧道路径白名单（方法级 + 默认拒绝）")

    # ── 合法必放（backend v2.5 定死的 5 条白名单，每条都验）─────────────────
    # ⚠️ 2026-10-04 backend v2.5 把白名单从 6 条改为 **5 条**（删 3 条避免超配）：
    #   × GET /api/sessions/{id}     —— V1 不做「列出 B 的会话」（属编排，v1.1）
    #   × GET /api/experts           —— A 侧用户不需要看 B 的专家列表
    #   × POST /api/wiki/search      —— 检索是 B 的 Agent 内部行为，不该由 A 穿透调用
    # 且方法与路径**同源声明**（规则名形如 "POST ^/path$"），杜绝「列了路径忘限方法」。
    # → 本用例集随该裁定同步；判据方向不变：**合法必放 / 恶意必拒**。
    allow_paths = [
        ("① 建会话 POST", "POST", "/api/sessions"),
        ("② 发消息 POST（任务本体）", "POST", "/api/sessions/abc/messages"),
        ("③ 断线补齐 GET", "GET", "/api/sessions/abc/messages"),
        ("④ 对端身份 GET", "GET", "/api/bridge/agent"),
        ("⑤ 查进度 GET", "GET", "/api/bridge/dispatch/abc/status"),
        ("小写方法 post", "post", "/api/sessions"),
        ("带 query（query 被归一化）", "POST", "/api/sessions?x=1"),
    ]
    for label, method, path in allow_paths:
        expect(label, "ALLOW", "ALLOW" if m.is_tunnel_path_allowed(method, path) else "DENY")

    # ── 恶意必拒（★ 绕过用例）────────────────────────────────────────────
    deny_paths = [
        # ★ 核心漏洞形态：路径在白名单里但**方法不对**（07 §3.5 强调的常见漏洞）
        ("🔴 建会话用 GET（越权读）", "GET", "/api/sessions"),
        ("🔴 建会话用 DELETE", "DELETE", "/api/sessions"),
        ("🔴 发消息用 PUT", "PUT", "/api/sessions/abc/messages"),
        ("🔴 发消息用 DELETE", "DELETE", "/api/sessions/abc/messages"),
        ("🔴 对端身份用 POST（只读面不许写）", "POST", "/api/bridge/agent"),
        ("🔴 查进度用 POST", "POST", "/api/bridge/dispatch/abc/status"),
        # ★ backend v2.5 显式删除的三条 —— 必须仍被拒（防回退）
        ("🔴 已删除：列出会话 GET /api/sessions/{id}", "GET", "/api/sessions/abc"),
        ("🔴 已删除：专家列表 GET /api/experts", "GET", "/api/experts"),
        ("🔴 已删除：wiki 检索 POST", "POST", "/api/wiki/search"),
        # local-only 面
        ("🔴 local-only 鉴权面", "POST", "/api/auth/login"),
        ("🔴 local-only 管理面", "GET", "/api/admin/users"),
        ("🔴 local-only 隧道控制面", "POST", "/api/bridge/control/reload"),
        ("🔴 local-only metrics", "GET", "/metrics"),
        ("🔴 local-only healthz", "GET", "/healthz"),
        ("🔴 local-only readyz", "GET", "/readyz"),
        ("★ G6 鉴别力：local-only 优先于任何形状", "POST", "/api/auth/dispatch/abc/status"),
        ("★ G6 鉴别力：伪造委派路径挂 local-only 下", "GET", "/api/admin/api/bridge/agent"),
        # 穿越 / 编码 / 注入
        ("🔴 路径穿越", "POST", "/api/sessions/../../etc/passwd"),
        ("🔴 编码绕过（%2e%2e）", "POST", "/api/sessions/%2e%2e/admin"),
        ("🔴 编码绕过（%2f）", "POST", "/api%2fsessions"),
        ("🔴 通配符路径（正则不该被字面 * 匹配）", "POST", "/api/sessions/*"),
        ("🔴 前缀近似不匹配（少一段）", "POST", "/api/session"),
        ("🔴 多余子路径", "POST", "/api/sessions/abc/messages/extra"),
        ("🔴 换行注入", "POST", "/api/sessions\nevil: 1"),
        ("🔴 CR 注入", "POST", "/api/sessions\r\nX: 1"),
        ("🔴 非 / 开头（相对路径）", "POST", "api/sessions"),
        ("🔴 scheme 伪装", "POST", "http://evil/api/sessions"),
        ("🔴 双斜杠变体", "POST", "//api/sessions"),
        ("🔴 大小写绕过", "POST", "/API/SESSIONS"),
        ("🔴 本地 wiki 写入（非委派面）", "POST", "/api/wiki/write_page"),
        ("🔴 本地用户管理", "POST", "/api/users/1/reset-password"),
    ]
    for label, method, path in deny_paths:
        expect(label, "DENY", "ALLOW" if m.is_tunnel_path_allowed(method, path) else "DENY")

    # ── 🔴 防死代码回归：方法的唯一真相是 RULE_METHODS ──────────────────────
    # backend 指出：DISPATCH_METHODS / READONLY_METHODS 曾「定义但零引用」，
    #   看起来是权威定义、实际改了零作用 → 比没有旋钮更危险。
    #   → 断言它们**不存在**（防止被重新引入并误导下一个人）。
    for dead in m.DEAD_CONSTANTS_MUST_NOT_EXIST:
        expect(f"死常量 {dead} 已删除（方法唯一真相是 RULE_METHODS）",
               "ABSENT", "ABSENT" if not hasattr(m, dead) else "PRESENT")
    # 方法表必须与规则集**同源**（每条规则都能在方法表里查到，且不多不少）
    rule_paths = {name.partition(" ")[2] for name, _ in m.TUNNEL_PATH_RULES}
    expect("RULE_METHODS 与规则集同源（无孤立项）", "YES",
           "YES" if set(m.RULE_METHODS) == rule_paths else f"NO:{set(m.RULE_METHODS) ^ rule_paths}")
    # 每条规则至少允许 1 个方法（否则是死规则）
    empty = [k for k, v in m.RULE_METHODS.items() if not v]
    expect("无空方法规则（每条规则至少 1 个方法）", "0", str(len(empty)))

    # ── local-only 与委派规则的**互斥性**不变量 ────────────────────────────
    # ⚠️ 实测发现（2026-10-04，devops）：当前规则集下 local-only 检查是
    #   **纯纵深防御** —— 委派正则全部以 `/api/bridge/dispatch` 开头，
    #   而 local-only 前缀里最近的 `/api/bridge/control` 与之**互斥**，
    #   所以「删掉 local-only 检查」不会改变任何用例结果（自检无法判红）。
    #
    # 🔴 这个「无法判红」本身是**必须被锁住的不变量**，否则将来有人给白名单
    #   加一条 `/api/bridge/*` 之类的宽规则时，local-only 就会静默失效。
    #   → 下面断言「委派正则的前缀集」与「local-only 前缀集」**互不相交**。
    lo_prefixes = tuple(m.LOCAL_ONLY_PREFIXES)
    # ⚠️ backend v2.5 后规则名格式为 "POST ^/path$"，取空格后的正则部分
    rule_prefixes = tuple(sorted({name.partition(" ")[2].split("?")[0].rstrip("$/")
                                 for name, _ in m.TUNNEL_PATH_RULES}))
    overlap = [p for p in lo_prefixes
               for r in rule_prefixes
               if r and (p == r or p.startswith(r + "/") or r.startswith(p + "/"))]
    expect("委派规则与 local-only 前缀互斥（local-only 为有效纵深防御）",
           "NO-OVERLAP", "NO-OVERLAP" if not overlap else f"OVERLAP:{overlap}")

    # ── 🔴 P0-9 不变量：凭据库前缀**必须在**禁止项里（规则存在性）──────────
    # ⚠️ 实测（2026-10-04）：只断言「GET /api/vault/credentials 被拒」是**不够**的 ——
    #   删掉 /api/vault 禁止项后，该路径**仍然 DENY**，因为它不匹配任何白名单正则，
    #   落到了「默认拒绝」。→ 行为断言被默认拒绝**掩盖**，测不出规则被删。
    #   与 G6 同类问题，但 vault 风险更高：一旦将来有人加 `/api/vault/...` 白名单，
    #   没有任何行为用例会拦住它 —— **A 就能读 B 的凭据**。
    #   → 故改为断言「规则存在性」：这条前缀必须在 LOCAL_ONLY_PREFIXES 里。
    for required in ("/api/vault", "/api/auth", "/api/admin", "/api/bridge/control"):
        expect(f"local-only 覆盖 {required}", "YES",
               "YES" if any(p == required or p.startswith(required) for p in lo_prefixes) else "NO")

    # 空输入必须拒（不得因「空值走默认分支」而放行）
    expect("空方法", "DENY", "ALLOW" if m.is_tunnel_path_allowed("", "/api/bridge/dispatch") else "DENY")
    expect("空路径", "DENY", "ALLOW" if m.is_tunnel_path_allowed("POST", "") else "DENY")

    # ═══════════════════════════════════════════════════════════════════════
    section("③ 敏感头剥离")

    must_strip = [
        "authorization", "Authorization", "AUTHORIZATION",
        "cookie", "Cookie",
        "host", "Host",
        "content-length",
        "connection", "Connection",
        "keep-alive",
        "transfer-encoding",
        "upgrade",
        "te", "trailer",
        "proxy-authorization", "proxy-authenticate",
        "x-forwarded-for", "x-forwarded-host", "x-real-ip",
        "x-bridge-peer", "x-bridge-node",
    ]
    for h in must_strip:
        out = m.strip_request_headers({h: "v", "x-keep": "k"})
        expect(f"剥离 {h}", "STRIP", "STRIP" if h not in out else "KEEP")

    # ★ 合法业务头不得被误伤（否则闸门会因为「剥太多」而破坏功能）
    must_keep = [
        "content-type", "Content-Type",
        "accept", "accept-language",
        "x-request-id", "x-correlation-id",
        "user-agent",
        "if-match", "etag",
    ]
    for h in must_keep:
        out = m.strip_request_headers({h: "v"})
        expect(f"保留 {h}", "KEEP", "KEEP" if h in out else "STRIP")

    # 入参不可被修改（避免调用方误以为已剥离）
    src = {"authorization": "Bearer X", "x-keep": "k"}
    m.strip_request_headers(src)
    expect("入参未被修改（无副作用）", "KEEP", "KEEP" if "authorization" in src else "STRIP")

    # ═══════════════════════════════════════════════════════════════════════
    section("④ 注入语义（连接所有者凭据）")

    out = m.inject_connection_owner_auth({"authorization": "Bearer ATTACKER", "x-keep": "k"}, "OWNER")
    expect("对端 authorization 被覆盖为连接所有者", "Bearer OWNER", out.get("authorization"))
    expect("覆盖后仍保留业务头", "k", out.get("x-keep", ""))

    out = m.inject_connection_owner_auth({"x-keep": "k"}, "OWNER")
    expect("无 authorization 时注入成功", "Bearer OWNER", out.get("authorization"))

    # 大小写形式的 authorization 也必须被覆盖
    out = m.inject_connection_owner_auth({"Authorization": "Bearer ATTACKER"}, "OWNER")
    keys_lower = {k.lower() for k in out}
    expect("大写 Authorization 也被覆盖（仅一个 authorization）", "1", str(sum(1 for k in out if k.lower() == "authorization")))

    # ═══════════════════════════════════════════════════════════════════════
    print(f"\n{'=' * 4} 结果：{_pass} 通过，{_failed} 失败 {'=' * 4}")
    return 1 if _failed else 0


if __name__ == "__main__":
    sys.exit(main())
