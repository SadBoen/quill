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

# 同理给 node。**必须在这里做而不是只写 ~/.bashrc**：
# `bash .scripts/gates.sh` 是非交互、非登录的 shell，既不读 .bashrc 也不读
# .profile —— 只写启动文件的话，PATH 在真正跑门禁时仍然是空的，
# 门禁会报「缺 node」，而 node 其实就装在那儿。
if ! command -v node >/dev/null 2>&1; then
  if [ -x "$HOME/.local/node/bin/node" ]; then
    export PATH="$HOME/.local/node/bin:$PATH"
    echo "  （node 从 ~/.local/node/bin 找到，已挂进本次 PATH）"
  fi
fi

FAILED=0
SKIPPED=0
# 哪些文本门禁需要它，由那几道门禁自己判断；这里只给出位置。
UPSTREAM_REF_DIR=".octop-ref/octop"
step() { printf '\n=== %s ===\n' "$1"; }
fail() { echo "  ✗ $1"; FAILED=1; }

# 某一步因为**环境不具备**而没跑成。它不算「通过」，但也不该被算成
# 「门禁没过」—— 那两件事都会误导：前者会让人以为验过了，后者会让人
# 去修根本没坏的代码。真正的处理是：记下来，最后以「不算全绿」退出。
skip() { echo "  - 未跑：$1"; SKIPPED=1; }

# ——— 工具链预检 ———
# **为什么单独做这一步**：缺工具时报「门禁未过」，会让人去修门禁代码，
# 而真正的原因是环境里没有那个工具。这与当年 awk 解析恒为 0 是同一类错误 ——
# 查不出来源的失败会被当成结论。
#
# 环境现状（2026-10-06 更新）：WSL2 里已经装好免 root 的 node（LTS，
# 解压在 `~/.local/node`，官方 SHASUMS 校验过），所以**两侧各自都能跑完
# 全部门禁**，M0 的 B0-2 环境项就此关闭。Windows 侧仍要单独跑，因为
# cargo 工具链不在那边。
step "工具链"
HAVE_CARGO=0; HAVE_NODE=0
if command -v cargo >/dev/null 2>&1; then HAVE_CARGO=1; echo "  ✓ cargo $(cargo --version 2>/dev/null | head -1)"; else echo "  - cargo 缺失"; fi
if command -v node  >/dev/null 2>&1; then HAVE_NODE=1;  echo "  ✓ node $(node --version)"; else echo "  - node 缺失（文本门禁与前端步骤无法在本机跑）"; fi

need_text_gate=1
if [ "$HAVE_NODE" = "0" ]; then
  echo "  → 文本门禁跳过：没有 node。这一段请在有 node 的一侧单独跑："
  echo "     node .mojibake-check.mjs / .i18n-check.mjs / .library-check.mjs / .upstream-check.mjs"
  echo "     node .provenance-check.mjs / .scripts/upstream-check-selftest.mjs / .scripts/provenance-selftest.mjs"
  echo "     node .scripts/upstream-check-selftest.mjs"
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

# 某道文本门禁依赖**私有上游参考源码**（.octop-ref/octop）。它不入库，
# fresh clone 上必然没有（见 .scripts/fetch-vendor.sh 与 UPSTREAM.md）。
#
# 这里必须区分两种情况，不能笼统地判失败：
#   · 没有上游 → 这一步**没跑**，如实记 SKIPPED，最后退出码非 0 并说明
#     「这不是全绿」。CI 与别人的 fresh clone 上就是这样。
#   · 有上游却判不过 → 真的腐烂了，判失败。
# 混为一谈的话，要么让所有 CI 永久红，要么让人以为出处核过了。
run_upstream_gate() {
  step "文本门禁 · $1"
  if [ ! -d "$UPSTREAM_REF_DIR" ]; then
    skip "$1 需要上游参考源码（.octop-ref/octop），本机没有。取回：bash .scripts/fetch-vendor.sh"
    return 0
  fi
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
  run_upstream_gate .library-check.mjs
  # 上游基线核对：比对 UPSTREAM.md 记的 pin 与本机实际检出。
  # 它核的是「本地检出对不对」，所以同样要真有那份检出才核得了。
  run_upstream_gate .upstream-check.mjs
  # 上游判定的自测：合成输入，不联网。跟 gate-selftest.sh 同一个道理 ——
  # 「落后上游」那道判定以前永远判通过，只有钉住才抓得住。
  run_text_gate .scripts/upstream-check-selftest.mjs
  # 出处校验：把 UPSTREAM-USAGE.md 里每条上游引用真的打开核一遍。
  # 它抓的那类腐烂已经真实发生过一次 —— 只写「文件名 + 行号」，octop 有两个
  # 同名文件，读者核不到会以为记录是假的。
  run_upstream_gate .provenance-check.mjs
  run_text_gate .scripts/provenance-selftest.mjs
fi

# ——— 项目状态的计算器：它自己也必须先被验证 ———
#
# `scripts/status.mjs` 取代了文档里手工维护的状态标记（见 docs/adr/0004）。
# 一个从不出错的判定逻辑是**最危险**的 —— 它会输出一张看起来很权威的表，
# 而那张表与现实无关。所以它带着自己的自测，且自测必须进门禁：
# 判定逻辑哪天写坏了，这里当场红。
if [ "$need_text_gate" = "1" ]; then
  step "项目状态判定逻辑的自测"
  if node scripts/status-selftest.mjs >/tmp/quill-gate-status-self.log 2>&1; then
    tail -1 /tmp/quill-gate-status-self.log | sed 's/^/  ✓ /'
  else
    fail "status 判定逻辑不可信 —— 它要取代手工标记，自测不过就不能用"
    grep -E '✗|项判错' /tmp/quill-gate-status-self.log | head -8 | sed 's/^/    /'
  fi

  # 判据声明本身也要核：待办里绑的测试名如果对不上（比如测试被改名了），
  # 那条判据永远不可能通过 —— 比「没通过」更坏，因为它的真实状态是未知的。
  step "判据声明自检"
  if node scripts/status.mjs --self-check >/tmp/quill-gate-status-decl.log 2>&1; then
    tail -1 /tmp/quill-gate-status-decl.log | sed 's/^/  ✓ /'
  else
    fail "有判据声明坏了（绑的测试名对不上等）—— 那些待办的真实状态是未知的"
    grep '✗' /tmp/quill-gate-status-decl.log | head -8 | sed 's/^/    /'
  fi
fi

if [ "$TEXT_ONLY" = "1" ]; then
  step "结果"
  if [ "$FAILED" != "0" ]; then
    echo "  有门禁没过"
  elif [ "$SKIPPED" = "1" ]; then
    echo "  跑到的部分全过；但缺上游参考源码，那几步**没有跑**（不算全绿）"
  elif [ "$need_text_gate" = "1" ]; then
    echo "  文本门禁全过（已跳过编译与测试，--text-only）"
  else
    echo "  本机没有 node，文本门禁未跑（--text-only）"
  fi
  if [ "$FAILED" != "0" ]; then exit 1; fi
  [ "$SKIPPED" = "1" ] && exit 1
  exit 0
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

# 一条命令跑**全部** Rust 测试。
#
# 原来这里只点名三个目标（server --lib / extensions_http / session_metrics_http）。
# 代价是 `http_contract` 那 33 条从没被执行过 —— 于是「专家市场接好了却忘了把
# 它加进已实现清单」这种错能一路绿灯。点名式清单必然会漏，而且漏得悄无声息。
run_cargo_tests "Rust 全工作区测试" cargo test --workspace

step "前端"
if [ "$HAVE_NODE" = "0" ] || [ ! -d ui/web ]; then
  echo "  - 跳过：本机没有 node。请在有 node 的一侧跑 npm run typecheck / npx vitest run / npm run build"
else
  cd ui/web || exit 1

  # ——— 本平台的原生依赖 ———
  # `node_modules` 在 D: 上，Windows 与 WSL 共用**同一份**。但 vite 8 用的
  # rolldown 带原生二进制，每个平台一个目录名（binding-win32-x64-msvc /
  # binding-linux-x64-gnu）。任一侧单独跑 `npm install` 都会把另一侧的删掉，
  # 于是另一侧启动就报 "Cannot find native binding"。
  #
  # 试过的几条路都不通：把两个包一起装，Windows 侧因为 libc 不符直接拒装
  # （notsup glibc）；用 --force 装上了，但下一次普通 `npm install` 又会被剪掉。
  # 它们不能写进 package.json —— 一旦写进去，Windows 上 `npm install` 会因为
  # 装不了 glibc 包而失败，那等于为了 WSL 把 Windows 侧弄坏。
  #
  # 所以按当前平台自愈：先**真的去加载一次** rolldown（而不是猜目录名），
  # 加载不了才补装。幂等，几秒钟。
  if node -e "import('rolldown').then(() => {}, () => process.exit(1))" >/dev/null 2>&1; then
    echo "  ✓ 本平台的原生依赖已就位"
  else
    echo "  · node_modules 是另一侧装的，缺本平台的原生依赖；现在补装…"
    if npm install --no-audit --no-fund >/tmp/quill-gate-npmi.log 2>&1; then
      if node -e "import('rolldown').then(() => {}, () => process.exit(1))" >/dev/null 2>&1; then
        echo "  ✓ 补装完成（另一侧再跑门禁时会自动补回它自己那份）"
      else
        fail "补装原生依赖后 rolldown 仍加载不了"; tail -5 /tmp/quill-gate-npmi.log | sed 's/^/    /'
      fi
    else
      fail "npm install 失败"; tail -5 /tmp/quill-gate-npmi.log | sed 's/^/    /'
    fi
  fi

  if npm run typecheck >/tmp/quill-gate-tsc.log 2>&1; then
    echo "  ✓ typecheck"
  else
    fail "tsc 未过"; tail -6 /tmp/quill-gate-tsc.log | sed 's/^/    /'
  fi
  # lint 也在门禁里。
  #
  # 原先这一段只有 typecheck / vitest / build，lint 从没被跑过 —— 于是
  # 「门禁退出 0」与「eslint 报 2 个 error」可以同时成立，而且不会有人
  # 发现。这正是本文件开头记的那类事故（「检查从来没真正生效过」）的
  # 另一个变种：不是检查写错了，是检查压根没接线。
  #
  # 判 error 不判 warning：`eslint .` 默认 warning 不影响退出码，而
  # react-hooks 这类规则默认就是 error，接不接线差别很大。
  if npx eslint . >/tmp/quill-gate-lint.log 2>&1; then
    # 把 warning 数也打出来：它不进退出码，但「从 11 降到 0」是看得见的。
    warn_n=$(grep -oE '[0-9]+ warnings?' /tmp/quill-gate-lint.log | head -1 | grep -oE '[0-9]+' || echo 0)
    echo "  ✓ lint（warning ${warn_n} 条，不计入退出码）"
  else
    fail "eslint 未过"; grep -E 'error|problems' /tmp/quill-gate-lint.log | head -10 | sed 's/^/    /'
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
[ "$HAVE_NODE" = "0" ] && SKIPPED=1
if [ "$FAILED" = "0" ]; then
  if [ "$SKIPPED" = "1" ]; then
    # 逐条说清缺什么，不要只说「缺 node」—— 少跑的原因可能不止一个。
    echo "  跑到的部分全过，但有步骤**没有跑**，所以不算全绿："
    [ "$HAVE_NODE" = "0" ] && echo "    · 本机缺 node：文本门禁与前端那几步未跑"
    [ -d "$UPSTREAM_REF_DIR" ] || echo "    · 缺上游参考源码（.octop-ref/octop）：专家库、上游基线、出处核验那三道未跑"
    exit 1
  fi
  echo "  门禁全过"
  exit 0
fi
echo "  有门禁没过 —— MILESTONES.md 的判据不成立，先修"
exit 1