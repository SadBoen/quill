#!/usr/bin/env bash
# ==============================================================================
# check-no-external-refs.sh —— G29 产物自包含闸门
#
# 规则：ui/web/dist/ 里不允许任何运行时外部资源引用。
#   ❌ <link href="https://cdn.jsdelivr.net/...">
#   ❌ <script src="https://unpkg.com/...">
#   ❌ 外部字体 / 遥测 / 分析脚本
#
# 为什么：目标场景是家用 NAS 与内网 VPS —— **断外网是常态，不是异常**。
# 外链资源失败时**不报错，只是白屏**（典型的静默失败），用户与运维都难定位。
# 这与「QUILL_WEB_DIR 配错要回退内嵌资源」是同一条原则的两面：
# **既然要保证离线可用，就不能有任何运行时外部依赖。**
#
# 用法：bash scripts/check-no-external-refs.sh [dist 目录] [--falsify]
# 退出码：0=无外链（通过） 1=发现外链（有问题） 2=用法/环境错误（**无法判定**）
#
# 契约（docs/FALSIFY_CONTRACT.md §1.2）：
#   ① 可注入：能对指定目录运行
#   ② 可区分三态：0 / 1 / 2，措辞不同
#   ③ 可被反向驱动：--falsify 时**注入坏输入并跑真实检测**
#      ⚠️ 绝不能写成 `if FALSIFY; then exit 1; fi` —— 实测那样"什么都不检查"的
#         恒绿闸门也能自证通过。falsify 必须真正跑一遍检测，确认它能报出违规。
# ==============================================================================
set -uo pipefail

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; RESET=''
fi

usage() {
  cat >&2 <<'EOF'
用法：check-no-external-refs.sh [dist 目录] [--falsify]
退出码：0=无外链  1=发现外链  2=用法/环境错误（无法判定，不计入通过）
EOF
  exit 2
}

DIST="${1:-ui/web/dist}"
FALSIFY=0
for a in "$@"; do
  [[ "$a" == "--falsify" ]] && FALSIFY=1
done

# --- 匹配规则 -------------------------------------------------------------
# 只在**会发起运行时请求**的 HTML/JS/CSS 里查。
# 白名单：注释、schema.org 元数据（http://schema.org 不是可 fetch 的资源）。
readonly PATTERN='(src|href)[[:space:]]*=[[:space:]]*["'"'"']https?://|@import[[:space:]]+(url\()?["'"'"']?https?://|url\([[:space:]]*["'"'"']?https?://'

# 排除行：注释、schema.org、sourceMappingURL、许可证注释
readonly EXCLUDE='<!--|schema\.org|sourceMappingURL|^\s*//|^\s*\*|licen[cs]e|@license'

# --- 真实检测（0=无 / 1=有）----------------------------------------------
# 抽成函数，供正常模式与 --falsify **共用同一条代码路径**
scan_for_refs() {
  local dir="$1" hits n
  hits=$(grep -rInE "$PATTERN" "$dir" 2>/dev/null | grep -vE "$EXCLUDE" || true)
  [[ -z "$hits" ]] && return 0
  printf '%s\n' "${RED}✗ G29 rule G29-1: 产物中发现运行时外部资源引用${RESET}" >&2
  printf '%s\n' "  ${YELLOW}(违反 AGENTS.md 铁律八：产物必须完全自包含)${RESET}" >&2
  printf '%s\n' "$hits" | head -40 | sed 's/^/    /' >&2
  n=$(printf '%s\n' "$hits" | grep -c . || true)
  if ((n > 40)); then printf '    ... 另有 %d 处\n' "$((n-40))" >&2; fi
  return 1
}

# --- 能力③：--falsify -----------------------------------------------------
if (( FALSIFY )); then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  printf '<script src="https://cdn.jsdelivr.net/npm/antd@5/dist/antd.min.js"></script>\n' > "$tmp/index.html"
  if scan_for_refs "$tmp" >/dev/null 2>&1; then
    # 检测逻辑没报出来 → 闸门**无法自证**（契约 §6.2：这不是通过，是失败）
    printf '%s\n' "${RED}✗ G29 SELF-TEST FAILED: 已知坏输入未被检出 —— 闸门无法自证${RESET}" >&2
    printf '%s\n' "  ${YELLOW}这不是『没有问题』，而是『检测逻辑没生效』。${RESET}" >&2
    exit 2
  fi
  printf '%s\n' "${RED}✗ G29 rule SELF-TEST: 故意注入的外链已被真实检出（falsify 模式，rc=1 为预期）${RESET}" >&2
  exit 1
fi

# --- 环境前置（能力②：无法判定必须 exit 2，绝不能返回 0）-----------------
command -v grep >/dev/null 2>&1 || {
  printf '%s\n' "${YELLOW}▲ G29 无法判定：缺 grep${RESET}" >&2
  printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
  exit 2
}

if [[ ! -d "$DIST" ]]; then
  printf '%s\n' "${YELLOW}▲ G29 无法判定：产物目录不存在（$DIST）${RESET}" >&2
  printf '%s\n' "  先跑前端构建（npm ci && npm run build）" >&2
  printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
  exit 2
fi

if ! scan_for_refs "$DIST"; then
  cat >&2 <<'EOF'

  目标机器是家用 NAS / 内网 VPS，断外网时这些资源会静默失败 → 首屏白屏。

  修复方式：
    · 把资源打进产物（vite 内联 / base64 / 本地打包）
    · 或确认它是纯注释后，从匹配白名单里排除
  确属误报时，请在脚本的 EXCLUDE 里加白名单并说明理由，不要直接删检查。
EOF
  exit 1
fi

files=$(find "$DIST" -type f 2>/dev/null | wc -l | tr -d ' ')
printf '%s\n' "${GREEN}✓ G29 无运行时外链，产物自包含${RESET} ${YELLOW}(已扫描 ${files} 个文件)${RESET}"
exit 0
