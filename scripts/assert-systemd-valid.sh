#!/usr/bin/env bash
# ==============================================================================
# assert-systemd-valid.sh —— systemd 单元文件合法性闸门
#
# ⚠️ 为什么要专门写这个脚本（E10 实测，本项目发现的最危险陷阱）：
#
#   $ printf "[Unit\nDescription=bad\n" > /tmp/bad.service   # 语法完全错误
#   $ systemd-analyze verify /tmp/bad.service; echo "退出码=$?"
#   退出码=0                          # ← 仍然返回 0！
#   /tmp/bad.service:1: Invalid section header '[Unit'   # 只在 stderr
#
# 因此写成 `systemd-analyze verify unit && echo OK` 的闸门
# **会永远绿灯，放过每一个坏 unit** —— 比没有闸门更危险，因为它给人虚假的安全感。
#
# 【本脚本 v2 的关键设计：先过滤良性噪音，再判定】
#   systemd-analyze verify 会把"ExecStart 指向的文件此刻不存在"也报成错误：
#       good.service: Command /opt/quill/current/quill-ctl is not executable:
#                     No such file or directory
#   这在 CI 上是**必然发生**的（CI 没装 Quill），且它对单元语法是正确的。
#   若把它当失败 → 闸门永远红 → 同样是无用的闸门。
#   故：先滤掉此类"环境性"提示，只对**真正的语法/指令错误**判红。
#
# 用法：
#   scripts/assert-systemd-valid.sh deploy/quill.service
#   scripts/assert-systemd-valid.sh deploy/*.service     # 全部通过才退出 0
#
# 退出码：0=全部合法 / 1=存在非法单元 / 2=用法错误或环境缺 systemd-analyze
#
# ⚠️ 闸门自己也要被闸门检查：nightly CI 有"闸门自检"用例反向验证本脚本
#    （见 docs/07_部署运维方案.md §3.5）。
# ==============================================================================
set -uo pipefail

# --- 颜色（注意：不可先 readonly 再赋值，必须一次声明） --------------------
if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; DIM=$'\033[2m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; DIM=''; RESET=''
fi

usage() {
  cat >&2 <<'EOF'
用法：assert-systemd-valid.sh <unit.service> [unit2.service ...]

退出码：0=全部合法  1=存在非法单元  2=用法错误 / 环境缺 systemd-analyze
EOF
  exit 2
}

die() { local code="$1"; shift; printf '%s\n' "${RED}✗ $1${RESET}" >&2; shift
        while (($#)); do printf '%s\n' "  $1" >&2; shift; done; exit "$code"; }

# --- 前置检查 -------------------------------------------------------------
command -v systemd-analyze >/dev/null 2>&1 \
  || die 2 "环境缺 systemd-analyze，无法校验单元文件。" \
           "安装：apt-get install -y systemd" \
           "⚠️ 禁止用 '|| true' 跳过 —— 那就是没有闸门。"

(($# >= 1)) || usage

# --- 判定核心 -------------------------------------------------------------
# 良性噪音：单元本身没问题，只是被引用的文件在当前机器上不存在。
# CI 上几乎必然出现（未安装 Quill），因此必须滤除，否则闸门永远红。
readonly BENIGN_PAT='is not executable: No such file or directory|Unable to (resolve|open)|No such file or directory$|man( page)?|documentation'

check_one() {
  local unit="$1" out rc filtered
  [[ -f "$unit" ]] || { printf '%s\n' "${RED}✗ 单元文件不存在：$unit${RESET}" >&2; return 1; }

  out=$(systemd-analyze verify "$unit" 2>&1); rc=$?
  filtered=$(printf '%s' "$out" | grep -vE "$BENIGN_PAT" || true)

  # 情形 A：滤后仍有输出 → 真正的语法/指令错误 → 失败
  if [[ -n "${filtered//[[:space:]]/}" ]]; then
    if ((rc == 0)); then
      printf '%s\n' "${RED}✗ $unit 非法 ���注意 systemd-analyze 退出码是 0，这是假绿灯）${RESET}" >&2
    else
      printf '%s\n' "${RED}✗ $unit 非法 (systemd-analyze 退出码 $rc)${RESET}" >&2
    fi
    printf '%s\n' "$filtered" | sed 's/^/    /' >&2
    return 1
  fi

  # 情形 B：无真实错误但退出码非 0 → 仅是环境性提示 → 放行但告警
  if ((rc != 0)); then
    printf '%s\n' "${YELLOW}⚠ $unit 语法合法，但存在环境性提示（不影响 CI 判定）${RESET}"
    printf '%s\n' "$out" | grep -E "$BENIGN_PAT" | sed 's/^/    /' >&2 || true
    return 0
  fi

  # 情形 C：完全干净
  printf '%s\n' "${GREEN}✓ $unit 合法${RESET}"
  return 0
}

# --- 主流程 ---------------------------------------------------------------
failed=0; total=0
for unit in "$@"; do
  total=$((total + 1))
  check_one "$unit" || failed=$((failed + 1))
done

if ((failed > 0)); then
  printf '\n%s\n' "${RED}═══ ${failed}/${total} 个单元文件非法 ═══${RESET}" >&2
  exit 1
fi

printf '\n%s\n' "${GREEN}═══ ${total}/${total} 个单元文件全部合法 ═══${RESET}"
printf '%s\n' "${YELLOW}提醒：语法合法 ≠ 能正常启动。仍需实机验证：${RESET}"
printf '%s\n' "  ${DIM}scripts/verify-unit.sh   # 装上去真跑一遍 + 验证 watchdog 能杀卡死进程${RESET}"
exit 0
