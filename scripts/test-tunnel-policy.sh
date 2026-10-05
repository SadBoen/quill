#!/usr/bin/env bash
# =============================================================================
# test-tunnel-policy.sh —— G36 自检入口（bash wrapper）
# =============================================================================
# ⚠️ 为什么需要这个 wrapper（实测踩坑，勿删）：
#   `scripts/run-gate-selftest.sh:214` 对 self_test **硬编码 `bash` 调用**：
#       out="$(bash "${ROOT}/${st}" 2>&1)"
#   所以 self_test **必须是 bash 脚本**。我最初把 self_test 直接指向
#   `test_tunnel_policy.py`（Python），结果 runner 用 bash 去跑它：
#       line 23: from: command not found
#   → self_test 变成「永远无法判定」，闸门恒为 rc=2，**而 manifest 校验通过**
#     （它只检查字段非空，不检查解释器是否匹配）。
#   **这是一道「看起来合规、实际什么都没跑」的闸门 —— 比没有闸门更危险。**
#   → 教训：新增闸门时，`requires_runtime` 与 self_test 的**解释器**必须匹配。
#
# 退出码：0=全部通过 / 1=有用例失败 / 2=无法判定（缺 python3 等）
# =============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PY="${PYTHON:-python3}"

if ! command -v "$PY" >/dev/null 2>&1; then
  # 措辞合规：rc=2 必须含「无法判定」或 ▲（run-gate-selftest.sh check_wording）
  printf '  ▲ 缺工具 %s —— 无法判定（不等于通过）\n' "$PY" >&2
  exit 2
fi

exec "$PY" "$ROOT/scripts/test_tunnel_policy.py" "$@"
