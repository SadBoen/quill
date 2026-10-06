#!/usr/bin/env bash
# 一条命令验证项目是活的。MILESTONES.md 的 M0 判据就是这个脚本退出 0。
#
# **为什么要有它**：以前「全项目 0 failed」这道纪律只存在于约定里，
# 而实际把关的那道门禁解析写错了 —— awk 按 '[ ;]' 分隔，失败数其实在 $7，
# 于是 failed 恒等于 0，跑多少个红它都印「通过」（见 gate-selftest.sh 的自测）。
# 纪律写下来了但从没真正把过关。现在它是一条能跑、能看见、退出码可信的命令。
#
# 用法：
#   bash .scripts/gates.sh              # 全部
#   bash .scripts/gates.sh --fast       # 跳过前端 build（最慢的一步）
#   bash .scripts/gates.sh --text-only  # 只跑四道文本门禁，不编译
#
# 注意：解析 cargo 输出的那段与 gate-selftest.sh 逐字一致。
# 改这里就必须同步改那里，并跑 `bash .scripts/gate-selftest.sh` 确认四种场景仍判对。
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

FAST=0
TEXT_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --fast) FAST=1 ;;
    --text-only) TEXT_ONLY=1 ;;
    *) echo "未知参数：$arg（可用：--fast / --text-only）"; exit 2 ;;
  esac
done

if ! command -v cargo >/dev/null 2>&1; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi

FAILED=0
step() { printf '\n=== %s ===\n' "$1"; }
fail() { echo "  ✗ $1"; FAILED=1; }

# ——— 工具链预检 ———
# **为什么单独做这一步**：缺工具时报「门禁未过」，会让人去修门禁代码，
# 而真正的原因是环境里没有那个工具。这与当年 awk 解析恒为 0 是同一类错误 ——
# 查不出来源的失败会被当成结论。
# 已知现状（2026-10-06）：WSL2 里有 cargo 但**没有 node**；
# Windows 侧有 node/npm 但没有 cargo 工具链。所以本脚本在 WSL 上只能跑
# Rust 那部分，前端那部分要在 Windows 侧跑（见 BACKLOG.md 的 B0-2）。
step "工具链"
HAVE_CARGO=0; HAVE_NODE=0
if command -v cargo >/dev/null 2>&1; then HAVE_CARGO=1; echo "  ✓ cargo $(cargo --version 2>/dev/null | head -1)"; else echo "  - cargo 缺失"; fi
if command -v node  >/dev/null 2>&1; then HAVE_NODE=1;  echo "  ✓ node $(node --version)"; else echo "  - node 缺失（文本门禁与前端步骤无法在本机跑）"; fi

need_text_gate=1
if [ "$HAVE_NODE" = "0" ]; then
  echo "  → 文本门禁跳过：没有 node。这一段请在有 node 的一侧单独跑："
  echo "     node .mojibake-check.mjs / .i18n-check.mjs / .library-check.mjs / .upstream-check.mjs"
  need_text_gate=0
fi

# ——— 与 .scripts/gate-selftest.sh 里那段判定逐字一致 ———
summarize_cargo() {
  local LOG="$1"
  grep -E '^test result' "$LOG" \
    | awk -F'[ ;]' '{for(i=1;i<NF;i++){if($(i+1)=="passed")p+=$i; if($(i+1)=="failed")f+=$i}} END {print p+0, f+0}'
}

run_cargo_tests() {
  local LABEL="$1"; shift
  local LOG; LOG="$(mktemp)"
  echo "  跑 $LABEL ..."
  # 退出码要立刻取。原来写成 `if ! "$@"; then local RC=$?` —— 进了 then 分支时
  # `$?` 已经是 `!` 自己的退出码，**恒为 0**，于是「编译失败」会被报成「退出码 0」。
  # 这与当年 awk 门禁恒为 0 是同一类错：把「跑失败」报成了「跑成功」。
  "$@" >"$LOG" 2>&1
  local RC=$?
  local COUNTS; COUNTS="$(summarize_cargo "$LOG")"
  local LINES; LINES="$(grep -cE '^test result' "$LOG" || true)"

  if [ "$LINES" = "0" ]; then
    # 一行 `test result` 都没有 = 编译失败或提前退出。此时 passed/failed 解析出来
    # 必然是 0 0，直接拿它当「通过」就是第二次骗人。
    fail "$LABEL 没有产出任何 'test result' 行（编译失败或中途退出，退出码 $RC）"
    cp "$LOG" /tmp/quill-gate-fail.log
    grep -E '^error' "$LOG" | head -8 | sed 's/^/    /'
  elif [ "$RC" -ne 0 ] || [ "${COUNTS#* }" != "0" ]; then
    fail "$LABEL 未全过（退出码 $RC，passed/failed = $COUNTS）"
    cp "$LOG" /tmp/quill-gate-fail.log
    grep -E '^test .* FAILED|^error' "$LOG" | head -15 | sed 's/^/    /'
  else
    echo "  ✓ $LABEL（passed/failed = $COUNTS）"
  fi
  rm -f "$LOG"
}

# ——— 文本门禁（无需编译，Windows 原生 shell 也能跑）———
run_text_gate() {
  step "文本门禁 · $1"
  if node "$1" >/tmp/quill-gate-node.log 2>&1; then
    tail -2 /tmp/quill-gate-node.log | sed 's/^/  ✓ /'
  else
    fail "$1 未过"
    tail -6 /tmp/quill-gate-node.log | sed 's/^/    /'
  fi
}

if [ "$need_text_gate" = "1" ]; then
  run_text_gate .mojibake-check.mjs
  run_text_gate .i18n-check.mjs
  run_text_gate .library-check.mjs
  run_text_gate .upstream-check.mjs
fi

if [ "$TEXT_ONLY" = "1" ]; then
  step "结果"
  if [ "$FAILED" = "0" ]; then
    [ "$need_text_gate" = "1" ] && echo "  文本门禁全过（已跳过编译与测试，--text-only）" \
                               || echo "  本机没有 node，文本门禁未跑（--text-only）"
  else
    echo "  有门禁没过"
  fi
  exit "$FAILED"
fi

# Rust 侧缺 cargo 时同样要说清是环境问题，不是门禁不通过。
if [ "$HAVE_CARGO" = "0" ]; then
  step "结果"
  echo "  本机没有 cargo，Rust 那几步未跑。在有 cargo 的一侧执行本脚本即可。"
  exit 1
fi

# ——— 门禁自己的门禁 ———
step "门禁自测"
if bash .scripts/gate-selftest.sh >/tmp/quill-gate-self.log 2>&1; then
  echo "  ✓ 判定逻辑的四种场景全对"
else
  fail "门禁自测没过 —— 门禁本身不可信，先修它"
  tail -8 /tmp/quill-gate-self.log | sed 's/^/    /'
fi

step "构建 · quill-server"
if cargo build -p quill-server >/tmp/quill-gate-build.log 2>&1; then
  echo "  ✓ cargo build -p quill-server"
else
  fail "cargo build 未过"
  grep -E '^error' /tmp/quill-gate-build.log | head -10 | sed 's/^/    /'
fi

run_cargo_tests "Rust 单测"      cargo test -p quill-server --lib
run_cargo_tests "扩展集成测试"  cargo test -p quill-server --test extensions_http
run_cargo_tests "会话指标集成"  cargo test -p quill-server --test session_metrics_http

step "前端"
if [ "$HAVE_NODE" = "0" ] || [ ! -d ui/web ]; then
  echo "  - 跳过：本机没有 node。请在有 node 的一侧跑 npm run typecheck / npx vitest run / npm run build"
else
  cd ui/web || exit 1
  if npm run typecheck >/tmp/quill-gate-tsc.log 2>&1; then
    echo "  ✓ typecheck"
  else
    fail "tsc 未过"; tail -6 /tmp/quill-gate-tsc.log | sed 's/^/    /'
  fi
  if npx vitest run >/tmp/quill-gate-vitest.log 2>&1; then
    grep -E 'Test Files|Tests ' /tmp/quill-gate-vitest.log | sed 's/^/  ✓ /'
  else
    fail "vitest 未过"; grep -E 'FAIL|Tests ' /tmp/quill-gate-vitest.log | head -10 | sed 's/^/    /'
  fi
  if [ "$FAST" = "1" ]; then
    echo "  - build 已跳过（--fast）"
  elif npm run build >/tmp/quill-gate-vite.log 2>&1; then
    echo "  ✓ build"
  else
    fail "vite build 未过"; tail -8 /tmp/quill-gate-vite.log | sed 's/^/    /'
  fi
fi

step "结果"
SKIPPED=0
[ "$HAVE_NODE" = "0" ] && SKIPPED=1
if [ "$FAILED" = "0" ]; then
  if [ "$SKIPPED" = "1" ]; then
    echo "  跑到的部分全过；但本机缺 node，文本门禁与前端那几步**没有跑**（不算全绿）"
    exit 1
  fi
  echo "  门禁全过"
  exit 0
fi
echo "  有门禁没过 —— MILESTONES.md 的判据不成立，先修"
exit 1