#!/usr/bin/env bash
# ==============================================================================
# test-newtype-guard.sh —— G41 的自检：证明 NTG 真的能判红、也真的不误报
# ==============================================================================
# 主理人维护。
#
#   用法：bash scripts/test-newtype-guard.sh
#   退出码：0=自检通过 / 1=自检失败 / 2=无法判定
#
# 为什么需要这一层（而不是把 NTG 直接登记成闸门）：
#   manifest 契约要求每道闸门有**独立的** self_test，且 `self_test != script`
#   （角色错位检查：自己验自己的脚本最容易两边一起错）。
#   本包装器负责「跑 NTG 的两个方向 + 注入一个恒绿副本看它会不会被骗」。
# ==============================================================================
set -uo pipefail

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; DIM=$'\033[2m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; DIM=''; RESET=''
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NTG="${ROOT}/crates/quill-testkit/scripts/test-newtype-guard.sh"

PASS=0; FAIL=0
declare -a FAILED=()

ok()   { PASS=$((PASS+1)); printf '  %s✓%s %s\n' "$GREEN" "$RESET" "$1"; }
bad()  { FAIL=$((FAIL+1)); FAILED+=("$1"); printf '  %s✗%s %s\n' "$RED" "$RESET" "$1"; }
undet(){ printf '  %s▲%s %s\n' "$YELLOW" "$RESET" "$1"; exit 2; }

printf '%s\n' "${DIM}── G41 自检 ──${RESET}"

[[ -f "$NTG" ]] || undet "被检脚本不存在：${NTG}（未检查，不计入通过）"
command -v cargo >/dev/null 2>&1 || {
  [[ -x "$HOME/.cargo/bin/cargo" ]] && export PATH="$HOME/.cargo/bin:$PATH"
  command -v cargo >/dev/null 2>&1 || undet "本机无 cargo，编译验证无法执行（未检查，不计入通过）"
}

# ── 第①组 正常方向：NTG 必须 rc=0（误用被拒 + 合法用法放行）──────────────────
out="$(bash "$NTG" 2>&1)"; rc=$?
if (( rc == 0 )); then
  ok "正常模式 rc=0（误用被编译器拒绝 + 合法用法放行）"
else
  bad "正常模式 rc=$rc（期望 0）"; printf '%s\n' "$out" | tail -12 | sed 's/^/      /'
fi

# ── 第②组 反向方向：--falsify 必须 rc=1（证明它分得清「拦住」与「没拦住」）─────
out="$(bash "$NTG" --falsify 2>&1)"; rc=$?
if (( rc == 1 )); then
  ok "--falsify rc=1（注入的合法探针被判「未拦住」→ 鉴别力成立）"
elif (( rc == 2 )); then
  undet "--falsify 返回 rc=2「无法判定」，本项不计入通过"
else
  bad "--falsify rc=$rc（期望 1；rc=0 说明它对任何输入都说通过 = 恒绿）"
  printf '%s\n' "$out" | tail -12 | sed 's/^/      /'
fi

# ── 第③组 故障注入：造一个「永远判绿」的 NTG 副本 ────────────────────────────
# 🔴 这一组是本自检的鉴别力来源。但注入点必须选对：
#   · 拿**真实 fixture** 去测短路副本是没意义的 —— 真实误用本来就被拦住，
#     副本和原版都会 rc=0，测不出任何东西。
#   · 正确做法是拿**探针**（全部合法的 fixture）去测：原版必须判「未拦住」rc=1；
#     若副本把这条判红路径短路成 rc=0，它就暴露成恒绿。
TMPD="$(mktemp -d)"
trap 'rm -rf "$TMPD"' EXIT
BROKEN="$TMPD/broken-ntg.sh"
# 短路「编译成功即判违规」那条分支里的 exit 1（第 143 行，两空格缩进）
if ! python3 - "$NTG" "$BROKEN" "$ROOT" <<'PYEOF'
import io, sys
src, dst, root = sys.argv[1], sys.argv[2], sys.argv[3]
lines = io.open(src, encoding="utf-8").read().split("\n")
# 短路「编译成功 → 判违规」分支里的 `  exit 1`（两空格缩进，第 143 行）
hits = 0
for i, l in enumerate(lines):
    if l == "  exit 1" and i > 0 and "NTG_PROBE_LEGAL" in "\n".join(lines[max(0, i-12):i]):
        lines[i] = '  echo "[G41-self-test] broken copy: pretending compliant";\n  exit 0'
        hits += 1
        break
if hits == 0:
    sys.exit(3)
out = "\n".join(lines)
# 副本在 tmp 下 REPO_ROOT 会指错，钉死为真实仓库根
out = "\n".join(
    ('REPO_ROOT="%s"' % root) if l.startswith("REPO_ROOT=") else l
    for l in out.split("\n"))
io.open(dst, "w", encoding="utf-8").write(out)
sys.exit(0)
PYEOF
then
  bad "故障注入失败：短路副本与原版完全相同，本组没有真的注入故障（锚点行已漂移）"
else
  out="$(bash "$BROKEN" --falsify 2>&1)"; rc=$?
  # 🔴 实测结论（比预期更强）：短路副本的 --falsify 返回 **rc=2「无法自证」**，
  #    不是 rc=0。原因是 --falsify 层拿**被测副本的退出码**当判据：
  #    副本判绿(0) ≠ 期望的判红(1) → 自证直接判「检测逻辑已失效」并退 2。
  #    这正是「结论必须来自返回值」的价值：短路没能骗过自证。
  if (( rc == 2 )); then
    ok "故障注入被识破：短路副本的 --falsify 返回 rc=2「无法自证」—— 短路骗不过自证"
  elif (( rc == 0 )); then
    bad "短路副本返回 rc=0：自证层没拦住它，--falsify 形同虚设"
    printf '%s\n' "$out" | tail -8 | sed 's/^/      /'
  else
    bad "短路副本返回 rc=$rc（期望 2「无法自证」；rc=1 说明短路没生效）"
    printf '%s\n' "$out" | tail -8 | sed 's/^/      /'
  fi
fi

# 反向断言：短路副本必须**表现出**被短路，否则上面那条结论不成立
out="$(bash "$BROKEN" --falsify 2>&1)"; rc=$?
if printf '%s' "$out" | grep -q 'broken copy: pretending compliant'; then
  ok "反向断言：短路确实生效（副本输出含短路标记，退出码 ${rc}）"
else
  bad "反向断言失败：副本未表现出被短路，注入可能没作用（rc=$rc）"
fi

printf '\n%s\n' "${DIM}────────────────────────────${RESET}"
if (( FAIL )); then
  printf '%s\n' "${RED}❌ G41 自检失败：${PASS} 通过 / ${FAIL} 失败${RESET}" >&2
  for c in "${FAILED[@]}"; do printf '   · %s\n' "$c" >&2; done
  exit 1
fi
printf '%s\n' "${GREEN}✓ G41 自检通过：${PASS}/${PASS}（含 1 故障注入 + 1 反向断言）${RESET}"
exit 0
