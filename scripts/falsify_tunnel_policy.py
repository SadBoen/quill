#!/usr/bin/env python3
"""
falsify_tunnel_policy.py —— 对 tunnel_policy_spec.py 注入 6 类故障，验证自检能否判红。

★ 存在的理由（铁律：未自检的闸门不算闸门）：
   本脚本证明 test_tunnel_policy.py **有鉴别力** —— 每一类真实防护被破坏时，
   自检必须报红。若某类故障注入后自检仍全绿，说明**自检有盲区**，必须补用例。

用法：python3 scripts/falsify_tunnel_policy.py
退出码：0=全部故障都被判红  1=有故障未被检出（有盲区）  2=环境问题
"""
from __future__ import annotations

import importlib.util
import io
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SPEC = os.path.join(HERE, "tunnel_policy_spec.py")
SELF_TEST = os.path.join(HERE, "test_tunnel_policy.py")

# (故障名, 说明, 原串, 新串)
FAULTS: list[tuple[str, str, str, str]] = [
    (
        "G1 白名单默认放行",
        "最危险的假闸门：未匹配的路径全部放行",
        "    # ★ 默认拒绝：走到这里说明没有任何规则匹配\n    return False",
        "    # 故障注入：默认放行\n    return True",
    ),
    (
        "G2 不限 HTTP 方法",
        "只列路径不列方法（07 §3.5 明确的常见漏洞）",
        "        if upper in allowed:",
        "        if True:  # 故障注入：不校验方法",
    ),
    (
        "G3 authorization 不剥离",
        "对端可注入自己的凭据冒充连接所有者",
        '    "authorization", "cookie", "proxy-authorization",\n',
        "",
    ),
    (
        "G4 IPv4-mapped 不折叠（纵深防御，应仍安全）",
        "折叠与 ::ffff:169.254.0.0/112 网段是两道独立防线；删掉折叠仍不得放行元数据",
        "    if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped is not None:\n        ip = ip.ipv4_mapped",
        "    # 故障注入：不折叠",
    ),
    (
        "G5 DNS 空结果放行",
        "DNS 解析失败时放行 = 绕过（fail-open）",
        '    if not resolved:\n        return SsrfDecision(False, "DNS 未返回任何地址")',
        '    if not resolved:\n        return SsrfDecision(True, "故障注入：放行")',
    ),
    (
        "G7 /api/vault 未禁止（P0-9 核心）",
        "若不禁 vault，A 派任务时可读 B 的凭据库 → P0-9 被直接绕过",
        '    "/api/vault",         # 🔴 凭据库：泄露即等于泄露 B 的登录口令\n',
        "",
    ),
    (
        "G6 local-only 面失效（纵深防御，应仍安全）",
        "委派正则与 local-only 前缀互斥 + 默认拒绝兜底；删掉 local-only 仍不得放行",
        '        if clean == prefix or clean.startswith(prefix + "/"):\n            return False',
        "        if False:\n            return False",
    ),
]


def run_self_test(spec_path: str) -> tuple[int, str]:
    """用指定的 spec 路径跑自检。"""
    env = dict(os.environ)
    env["TUNNEL_POLICY_SPEC"] = spec_path
    p = subprocess.run(
        [sys.executable, SELF_TEST],
        capture_output=True,
        text=True,
        env=env,
    )
    tail = [ln for ln in p.stdout.strip().splitlines() if ln.strip()]
    return p.returncode, (tail[-1] if tail else "(无输出)")


def main() -> int:
    if not os.path.isfile(SPEC) or not os.path.isfile(SELF_TEST):
        print("  x 缺少 spec 或 self-test 文件", file=sys.stderr)
        return 2

    original = io.open(SPEC, encoding="utf-8").read()

    # 基线：不注入故障时必须全绿，否则谈不上鉴别力
    rc, out = run_self_test(SPEC)
    print(f"基线（无故障）: {out}  exit={rc}")
    if rc != 0:
        print("  x 基线就不绿 —— 先修自检或策略，再谈鉴别力")
        return 2

    print()
    undetected: list[str] = []
    tmpdir = tempfile.mkdtemp(prefix="tps-falsify-")
    try:
        for name, desc, old, new in FAULTS:
            if old not in original:
                print(f"  x {name}: 锚点未找到 —— 策略源码已变，注入器需同步更新")
                undetected.append(f"{name}（锚点失效）")
                continue
            mutated = original.replace(old, new, 1)
            if mutated == original:
                print(f"  x {name}: 注入后内容未变（无效注入）")
                undetected.append(f"{name}（无效注入）")
                continue

            fpath = os.path.join(tmpdir, "spec_mutated.py")
            io.open(fpath, "w", encoding="utf-8", newline="").write(mutated)
            rc, out = run_self_test(fpath)
            # ★ 判据分两类（2026-10-04 实测得出，勿简化）：
            #   · 关键防线故障（G1/G2/G3/G5）→ 自检**必须报红**，否则自检有盲区
            #   · 纵深防御故障（G4/G6）→ 该防线**本就有第二道**，
            #     所以「仍报绿」是**正确且期望**的结果 —— 因为元数据/local-only
            #     仍被另一道防线拦住。若这里报红，反而说明两道防线是同一道。
            # ★ 判据取自 desc（原因说明），因为 G4/G6 的「纵深防御」定性写在那里
            redundant = "纵深防御" in desc or "纵深防御" in name
            if redundant:
                good = (rc == 0)
                mark = "仍安全" if good else "**意外报红**"
            else:
                good = (rc != 0)
                mark = "判红" if good else "**未判红**"
            print(f"  {mark:<12} {name} {out}  exit={rc}")
            print(f"               └ {desc}")
            if not good:
                undetected.append(name)
    finally:
        import shutil
        shutil.rmtree(tmpdir, ignore_errors=True)

    print()
    if undetected:
        print(f"结论：{len(undetected)}/{len(FAULTS)} 类故障未被检出 → **自检有盲区**")
        for n in undetected:
            print(f"  · {n}")
        return 1
    # ⚠️ 判据来源是 **name**（G4/G6 的「纵深防御」定性写在 name 里，
    #    写在 desc 里的是故障后果描述，措辞不同）—— 这里必须与主循环用同一来源，
    #    否则统计会与逐条结论自相矛盾（本项目踩过：先按 desc 统计得 6，
    #    再按 name 判定得 2，两处数字打架）。
    crit = [n for n, _, _, _ in FAULTS if "纵深防御" not in n]
    defn = [n for n, _, _, _ in FAULTS if "纵深防御" in n]
    print(f"结论：{len(crit)}/{len(crit)} 类关键防线故障全部判红 → 自检具备鉴别力")
    print(f"      另有 {len(defn)} 类纵深防御故障（G4 IPv4-mapped 折叠 / G6 local-only 面）：")
    print("      删掉它们**不会**导致放行（另一道防线兜底），所以「不报红」是**期望结果**。")
    print("      → 这两类已改为断言『防线仍生效』+『两防线互斥不变量』，而非断言『能判红』。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
