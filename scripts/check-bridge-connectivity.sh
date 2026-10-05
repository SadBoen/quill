#!/usr/bin/env bash
# =============================================================================
# check-bridge-connectivity.sh —— 隧道连通性自检（三层，**分开报告**）
# =============================================================================
# 🔴🔴 本脚本最重要的设计原则（task #16 的核心要求）：
#   **禁止把「本地能连」报成「跨网能用」。**
#
#   三层各自独立报告，**不得合并、不得互相推断**：
#   ┌─ L1 声明式（离线可跑）—— 配置能否建起来：路径白名单/方法/SSRF/端点声明齐不齐
#   ├─ L2 行为式（本地双节点）—— 真实握手 + 一次委派往返
#   └─ L3 跨网路径（需外部网络）—— 真跨网/真反代/真 TLS
#
#   为什么必须分开：L2 在本机回环上通过，**完全不能**推出 L3 可用
#   （NAT/防火墙/反代超时/DNS/TLS 链路上任何一环都能让 L2 绿而 L3 红）。
#   历史上「本地测试全绿 → 上线连不上」是最常见的误判来源。
#
# 退出码（三态，与仓库契约一致）：
#   0 = 全绿      1 = 有问题      2 = 无法判定（**绝不计入通过**）
#   ⚠️ 只要 L3 未跑，整体**最高只能是 2**（未判定），绝不因 L1/L2 绿就报 0。
# =============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SPEC="$ROOT/scripts/tunnel_policy_spec.py"
PY="${PYTHON:-python3}"
CRATE="$ROOT/crates/quill-bridge"

L1="skipped"; L2="skipped"; L3="skipped"
declare -a NOTES=()

note(){ NOTES+=("$1"); printf '  · %s\n' "$1"; }

printf '═══ 隧道连通性自检（三层）═══\n\n'

# ── L1 声明式（离线可跑）────────────────────────────────────────────────
printf '【L1 声明式】配置能否建起来\n'
if ! command -v "$PY" >/dev/null 2>&1; then
  L1="undetermined"; note "缺 python3 —— 无法判定"
elif [[ ! -f "$SPEC" ]]; then
  L1="undetermined"; note "缺策略规范 —— 无法判定"
else
  "$PY" "$ROOT/scripts/test_tunnel_policy.py" >/dev/null 2>&1
  case $? in
    0) L1="pass"; note "路径白名单/方法级/SSRF/头剥离 104 项全部通过（离线可验证部分）" ;;
    1) L1="fail";  note "策略自检有失败用例 —— 配置层已就绪但规则被削弱" ;;
    *) L1="undetermined"; note "策略自检无法判定" ;;
  esac
  # 端点声明一致性：部署配置必须与规范同源
  if [[ -f "$ROOT/deploy/quill-bridge.env.example" ]]; then
    if "$PY" - "$ROOT/deploy/quill-bridge.env.example" "$SPEC" <<'PYEOF'
import importlib.util, re, sys
env = open(sys.argv[1], encoding="utf-8").read()
spec = importlib.util.spec_from_file_location("tps", sys.argv[2])
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
lo = set(re.search(r"^TUNNEL_LOCAL_ONLY_PREFIXES=(.+)$", env, re.M).group(1).split(","))
assert lo == set(m.LOCAL_ONLY_PREFIXES), f"local-only 不一致: {lo ^ set(m.LOCAL_ONLY_PREFIXES)}"
rules = []
i = 1
while True:
    mm = re.search(rf"^TUNNEL_RULE_{i}=(.+)$", env, re.M)
    if not mm: break
    meth, _, path = mm.group(1).partition(" ")
    rules.append((frozenset(meth.split("|")), path)); i += 1
spec_rules = [(frozenset(v), k) for k, v in m.RULE_METHODS.items()]
assert set(rules) == set(spec_rules), f"白名单规则不一致: env={len(rules)} spec={len(spec_rules)}"
body = re.search(r"^MAX_TUNNEL_BODY_BYTES=(\d+)$", env, re.M).group(1)
assert int(body) == m.MAX_TUNNEL_BODY_BYTES, "body 上限不一致"
PYEOF
    then
      note "部署配置与策略规范同源（白名单/local-only/body 上限一致）"
    else
      L1="fail"; note "🔴 部署配置与策略规范**不一致** —— 改一处必须改两处"
    fi
  else
    note "无 deploy/quill-bridge.env.example —— 端点声明未判定"
    [ "$L1" = "pass" ] && L1="undetermined"
  fi
fi
printf '  → L1 = %s\n\n' "$L1"

# ── L2 行为式（本地双节点握手 + 委派往返）─────────────────────────────
printf '【L2 行为式】本地双节点真实握手 + 一次委派往返\n'
# ⚠️ 诚实说明：L2 需要**可运行的隧道端点**（WS 服务 + ACP 帧 + 委派处理器）。
#    `quill-bridge` 目前是 38 行骨架占位，端点尚未实现 →
#    **不能**用「mock 返回成功」冒充 L2 通过。
if [[ ! -d "$CRATE" ]]; then
  L2="undetermined"; note "crates/quill-bridge 不存在 —— 无法判定"
elif ! grep -rqE 'fn handle|axum|Router|WebSocket|tower_http' "$CRATE/src" 2>/dev/null; then
  L2="undetermined"
  note "🔴 quill-bridge 仍是骨架（无可运行隧道端点）—— L2 **未跑**"
  note "   ⚠️ 绝不用 mock 冒充 L2 通过：本机能连 ≠ 跨网能连（见文件头原则）"
else
  # 有端点实现时，才做真实双节点握手（此处留待实现落地后接入）
  L2="undetermined"
  note "检测到隧道端点代码 —— 但双节点握手用例尚未接入（需 backend 提供可运行 demo）"
fi
printf '  → L2 = %s\n\n' "$L2"

# ── L3 跨网路径（需外部网络，CI 保证不了）──────────────────────────────
printf '【L3 跨网路径】真跨网 / 真反代 / 真 TLS\n'
L3="undetermined"
if [[ "${QUILL_BRIDGE_E2E:-0}" != "1" ]]; then
  note "🔴 **L3 未跑** —— 需显式 QUILL_BRIDGE_E2E=1 且提供对端实例"
  note "   CI 保证不了这一层：NAT/防火墙/反代超时/DNS/TLS 任何一环都能让 L2 绿而 L3 红"
elif [[ -z "${QUILL_BRIDGE_PEER_BASE_URL:-}" ]]; then
  note "已请求 E2E 但未提供 QUILL_BRIDGE_PEER_BASE_URL —— 无法判定"
else
  note "E2E 已请求但本脚本尚未实现跨网探针 —— 无法判定（不谎报通过）"
fi
printf '  → L3 = %s\n\n' "$L3"

# ── 汇总：三层分开报告，L3 未跑则整体最高 2 ────────────────────────────
printf '═══ 汇总 ═══\n'
printf '  L1 声明式（离线）    : %s\n' "$L1"
printf '  L2 行为式（本地）    : %s\n' "$L2"
printf '  L3 跨网（需外部网络） : %s\n' "$L3"
echo
for n in "${NOTES[@]}"; do printf '  · %s\n' "$n"; done
echo

if [ "$L1" = "fail" ] || [ "$L2" = "fail" ] || [ "$L3" = "fail" ]; then
  echo "结论：有层判定为**违规**。"
  exit 1
fi
if [ "$L3" != "pass" ]; then
  echo "结论：L1/L2 状态见上，但 **L3 跨网未验证** → 整体**未判定**，不等于通过。"
  echo "      ⚠️ 禁止据此宣称「委派功能可用」—— 跨网可用性只有 L3 能证明。"
  exit 2
fi
echo "结论：三层全部通过。"
exit 0
