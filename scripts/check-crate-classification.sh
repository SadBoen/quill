#!/usr/bin/env bash
# ==============================================================================
# check-crate-classification.sh —— 边界规则 9（三层 crate 分类 + 反向断言）
#
# 主理人裁决：
#   【产品 crate】11 个（quill-bridge 于 2026-10-05 补入），受依赖方向检查
#   【支撑 crate】quill-testkit —— 豁免依赖方向，但禁止被产品 [dependencies] 引用
#   【二进制】    quill-server —— 依赖图上游
#
# 🔴 判据（本脚本存在的理由）：
#   > quill-testkit 出现在任何产品的 [dependencies]（而非 [dev-dependencies]）→ 报错
#
#   为什么"豁免"本身也要被检查：
#     · 只列白名单、其余静默放过 → 支撑 crate 被误当产品依赖时无人察觉（第 1 类假闸门）
#     · 豁免处写"忽略 testkit"  → 同样无人察觉，且更难发现
#     · 豁免 + 反向断言         → 显式、受检查、输出可见
#
# 判据的一句话版本：**每个豁免都必须配一条反向断言，否则豁免等于无人看管的空白。**
#
# 用法：bash scripts/check-crate-classification.sh [crates 目录]   默认 crates
# 退出码：0=合规 / 1=违规 / 2=环境错误
#
# ⚠️ 显式豁免清单（规则类闸门第 3 陷阱）：
#   本脚本自身含 SUPPORT_CRATE 字符串，但它只扫 Cargo.toml，不扫 .rs，
#   且显式排除 xtask —— 见下方 EXCLUDE_SELF。
# ==============================================================================
set -uo pipefail

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; DIM=$'\033[2m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; DIM=''; RESET=''
fi

# 🔴 参数解析：--falsify 会被当成目录名，必须先剔除。
#    否则 `--falsify` 单独调用时 $CRATES="--falsify" → 目录不存在 → 误判无法判定。
FALSIFY=0
CRATES=""
for a in "$@"; do
  if [[ "$a" == "--falsify" ]]; then
    FALSIFY=1
  elif [[ -z "$CRATES" ]]; then
    CRATES="$a"
  fi
done
[[ -z "$CRATES" ]] && CRATES="crates"



# ⚠️ falsify 模式**自带**构造的目录，不需要 $CRATES 存在
if (( ! FALSIFY )) && [[ ! -d "$CRATES" ]]; then
  printf '%s\n' "${YELLOW}▲ RULE-9 无法判定：目录不存在（$CRATES）${RESET}" >&2
  printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
  exit 2
fi

# --- 三层清单（主理人裁决，权威定义在此）----------------------------------
readonly PRODUCTS=(
  quill-adapters quill-domain quill-store quill-control
  quill-wiki quill-agent quill-ext-hub
  quill-backup quill-upgrade quill-xtask
  # 主理人 2026-10-05 裁决：quill-bridge 归**产品**层。
  #   依据（crates/quill-bridge/src/lib.rs:10-15 自身声明）：是库、非二进制、
  #   非测试支撑；✅ 可依赖 quill-adapters / quill-domain，
  #   ❌ 禁止依赖 quill-server（成环）/ quill-agent / quill-control。
  #   上一团队提交 1ddd539 建了 crate 却没同步本清单 → 闸门一直报红
  #   （「未知分类：quill-bridge」）。清单是权威定义，不是可选白名单。
  quill-bridge
)
readonly SUPPORT=(quill-testkit)
# ⚠️ quill-cli 于 2026-10-05 随命令行入口落地时加入本清单。
#   归【二进制】层的依据：它是**面向用户的入口**而非库，与 quill-server 同处依赖图顶端，
#   因此允许依赖全部 quill-*（契约 §一：只有它和 quill-server 可以）。
#   上一轮加了 crate 却没同步本清单 → 闸门一直报红「未知分类：quill-cli」。
#   **清单是权威定义，不是可选白名单**——建新 crate 必须同时改这里，否则闸门立刻变红。
readonly BINARIES=(quill-server quill-cli)
# ⚠️ 显式排除（规则类闸门第 3 陷阱）：xtask 自身会包含这些字符串
readonly EXCLUDE_SELF=("quill-xtask")

is_product() { local n="$1"; for p in "${PRODUCTS[@]}"; do [[ "$n" == "$p" ]] && return 0; done; return 1; }
is_support() { local n="$1"; for s in "${SUPPORT[@]}"; do [[ "$n" == "$s" ]] && return 0; done; return 1; }
is_binary()  { local n="$1"; for b in "${BINARIES[@]}"; do [[ "$n" == "$b" ]] && return 0; done; return 1; }
is_excluded(){ local n="$1"; for e in "${EXCLUDE_SELF[@]}"; do [[ "$n" == "$e" ]] && return 0; done; return 1; }

fail=0

# --- 检查 1：目录必须在三类之内（出现第四类 → 直接红，不静默放过）--------
check_all() {
local CRATES="${1:-crates}"
local fail=0
echo "分类检查（crates/ 下每个目录必须属于产品/支撑/二进制之一）"
while IFS= read -r d; do
  [[ -n "$d" ]] || continue
  n=$(basename "$d")
  if   is_product  "$n"; then printf '  %s产品%s   %s\n'   "$GREEN" "$RESET" "$n"
  elif is_support  "$n"; then printf '  %s支撑%s   %s  （豁免依赖方向，但有反向断言）\n' "$YELLOW" "$RESET" "$n"
  elif is_binary   "$n"; then printf '  %s二进制%s %s\n'   "$GREEN" "$RESET" "$n"
  elif is_excluded "$n"; then printf '  %s工具%s   %s  （xtask 自身，规则豁免）\n' "$DIM" "$RESET" "$n"
  else
    printf '%s\n' "  ${RED}✗ rule 9: 未知分类：$n${RESET}" >&2
    fail=1
  fi
done < <(find "$CRATES" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort)

# --- 检查 2：🔴 反向断言（豁免的核心）-----------------------------------
# 支撑 crate 出现在产品 crate 的 [dependencies] → 红
# 出现在 [dev-dependencies] → 绿（正确用法）
echo
echo "反向断言：支撑 crate 不得进产品 [dependencies]"
for sup in "${SUPPORT[@]}"; do
  for d in "$CRATES"/*/; do
    [[ -d "$d" ]] || continue
    prod=$(basename "$d")
    is_product "$prod" || continue
    is_excluded "$prod" && continue
    mf="$d/Cargo.toml"
    [[ -f "$mf" ]] || continue

    # 提取 [dependencies] 段（到下一个 [ 段为止）
    dep_block=$(awk '
      /^\[[^]]+\][[:space:]]*$/ { seg=$0; inblk=(seg ~ /^\[dependencies\]/); next }
      inblk { print }
    ' "$mf")

    if printf '%s' "$dep_block" | grep -qE "^[[:space:]]*${sup}[[:space:]]*="; then
      printf '%s\n' "  ${RED}✗ rule 9: $prod 的 [dependencies] 含 $sup${RESET}" >&2
      printf '%s\n' "    $mf" >&2
      printf '%s\n' "    ${YELLOW}支撑 crate 只能出现在 [dev-dependencies]${RESET}" >&2
      fail=1
    elif grep -qE "^[[:space:]]*${sup}[[:space:]]*=" "$mf"; then
      printf '  %s✓%s %s 仅在 [dev-dependencies] 引用 %s\n' "$GREEN" "$RESET" "$prod" "$sup"
    fi
  done
done

# --- 检查 3：支撑 crate 必须声明 publish = false -------------------------
echo
echo "支撑 crate 自身须声明 publish = false（不进 release 依赖树）"
for sup in "${SUPPORT[@]}"; do
  mf="$CRATES/$sup/Cargo.toml"
  if [[ ! -f "$mf" ]]; then
    printf '  %s—%s %s/Cargo.toml 不存在（尚未建）\n' "$YELLOW" "$RESET" "$sup"
  elif grep -qE '^[[:space:]]*publish[[:space:]]*=[[:space:]]*false' "$mf"; then
    printf '  %s✓%s %s publish = false\n' "$GREEN" "$RESET" "$sup"
  else
    printf '%s\n' "  ${RED}✗ rule 9: $sup 缺 publish = false${RESET}" >&2
    printf '%s\n' "    它是测试设施，不应进 release 产物" >&2
    fail=1
  fi
done

if ((fail)); then
  cat >&2 <<'EOF'

  为什么这条不能靠"白名单只列产品 crate"顺带解决：
    支撑 crate 若被误当产品依赖（写进 [dependencies]），
    测试代码会进 release 二进制 —— 体积膨胀，且测试专用依赖可能引入
    额外的传递依赖与攻击面。

  修复：把该依赖移到 [dev-dependencies]。
EOF
  return 1
fi
return 0
}

# --- 正常模式：跑真实检查 ------------------------------------------------

# --- 能力③：--falsify（必须在所有函数与数组定义之后）-------------------------
# ⚠️ 位置要求：本块用到 check_all() 与 readonly PRODUCTS/SUPPORT 数组，
#    必须放在它们**之后**，否则 set -u 下会因未定义变量而静默失效。
#    那样"什么都不检查"的恒绿闸门也能自证通过。
#    这里是：造一个**支撑 crate 出现在产品 [dependencies]** 的真实样本，
#    跑同一套判定逻辑，确认它真的报红。
if (( FALSIFY )); then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/crates/quill-wiki" "$tmp/crates/quill-testkit"
  cat > "$tmp/crates/quill-testkit/Cargo.toml" <<'EOF'
[package]
name = "quill-testkit"
publish = false
EOF
  # 违规：产品 crate 把支撑 crate 写进了 [dependencies]
  cat > "$tmp/crates/quill-wiki/Cargo.toml" <<'EOF'
[package]
name = "quill-wiki"
[dependencies]
quill-testkit = { path = "../quill-testkit" }
EOF
  # ⚠️ 传 "$tmp/crates" 而不是靠 $1 —— falsify 模式下 $1 是 "--falsify"
  # 🔴 判据必须是 **check_all 的返回值**，不是输出里有没有 "rule 9" 字样。
  #   理由：若检测逻辑被破坏（如把 return 1 改成 return 0），
  #   它仍会**打印** "✗ rule 9: ..." 再返回 0 —— 靠 grep 字样会误判为"已检出"。
  #   **返回值才是检测逻辑的真实结论**（契约 §1.2 能力④）。
  _dbg=$(mktemp)
  check_all "$tmp/crates" >"$_dbg" 2>&1
  _rc=$?
  if (( _rc == 1 )) && grep -q "rule 9" "$_dbg"; then
    rm -f "$_dbg"
    printf '%s\n' "${RED}✗ RULE-9 rule SELF-TEST: 故意注入的依赖违规已被真实检出（falsify 模式，rc=1 为预期）${RESET}" >&2
    exit 1
  fi
  printf '%s\n' "${RED}✗ RULE-9 SELF-TEST FAILED: 已知坏输入未被检出 —— 闸门无法自证（check_all rc=${_rc}，期望 1）${RESET}" >&2
  printf '%s\n' "$_dbg" | sed 's/^/    /' >&2
  printf '%s\n' "  ${YELLOW}这不是『没有问题』，而是『检测逻辑没生效』。${RESET}" >&2
  rm -f "$_dbg"
  exit 2
fi
check_all "$CRATES" || exit 1
printf '\n%s\n' "${GREEN}═══ 规则 9 通过：三层分类合规，豁免均有反向断言 ═══${RESET}"
exit 0
