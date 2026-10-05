#!/usr/bin/env bash
# ==============================================================================
# check-crate-deps.sh —— G40：crate 依赖方向契约闸门
#
# 🔴 本脚本存在的理由（覆盖缺口的补上）：
#   docs/PHASE2_CONTRACT.md §一 规定了 crate 依赖方向的**硬约束**，此前
#   **没有任何闸门在执行它们**。原本该负责的 `cargo xtask boundary`
#   从未存在（实测 `cargo xtask` → `error: no such command: xtask`），
#   `crates/quill-xtask` 是空壳。scripts/check-boundary-singletons.sh（G26）
#   只管全局单例 / include! 逃逸 / 软链 / build.rs，**管不到依赖图**。
#   于是 crates/quill-adapters/src/lib.rs 里"xtask boundary 会拦"的假声明
#   可以在无人复核的情况下存活。
#
# 检查项（依赖图 + 可见性收敛）：
#   G40-a  quill-adapters 不得依赖任何其他 quill-*；且任何 crate 不得依赖自己
#   G40-b  quill-wiki 不得依赖 quill-agent
#   G40-c  任何 quill-* 不得依赖 vendor/goose（含 path 与 crates.io 两种写法）
#   G40-d  quill-server 之外不得出现 unreachable_pub
#   G40-e  反向/健康断言：列出**实际依赖边**，报告"检查了几条边"
#
# 契约允许（**故意不拦**，写反了就是误报闸门）：
#   · quill-agent 依赖 quill-wiki —— PHASE2 契约明确允许（经 trait 间接访问）
#   · [dev-dependencies] 里的 quill-* —— dev 依赖不进发布产物，不成环
#
# 用法：bash scripts/check-crate-deps.sh [crates 目录] [--falsify]  默认 crates
# 退出码：0=通过 / 1=检出违规 / 2=无法判定（措辞含 ▲，绝不计入通过）
#
# 铁律：禁止 `|| true` / `|| echo 跳过` 兜底。缺工具 = rc=2。
# ==============================================================================
set -uo pipefail

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; DIM=$'\033[2m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; DIM=''; RESET=''
fi

# --- 参数解析：--falsify 可能出现在任意位置 --------------------------------
# 🔴 run-gate-selftest.sh 调用形式是 `bash <script> <TARGET_DIR> --falsify`，
#    即 **--falsify 是第二个参数**。因此必须遍历全部参数，不能只看 $1。
FALSIFY=0
CRATES=""
for a in "$@"; do
  case "$a" in
    --falsify) FALSIFY=1 ;;
    *) [[ -z "$CRATES" ]] && CRATES="$a" ;;
  esac
done
[[ -z "$CRATES" ]] && CRATES="crates"

# --- 工具能力检查：缺工具必须判 rc=2，不许兜底 ------------------------------
MISSING=()
for t in awk grep find sort mktemp basename; do
  command -v "$t" >/dev/null 2>&1 || MISSING+=("$t")
done
if (( ${#MISSING[@]} )); then
  printf '%s\n' "${YELLOW}▲ G40 无法判定：缺少必需工具：${MISSING[*]}${RESET}" >&2
  printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过（不用 || true 兜底）。" >&2
  exit 2
fi

if (( ! FALSIFY )) && [[ ! -d "$CRATES" ]]; then
  printf '%s\n' "${YELLOW}▲ G40 无法判定：crates 目录不存在（$CRATES）${RESET}" >&2
  printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
  exit 2
fi

# ==============================================================================
# 依赖提取：awk 段切分
# ==============================================================================
# 设计要点（为什么不会误匹配 [dev-dependencies]）：
#   1. 段名先**剥掉行尾空白与注释**，再 `gsub(/[ \t]/,"")` 去掉全部空白，
#      最后用**严格相等** `seg == "[dependencies]"` 判定。
#      严格相等意味着 `[dev-dependencies]`、`[build-dependencies]`、
#      `[target.'cfg(unix)'.dependencies]` 全部**不匹配**。
#      ⚠️ 绝不能用前缀正则 `seg ~ /^\[dependencies\]/` 之外的宽松写法，
#      也不能用 `/dependencies/` —— 那会把 dev 依赖算进来（成环误报）。
#   2. 段名行必须**独占一行**且以 `[` 开头（`/^[ \t]*\[/`），
#      因此不会把 `features = ["dependencies"]` 这类行尾字面量当成段切换。
#   3. 依赖名提取：去空白后必须整行匹配 `^[A-Za-z0-9_-]+[ \t]*=`。
#      · 整行注释（`# axum = "0.8"`）以 `#` 开头，**不匹配** → 自动排除
#      · 多行内联表的续行（`"runtime-tokio",` / `]`）不匹配 → 自动排除
#      · 去掉 `quill-` vs `quill_` 的写法差异由 norm() 归一
#   4. 行尾注释只剥 `[ \t]+#`（要求 # 前有空白），保留 `path = "../a#b"`。
extract_deps() {
  awk '
    /^[ \t]*\[/ {
      seg = $0
      sub(/[ \t]*#.*$/, "", seg)
      gsub(/[ \t]/, "", seg)
      inblk = (seg == "[dependencies]")
      next
    }
    inblk {
      line = $0
      sub(/[ \t]+#.*$/, "", line)
      gsub(/^[ \t]+|[ \t]+$/, "", line)
      if (line == "") next
      if (line ~ /^[A-Za-z0-9_-]+[ \t]*=/) {
        n = line
        sub(/[ \t]*=.*$/, "", n)
        print n "\t" line
      }
      next
    }
  ' "$1"
}

# 顶层内联依赖表 `dependencies = { ... }`（不在 [dependencies] 段下）
# → 本 awk 解析器**解析不了** → 报无法判定，绝不静默放过（否则是恒绿）。
has_inline_deps() {
  grep -qE '^[ \t]*dependencies[ \t]*=[ \t]*\{' "$1"
}

# Rust 注释剥离：注释里的 unreachable_pub 不是代码，必须放行（防误报）。
strip_rust_comments() {
  awk '
    BEGIN { inb = 0 }
    {
      line = $0
      if (inb) {
        p = index(line, "*/")
        if (p == 0) next
        line = substr(line, p + 2)
        inb = 0
      }
      while ((p = index(line, "/*")) > 0) {
        pre  = substr(line, 1, p - 1)
        rest = substr(line, p + 2)
        q = index(rest, "*/")
        if (q == 0) { line = pre; inb = 1; break }
        line = pre " " substr(rest, q + 2)
      }
      if (!inb) {
        sub(/\/\/.*$/, "", line)
        print line
      }
    }
  ' "$1"
}

# Cargo 允许 `quill-store` 与 `quill_store` 两种 key 写法 → 归一后再比较
norm() { printf '%s' "$1" | tr '-' '_'; }

# ==============================================================================
# check_all —— 正常模式与 --falsify 模式**共用同一份实现**（绝不复制第二份）
#   契约 §1.2：判据必须取返回值，输出文本只作佐证。
# ==============================================================================
check_all() {
  local CRATES="${1:-crates}"
  local fail=0 undet=0
  local edges=0 qedges=0 crates_n=0 manifests=0 no_manifest=0
  local rs_scanned=0 rs_excluded=0 inline_found=0

  printf 'G40 crate 依赖方向检查（%s）\n' "$CRATES"
  printf '%s\n' "—— 契约依据 docs/PHASE2_CONTRACT.md §一"

  local dirs=()
  while IFS= read -r d; do
    [[ -n "$d" ]] && dirs+=("$d")
  done < <(find "$CRATES" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort)

  if (( ${#dirs[@]} == 0 )); then
    printf '%s\n' "${YELLOW}▲ G40 无法判定：$CRATES 下没有任何 crate 目录 —— 恒绿嫌疑，检查器什么也没看${RESET}" >&2
    return 2
  fi
  crates_n=${#dirs[@]}
  printf '已发现 %d 个 crate 目录\n\n' "$crates_n"

  local d name mf dep dname dline dnorm
  for d in "${dirs[@]}"; do
    name=$(basename "$d")
    mf="$d/Cargo.toml"
    if [[ ! -f "$mf" ]]; then
      no_manifest=$((no_manifest + 1))
      printf '  %s—%s %s（无 Cargo.toml，未检查其依赖）\n' "$DIM" "$RESET" "$name"
      continue
    fi
    manifests=$((manifests + 1))

    if has_inline_deps "$mf"; then
      inline_found=$((inline_found + 1))
      printf '%s\n' "  ${YELLOW}▲ G40 无法判定：$name 使用顶层内联依赖表 \`dependencies = { ... }\`，本 awk 解析器不覆盖该写法${RESET}" >&2
      printf '%s\n' "    $mf（改用 [dependencies] 段写法，或扩展本解析器后再判）" >&2
      undet=1
      continue
    fi

    printf '  %s→%s %s\n' "$DIM" "$RESET" "$name"

    while IFS=$'\t' read -r dname dline; do
      [[ -n "$dname" ]] || continue
      edges=$((edges + 1))
      dnorm=$(norm "$dname")

      if [[ "$dnorm" == quill_* ]]; then
        qedges=$((qedges + 1))
        printf '      %s✓%s %s → %s\n' "$GREEN" "$RESET" "$name" "$dname"
      else
        printf '      %s·%s %s → %s\n' "$DIM" "$RESET" "$name" "$dname"
      fi

      # --- G40-a：自环 ---------------------------------------------------
      if [[ "$dnorm" == "$(norm "$name")" ]]; then
        printf '%s\n' "    ${RED}✗ rule G40-a: 自环 —— $name 依赖自己${RESET}" >&2
        printf '%s\n' "      $mf: $dline" >&2
        fail=1
        continue
      fi

      # --- G40-a：quill-adapters 不得依赖任何 quill-* ---------------------
      if [[ "$name" == "quill-adapters" && "$dnorm" == quill_* ]]; then
        printf '%s\n' "    ${RED}✗ rule G40-a: quill-adapters 依赖了其他 quill-*（$dname）${RESET}" >&2
        printf '%s\n' "      $mf: $dline" >&2
        printf '%s\n' "      ${YELLOW}adapters 是契约层，反向依赖会形成环${RESET}" >&2
        fail=1
      fi

      # --- G40-b：quill-wiki 不得依赖 quill-agent -------------------------
      if [[ "$name" == "quill-wiki" && "$dnorm" == "quill_agent" ]]; then
        printf '%s\n' "    ${RED}✗ rule G40-b: quill-wiki 依赖 quill-agent${RESET}" >&2
        printf '%s\n' "      $mf: $dline" >&2
        printf '%s\n' "      ${YELLOW}wiki 驱动 agent，反过来会成环${RESET}" >&2
        fail=1
      fi

      # --- G40-c：不得依赖 vendor/goose（含 path 与 crates.io 两种写法）----
      if [[ "$dnorm" == goose || "$dnorm" == goose_* ]] \
         || printf '%s' "$dline" | grep -qE 'vendor[/\\]goose'; then
        printf '%s\n' "    ${RED}✗ rule G40-c: $name 依赖 goose（$dname）${RESET}" >&2
        printf '%s\n' "      $mf: $dline" >&2
        printf '%s\n' "      ${YELLOW}只能经 quill-adapters 间接使用 goose（守 D1 边界）${RESET}" >&2
        fail=1
      fi
    done < <(extract_deps "$mf")

    # --- G40-d：quill-server 之外不得出现 unreachable_pub -----------------
    if [[ "$name" == "quill-server" ]]; then
      printf '      %s·%s unreachable_pub：豁免（quill-server 是唯一允许的可见性出口）\n' "$DIM" "$RESET"
    else
      local f
      while IFS= read -r f; do
        [[ -n "$f" ]] || continue
        rs_scanned=$((rs_scanned + 1))
        if strip_rust_comments "$f" | grep -q 'unreachable_pub'; then
          printf '%s\n' "    ${RED}✗ rule G40-d: $name 出现 unreachable_pub（非 quill-server）${RESET}" >&2
          printf '%s\n' "      $f: $(grep -n 'unreachable_pub' "$f" | head -1)" >&2
          printf '%s\n' "      ${YELLOW}可见性须收敛；只有 quill-server 允许暴露 pub${RESET}" >&2
          fail=1
        fi
      done < <(find "$d" -type f -name '*.rs' -not -path '*/target/*' 2>/dev/null)
      # fixture 目录显式豁免（故意违规的边界样本，不是本仓库源码），**计数输出**——
      # 豁免若不计数就会变成无人知道何时变宽的漏报区。
      local fx
      while IFS= read -r fx; do
        [[ -n "$fx" ]] && rs_excluded=$((rs_excluded + 1))
      done < <(find "$d" -type f -name '*.rs' -path '*/fixtures/*' -not -path '*/target/*' 2>/dev/null)
    fi
  done

  # --- G40-e：反向/健康断言 —— "0 与未检查必须可区分" -----------------------
  printf '\n%s\n' "G40-e 反向断言：本次实际检查的依赖边"
  printf '  已检查依赖边总数：%d 条（其中 quill-* 内部边 %d 条）\n' "$edges" "$qedges"
  printf '  已检查 crate 目录：%d 个（其中含 Cargo.toml %d 个、无 Cargo.toml %d 个）\n' \
         "$crates_n" "$manifests" "$no_manifest"
  printf '  已检查 .rs 文件：%d 个（unreachable_pub 扫描；quill-server 按契约豁免不计）\n' "$rs_scanned"
  if (( rs_excluded )); then
    printf '  %s⚠%s 已豁免 fixtures/ 下 .rs 文件：%d 个（故意违规的边界样本，非本仓库源码）\n' \
           "$YELLOW" "$RESET" "$rs_excluded"
  fi
  printf '  %s\n' "  ⚠️ 「0 条边」与「一条边都没检查」在屏幕上必须能区分：以上为实际计数，非占位。"

  # --- 三态收敛 ----------------------------------------------------------
  if (( edges == 0 )); then
    printf '%s\n' "${YELLOW}▲ G40 无法判定：一条依赖边都没解析到 —— 恒绿嫌疑（检查器可能已失效）${RESET}" >&2
    printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
    undet=1
  fi
  if (( rs_scanned == 0 )); then
    printf '%s\n' "${YELLOW}▲ G40 无法判定：一个 .rs 文件都没扫描到 —— unreachable_pub 检查形同虚设${RESET}" >&2
    undet=1
  fi

  if (( fail )); then
    printf '\n%s\n' "${RED}═══ G40 判红：crate 依赖方向 / 可见性违规 ═══${RESET}" >&2
    return 1
  fi
  if (( undet )); then
    printf '\n%s\n' "${YELLOW}═══ G40 存在无法判定项：不计入通过 ═══${RESET}" >&2
    return 2
  fi
  return 0
}

# ==============================================================================
# --falsify（在所有函数定义之后 —— 否则 set -u 下静默失效）
#   造临时仓库（含违规 crates 树），走**同一个 check_all()**。
#   判据 = 返回值 rc==1 **且**输出含 `rule G40-a`（绑定探针实际命中的那条规则，
#   不是"对全部规则断言" —— 否则真实闸门会因其它规则无探针而被误判为坏）。
# ==============================================================================
if (( FALSIFY )); then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  # ⚠️ 探针在 mktemp -d 内，**绝不在项目仓库内**，也绝不对仓库 git add。
  mkdir -p "$tmp/crates/quill-adapters/src" "$tmp/crates/quill-store/src"
  cat > "$tmp/crates/quill-store/Cargo.toml" <<'EOF'
[package]
name = "quill-store"
version = "0.1.0"
EOF
  : > "$tmp/crates/quill-store/src/lib.rs"
  # 探针：quill-adapters 的 [dependencies] 里出现 quill-store → 违反 G40-a
  cat > "$tmp/crates/quill-adapters/Cargo.toml" <<'EOF'
[package]
name = "quill-adapters"
version = "0.1.0"

[dependencies]
quill-store = { path = "../quill-store" }
EOF
  : > "$tmp/crates/quill-adapters/src/lib.rs"

  _dbg=$(mktemp)
  check_all "$tmp/crates" >"$_dbg" 2>&1
  _rc=$?
  if (( _rc == 1 )) && grep -q 'rule G40-a' "$_dbg"; then
    # ⚠️ 必须把**检出的规则 ID 原样打出来**：run-gate-selftest.sh 用
    #    `grep -qE 'rule[[:space:]]+[A-Za-z0-9]'` 判定"失败信息是否精确"，
    #    只打印 SELF-TEST OK 而不带 rule ID 会被 runner 判为「失败信息不精确」而拒收。
    printf '%s\n' "${RED}✗ G40 SELF-TEST OK: 注入的 quill-adapters→quill-store 已被检出（falsify 模式，rc=1 为预期）${RESET}" >&2
    # ⚠️ 这里必须**直接读文件**：`$_dbg` 是 mktemp 返回的**路径**而非内容，
    #    写成 `printf '%s\n' "$_dbg" | grep ...` 会把路径本身喂给 grep → 恒空。
    #    （该 bug 曾真实存在过：runner 报「失败信息不精确：无 rule ID」才发现。）
    grep 'rule G40' "$_dbg" | sed 's/^/      检出：/' >&2
    rm -f "$_dbg"
    exit 1
  fi
  printf '%s\n' "${RED}✗ G40 SELF-TEST FAILED: 已知坏输入未被检出 —— 检测逻辑已失效（check_all rc=${_rc}，期望 1 且输出含 rule G40-a）${RESET}" >&2
  sed 's/^/    /' "$_dbg" >&2
  printf '%s\n' "  ${YELLOW}这不是『没有问题』，而是『检测逻辑没生效』。${RESET}" >&2
  rm -f "$_dbg"
  exit 2
fi

check_all "$CRATES"
rc=$?
case $rc in
  0) printf '\n%s\n' "${GREEN}═══ G40 通过：crate 依赖方向合规（adapters 反向依赖 / wiki→agent / goose 直连 / unreachable_pub）═══${RESET}" ;;
  1) exit 1 ;;
  2) exit 2 ;;
  *) printf '%s\n' "${YELLOW}▲ G40 无法判定：check_all 返回了契约外的退出码 $rc${RESET}" >&2; exit 2 ;;
esac
