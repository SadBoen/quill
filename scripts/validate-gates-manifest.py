#!/usr/bin/env python3
"""
闸门 manifest 的 schema 校验器（契约 §3.2 / §4 / §9）

**为什么要独立成文件，而不是硬编码在 runner 里**：
硬编码在 runner 里，加第五道闸门时容易漏掉某条校验 —— 那是"约定"而非"强制"。
独立校验器有两个好处：
  1. **可单独测试**：`python3 validate-gates-manifest.py` 自己能被验证
  2. **单一真相源**：runner 与 CI 调同一个脚本，规则不会分叉

依据：`docs/FALSIFY_CONTRACT.md` §3.2（四字段）/ §4（不误报）/ §9（退役留痕）

退出码：0 = 合规 / 1 = 不合规（列出全部违规项）
"""

import json
import os
import sys

# ── 契约要求的字段集 ──────────────────────────────────────────────────────
# 🔴 这一处是"规则"的唯一定义处。加新规则只改这里，runner 与 CI 自动同步。
REQUIRED_TOP = ("gates",)

# 每道闸门（非退役）必填
REQUIRED_GATE = ("id", "script", "falsifiable")

# falsifiable=false 时**必须**给出四字段（契约 §3.2，主理人裁决"三字段缺一不可"）
FALSE_REQUIRES = ("reason", "verified_by", "verified_platform", "verified_at")

# 运行时依赖（契约 §0.4：检查器不得与被检查对象共享运行时依赖）
# 合法值： "none"（不依赖 cargo，可在工具链损坏时运行） / "cargo" / "python3" 等
RUNTIME_VALUES = ("none", "cargo", "python3", "bash")

# 退役条目必填（契约 §9：不得直接删）
RETIRED_REQUIRES = ("retired_at", "retired_reason")

# 建议字段（缺失只警告，不拒绝）
RECOMMENDED = ("self_test", "rules", "desc")


def validate(doc):
    """返回 (errors, warnings) 两个列表。"""
    errors, warnings = [], []

    for k in REQUIRED_TOP:
        if k not in doc:
            errors.append(f"manifest 缺顶层字段 `{k}`")
    if errors:
        return errors, warnings

    gates = doc["gates"]
    if not isinstance(gates, list):
        errors.append("`gates` 必须是数组")
        return errors, warnings
    if not gates:
        errors.append("`gates` 为空 —— 没有任何闸门可校验")

    seen_ids = set()

    for i, g in enumerate(gates):
        where = f"gates[{i}]"

        if not isinstance(g, dict):
            errors.append(f"{where} 必须是对象")
            continue

        gid = g.get("id", f"<第{i}项无 id>")
        where = f"[{gid}]"

        # id 唯一
        if gid in seen_ids:
            errors.append(f"{where} id 重复")
        seen_ids.add(gid)

        # ── 退役条目（契约 §9）──
        if g.get("retired_at"):
            missing = [f for f in RETIRED_REQUIRES if not g.get(f)]
            if missing:
                errors.append(
                    f"{where} 退役条目缺 {missing}（契约 §9 —— 删除记录与创建记录同样重要，"
                    f"否则半年后会有人重新实现一遍）"
                )
            if g.get("keep_entry") is not True:
                warnings.append(
                    f"{where} 退役条目建议显式写 \"keep_entry\": true（runner 靠它确认这是有意保留）"
                )
            # 退役条目不再校验 script/falsifiable
            continue

        # ── 常规条目：必填字段 ──
        missing = [f for f in REQUIRED_GATE if f not in g]
        if missing:
            errors.append(f"{where} 缺必填字段 {missing}")
            continue

        if not isinstance(g.get("falsifiable"), bool):
            errors.append(
                f"{where} `falsifiable` 必须是布尔值（当前为 {g.get('falsifiable')!r}）—— "
                f"字符串 \"false\" 会被当成真值，这是典型的静默失效"
            )

        # ── 契约 §3.5：self_test 不得等于 script（角色错位）──
        if g.get("self_test") and g.get("self_test") == g.get("script"):
            errors.append(
                f"{where} `self_test` 与 `script` 相同 —— **角色错位**（契约 §3.5）。\n"
                f"      script     = 被验证的对象（闸门本体）\n"
                f"      self_test  = 验证它的测试脚本（跑「会红」与「不该红」两侧）\n"
                f"      两者由同一人写时最容易踩 —— 那时它们看起来就是同一个东西。"
            )

        # ── 契约 §3.2：falsifiable=false 的四字段 ──
        if g.get("falsifiable") is False:
            missing = [f for f in FALSE_REQUIRES if not g.get(f)]
            if missing:
                errors.append(
                    f"{where} falsifiable:false 但缺 {missing}（契约 §3.2）。\n"
                    f"      reason          —— 否则变成「懒得验证」的借口\n"
                    f"      verified_by     —— 无人复验的 false 是**沉默的许可**\n"
                    f"      verified_platform —— G26-4 需 POSIX 软链，Windows 上本就是无法判定\n"
                    f"      verified_at     —— 否则永远是「待办」，没有实际约束力"
                )

        # ── 契约 §0.4：运行时依赖必须显式声明 ──
        rr = g.get("requires_runtime")
        if rr is None:
            errors.append(
                f"{where} 缺 `requires_runtime`（契约 §0.4 —— 检查器与被检查对象不得共享运行时依赖）。\n"
                f"      合法值：{RUNTIME_VALUES}。取 \"none\" 表示该闸门在 cargo 损坏时仍能运行。"
            )
        elif rr not in RUNTIME_VALUES:
            errors.append(f"{where} requires_runtime 非法：{rr!r}（合法值：{RUNTIME_VALUES}）")

        # ── 建议字段 ──
        for f in RECOMMENDED:
            if not g.get(f):
                warnings.append(f"{where} 建议声明 `{f}`")

    return errors, warnings


# ══════════════════════════════════════════════════════════════════════
# G35 自检：校验器自己必须能抓到各类违规
# ══════════════════════════════════════════════════════════════════════
# 理由（主理人）：这条校验器是「检查其他检查器配置」的 ——
# 若它自己有假绿，整个 manifest 治理就是假的。
# 所以不能只手工验证一次，必须留成**可执行证据**。
CASES = [
    # (用例名, manifest dict, 期望命中的错误关键词)
    ("falsifiable:false 缺四字段",
     {"gates": [{"id": "G1", "script": "a.sh", "falsifiable": False,
                 "requires_runtime": "none", "reason": "x", "verified_by": "y",
                 "verified_platform": "z"}]},          # 缺 verified_at
     "verified_at"),
    ("falsifiable 为字符串 \"false\"",
     {"gates": [{"id": "G1", "script": "a.sh", "falsifiable": "false",
                 "requires_runtime": "none"}]},
     "必须是布尔值"),
    ("缺必填字段 script",
     {"gates": [{"id": "G1", "falsifiable": True, "requires_runtime": "none"}]},
     "缺必填字段"),
    ("id 重复",
     {"gates": [{"id": "G1", "script": "a.sh", "falsifiable": True,
                 "requires_runtime": "none"},
                {"id": "G1", "script": "b.sh", "falsifiable": True,
                 "requires_runtime": "none"}]},
     "id 重复"),
    ("退役条目缺 retired_reason",
     {"gates": [{"id": "G9", "retired_at": "2026-01-01"}]},
     "retired_reason"),
    ("缺 requires_runtime",
     {"gates": [{"id": "G1", "script": "a.sh", "falsifiable": True}]},
     "requires_runtime"),
    ("requires_runtime 非法值",
     {"gates": [{"id": "G1", "script": "a.sh", "falsifiable": True,
                 "requires_runtime": "rust-toolchain-please"}]},
     "非法"),
    ("gates 为空",
     {"gates": []},
     "没有任何闸门"),
    ("顶层缺 gates",
     {},
     "缺顶层字段"),
]

GOOD = {"gates": [{"id": "G1", "script": "a.sh", "falsifiable": True,
                   "requires_runtime": "none", "self_test": "t.sh",
                   "rules": ["R1"], "desc": "示例"}]}


def self_test():
    """返回 (通过数, 失败列表)。"""
    failures = []

    # 1. 正向：合规 manifest 必须**不**报错
    errs, _ = validate(GOOD)
    if errs:
        failures.append(f"[误报] 合规 manifest 被判不合规：{errs}")

    # 2. 反向：每类违规都必须被抓到
    for name, doc, kw in CASES:
        errs, _ = validate(doc)
        joined = " ".join(errs)
        if kw not in joined:
            failures.append(
                f"[漏报] {name}：期望错误含 {kw!r}，实际得到：{errs or '（无错误 —— 漏报！）'}"
            )

    return len(CASES) + 1 - len(failures), failures


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--self-test":
        passed, failures = self_test()
        total = len(CASES) + 1
        if failures:
            print(f"❌ G35 自检失败：{len(failures)}/{total} 项未通过", file=sys.stderr)
            for f in failures:
                print(f"   {f}", file=sys.stderr)
            print("\n⚠️ 校验器自己有假绿 → 整个 manifest 治理是假的。禁止发布。", file=sys.stderr)
            return 1
        print(f"✓ G35 自检通过：{passed}/{total}（1 正向 + {len(CASES)} 反向）")
        print("  每类违规都被确认会抓到 —— 校验器不是摆设。")
        return 0

    if len(sys.argv) < 2:
        print("用法：python3 validate-gates-manifest.py <manifest.json>", file=sys.stderr)
        return 2

    path = sys.argv[1]
    try:
        with open(path, encoding="utf-8") as f:
            doc = json.load(f)
    except FileNotFoundError:
        print(f"❌ manifest 不存在：{path}", file=sys.stderr)
        return 1
    except json.JSONDecodeError as e:
        print(f"❌ manifest 不是合法 JSON：{e}", file=sys.stderr)
        return 1

    errors, warnings = validate(doc)

    for w in warnings:
        print(f"⚠️  {w}", file=sys.stderr)
    for e in errors:
        print(f"❌ {e}", file=sys.stderr)

    if errors:
        print(f"\n❌ manifest 不合规：{len(errors)} 个错误 / {len(warnings)} 个警告", file=sys.stderr)
        print("   依据 docs/FALSIFY_CONTRACT.md §3.2 / §4 / §9", file=sys.stderr)
        return 1

    n = len(doc.get("gates", []))
    ret = sum(1 for g in doc["gates"] if g.get("retired_at"))
    print(f"✓ manifest 合规：{n} 道闸门（其中 {ret} 道已退役）")
    if warnings:
        print(f"  {len(warnings)} 个警告（非阻塞）")
    return 0


# ══════════════════════════════════════════════════════════════════════
# G35 第 11 项：空闸门 / 恒绿闸门必须【无法自证】
# ══════════════════════════════════════════════════════════════════════
# 背景（主理人 2026-10-04 发现的根因缺陷）：
#   契约初版 demo 写的是 `if (( FALSIFY )); then exit 1; fi` —— 无条件报红。
#   结果「什么都不检查的闸门」与「永远判绿的闸门」都能通过 --falsify 自证。
#   **那正是契约 §3 自己警告的"恒绿闸门"，却藏在示例代码里。**
#
# 本项把该缺陷固化成可执行证据：构造两个假闸门，确认 runner 拒绝它们。
FAKE_GATES = {
    "empty": """#!/usr/bin/env bash
# 反例：什么都不检查，只在 --falsify 时无条件报红（契约初版的写法）
[[ "${2:-}" == "--falsify" ]] && { echo "x" >&2; exit 1; }
echo "一切正常"; exit 0
""",
    "always_green": """#!/usr/bin/env bash
# 反例：永远判绿
echo "一切正常"; exit 0
""",
}


def check_fake_gates(script_dir):
    """构造假闸门并检查 runner 的判定。返回失败列表。"""
    import subprocess, tempfile, os, json
    failures = []
    real_gates = os.path.join(os.path.dirname(os.path.abspath(__file__)), "gates.manifest.json")
    runner = os.path.join(os.path.dirname(os.path.abspath(__file__)), "run-gate-selftest.sh")
    if not (os.path.exists(real_gates) and os.path.exists(runner)):
        return ["缺少 runner 或 manifest，无法执行第 11 项"]

    for kind, body in FAKE_GATES.items():
        with tempfile.TemporaryDirectory() as td:
            # ⚠️ 假闸门**必须放在 scripts/ 下**：runner 以 ROOT 为基准解析 script 路径，
            #    放在临时目录会因路径解析不到而判「脚本不存在」——
            #    那样测的是「找不到脚本」，不是「假自证」，用例就白写了。
            here = os.path.dirname(os.path.abspath(__file__))
            rel = f".fals-gate-{kind}.probe.sh"   # 相对 scripts/（runner 的 ROOT 就是 scripts/ 的父）
            fake = os.path.join(os.path.dirname(here), rel)
            with open(fake, "w", encoding="utf-8") as f:
                f.write(body)
            os.chmod(fake, 0o755)
            m = {"gates": [{"id": "G-FAKE", "script": rel,
                            "self_test": rel, "falsifiable": True,
                            "requires_runtime": "bash",
                            "rules": ["R-1"], "desc": "假闸门检测"}]}
            mf = os.path.join(td, "m.json")
            with open(mf, "w", encoding="utf-8") as f:
                json.dump(m, f)
            # 只跑反向模式：假闸门必须被拒绝
            env = dict(os.environ, QUILL_GATES_MANIFEST=mf)
            r = subprocess.run(["bash", runner, "--falsify"],
                               capture_output=True, text=True, env=env)
            try:
                os.remove(fake)     # 清理：假闸门绝不能留在仓库里
            except OSError:
                pass
            if r.returncode == 0:
                failures.append(
                    f"[漏报] 「{kind}」假闸门通过了反向自检 —— "
                    f"契约 demo 仍是无条件报红写法？runner 缺 SELF-TEST 判定？"
                )
    return failures


def self_test_extended():
    passed, failures = self_test()
    if os.environ.get("QUILL_SKIP_FAKE_GATE") != "1":
        failures += check_fake_gates(os.path.dirname(os.path.abspath(__file__)))
        passed = passed + 1 - len([f for f in failures if "假闸门" in f])
    return passed, failures


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        p2, f2 = self_test_extended()
        t2 = len(CASES) + 2
        if f2:
            print(f"❌ G35 自检失败：{len(f2)}/{t2} 项未通过", file=sys.stderr)
            for x in f2:
                print(f"   {x}", file=sys.stderr)
            print("\n⚠️ 校验器自己有假绿 → 整个 manifest 治理是假的。禁止发布。", file=sys.stderr)
            sys.exit(1)
        print(f"✓ G35 自检通过：{p2}/{t2}（1 正向 + {len(CASES)} 反向 + 1 假闸门检测）")
        print("  含：空闸门与恒绿闸门【均无法自证】—— 无条件报红的写法已被堵死")
        sys.exit(0)
    sys.exit(main())
