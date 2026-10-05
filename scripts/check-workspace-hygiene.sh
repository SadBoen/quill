#!/usr/bin/env bash
# =============================================================================
# G38 工作区卫生闸门
# =============================================================================
# 查三件事，**每条都能判红**：
#   G38-a  自检临时目录残留：仓库内任何 .tmp-* 目录（.scratch/ 除外）
#   G38-b  被 git 跟踪的调试/中间态残留：_out*.txt / .git-rewrite/ / _r/ / _p2/ …
#   G38-c  被跟踪文件 **index 内容** 行尾一致：i/crlf 或 i/mixed 即违规
#
# ── 三态契约（铁律二 / 十六 / FALSIFY_CONTRACT）────────────────────────────
#   0 = 通过   1 = 检出违规   2 = 无法判定（**绝不计入通过**，措辞必含 ▲）
#
# ── --falsify（契约能力③④）───────────────────────────────────────────────
#   ① 注入**确定违规**的探针（.tmp-fake-leak/ 残留目录 + CRLF 污染文件）
#   ② 走**与正常模式完全相同**的 check_all()（不复制第二份实现）
#   ③ 断言「探针本身」被点名报出；报不出 ⇒ 检测逻辑已失效 ⇒ exit 2
#   ⚠️ 绝不能写成 `if FALSIFY; then exit 1; fi` —— 那种写法毫无鉴别力：
#      空闸门与恒绿闸门都能靠它「自证成功」（AGENTS.md 铁律：falsify 三条）。
# =============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; CYAN=$'\033[36m'; RESET=$'\033[0m'

# ── 参数约定（与 run-gate-selftest.sh 的调用形态对齐）────────────────────
#   $1 = 扫描根（runner 传 mktemp 目录；缺省扫仓库根）
#   其余 = --falsify / --selftest
SCAN_ROOT="$ROOT"
FALSIFY=0
for a in "$@"; do
  case "$a" in
    --falsify)  FALSIFY=1 ;;
    --selftest) FALSIFY=0 ;;
    -*) : ;;                          # 未知开关：忽略，不静默改变语义
    *)  [[ -d "$a" ]] && SCAN_ROOT="$a" ;;
  esac
done
[[ -d "$SCAN_ROOT" ]] || SCAN_ROOT="$ROOT"

# ── 依赖按**真实需要**声明（AGENTS.md：别把「实现语言相同」当「依赖相同」）──
#   G38-a 需要 find；G38-b / G38-c 需要 git。缺任一 ⇒ 无法判定，**不兜底**。
for t in find git; do
  command -v "$t" >/dev/null 2>&1 || {
    printf '%s▲ G38 无法判定：缺工具 %s —— 未检查 ≠ 通过，本闸门不计入通过%s\n' "$YELLOW" "$t" "$RESET" >&2
    exit 2
  }
done
# 🔴 2026-10-05 性能修正：这条「能力探测」原本在**当前目录**（= 真实仓库）跑全量
#   `git ls-files --eol`，实测 23 秒 —— 而它只是想确认 git 认不认识 `--eol` 这个选项。
#   改成在**空临时仓库**里探测：语义完全相同（确认 flag 被支持），成本 2 ms。
#   ⚠️ 别把它改回 CWD：那会让每次调用都白付 23 秒，包括扫临时探针仓库时。
_git_eol_probe="$(mktemp -d)"
if ! ( cd "$_git_eol_probe" && git init -q . && git ls-files --eol >/dev/null 2>&1 ); then
  printf '%s▲ G38 无法判定：git ls-files --eol 不可用（git 过旧）—— 不计入通过%s\n' "$YELLOW" "$RESET" >&2
  rm -rf "$_git_eol_probe" 2>/dev/null
  exit 2
fi
rm -rf "$_git_eol_probe" 2>/dev/null
GIT_OK=0
git -C "$SCAN_ROOT" rev-parse --git-dir >/dev/null 2>&1 && GIT_OK=1

# ── G38-b 模式表（外置成表，便于审计，不埋在逻辑里）──────────────────────
#   _out*.txt          重定向调试输出被误提交（本次事故原型）
#   .git-rewrite/      git fast-import 历史重写的工作态（206 个文件、480K）
#   _r/ _p2/           fixture 临时目录被误提交
#   _scratch/          成员临时工作区（.gitignore 已声明，绝不入库）
DEBUG_PATTERNS='(^|/)_out[^/]*\.txt$|(^|/)\.git-rewrite/|(^|/)_[rp][0-9]*/|(^|/)_scratch/|(^|/)__pycache__/'

# =============================================================================
# check_all —— **唯一**的检测实现（正常模式与 --falsify 共用，绝不复制两份）
#   $1 = 扫描根   $2 = 该根是否在 git 工作树内（0/1）
#   返回 0=干净 / 1=有违规 / 2=无法判定
# =============================================================================
check_all() {
  local root="$1" git_ok="$2" rc=0

  # ── G38-a ────────────────────────────────────────────────────────────
  # ⚠️ 必须 -prune 掉 .git/.scratch/target/node_modules：否则会钻进
  #    .git 对象库与 target 产物，既慢又造成海量误报。
  local leaks n_a
  leaks="$(find "$root" \
              \( -name .git -o -name .scratch -o -name target -o -name node_modules \) -prune -o \
              -type d -name '.tmp-*' -print 2>/dev/null)"
  n_a=$(printf '%s' "$leaks" | grep -c . )
  if (( n_a > 0 )); then
    printf '%s✗ rule G38-a: 自检临时目录残留 %d 个（自检产物，从不清理）%s\n' "$RED" "$n_a" "$RESET"
    printf '%s\n' "$leaks" | head -10 | sed 's/^/      /'
    (( n_a > 10 )) && printf '      … 另有 %d 个未列出\n' "$((n_a - 10))"
    rc=1
  fi
  printf '  [G38-a] 已扫描 %s：.tmp-* 残留 %d 个\n' "$root" "$n_a"

  if (( git_ok )); then
    local tracked n_all
    tracked="$(git -C "$root" ls-files)"
    n_all=$(printf '%s' "$tracked" | grep -c . )
    if (( n_all == 0 )); then
      # 「0 条」无法区分「干净」与「未检查」→ 显式判无法判定
      printf '%s▲ G38-b 无法判定：git ls-files 在 %s 返回 0 条 —— 无法区分「干净」与「未检查」%s\n' "$YELLOW" "$root" "$RESET" >&2
      return 2
    fi

    # ── G38-b ──────────────────────────────────────────────────────────
    local hit n_b
    hit="$(printf '%s\n' "$tracked" | grep -E "$DEBUG_PATTERNS")"
    n_b=$(printf '%s' "$hit" | grep -c . )
    if (( n_b > 0 )); then
      printf '%s✗ rule G38-b: 被 git 跟踪的调试/中间态残留 %d 条%s\n' "$RED" "$n_b" "$RESET"
      printf '%s\n' "$hit" | head -10 | sed 's/^/      /'
      (( n_b > 10 )) && printf '      … 另有 %d 条未列出\n' "$((n_b - 10))"
      rc=1
    fi
    printf '  [G38-b] 已检查被跟踪文件 %d 个：调试/中间态残留 %d 条\n' "$n_all" "$n_b"

    # ── G38-c ──────────────────────────────────────────────────────────
    # 判据落在 **index blob**（i/crlf / i/mixed），不是工作区：
    #   ① 真正进历史的是 index 内容；
    #   ② 免受各人 core.autocrlf 差异影响（否则本机绿、别人红）。
    # i/none（符号链接/空文件）与 i/-text（二进制）**不算违规** ——
    #   本库实测 363 条，不排除就是 363 条误报（误报闸门比假闸门更常见）。
    local eol_out eol_bad n_c n_wcrlf n_wmixed
    # 🔴 2026-10-05 性能修正：`git ls-files --eol` **只跑一次**，结果复用。
    #   `--eol` 要逐个读**工作区文件内容**才能判行尾，本仓库 2652 个跟踪文件。
    #   实测（WSL2 /mnt/d，9p 挂载到 Windows）：单次 **23.1 秒**（三次 23 119 / 23 183 /
    #   24 065 ms，几乎完全一致）。
    #   原实现连着调三次 → **69 秒**。这是纯浪费：三次读的是同一份数据。
    #   仓库搬到 ext4/native fs 上会快一个量级，但**重复三次在任何平台都是错的**。
    eol_out="$(git -C "$root" ls-files --eol)"
    eol_bad="$(printf '%s\n' "$eol_out" | grep -E '^i/(crlf|mixed)[[:space:]]')"
    n_c=$(printf '%s' "$eol_bad" | grep -c . )
    n_wcrlf=$(printf '%s\n' "$eol_out" | grep -cE '^i/lf[[:space:]]+w/crlf[[:space:]]' )
    n_wmixed=$(printf '%s\n' "$eol_out" | grep -cE '^i/lf[[:space:]]+w/mixed[[:space:]]' )
    if (( n_c > 0 )); then
      printf '%s✗ rule G38-c: 被跟踪文件行尾不一致 %d 个（index 内含 CRLF 或混行）%s\n' "$RED" "$n_c" "$RESET"
      printf '%s\n' "$eol_bad" | head -10 | sed 's/^/      /'
      rc=1
    fi
    printf '  [G38-c] 已检查 index 行尾 %d 个：违规 %d\n' "$n_all" "$n_c"
    # ⚠️ 提示**不是违规**，明确与「未检查」区分开
    if (( n_wcrlf > 0 || n_wmixed > 0 )); then
      printf '  ⚠ 提示（非违规）：工作区 w/crlf %d / w/mixed %d —— index 干净，git checkout 会按 .gitattributes 自愈\n' \
             "$n_wcrlf" "$n_wmixed"
    fi
  else
    # ⚠️ 「未检查」必须**显式可区分**（AGENTS.md：0 与未检查必须可区分）
    printf '%s▲ G38-b / G38-c 本次范围不在 git 工作树内（%s）—— 本次未检查，不计入通过%s\n' "$YELLOW" "$root" "$RESET" >&2
  fi

  return $rc
}

# =============================================================================
# --falsify：注入确定违规的探针 → 同一 check_all → 断言「探针本身」被点名
# =============================================================================
if (( FALSIFY )); then
  printf '%s── G38 (falsify) ── 已注入探针：.tmp-fake-leak/ + crlf-probe.txt%s\n' "$CYAN" "$RESET"

  PROBE="$(mktemp -d)" || {
    printf '%s▲ G38 无法判定：mktemp -d 失败%s\n' "$YELLOW" "$RESET" >&2; exit 2; }
  trap 'rm -rf "$PROBE"' EXIT

  # 🔴 安全护栏：探针目录必须在项目仓库**之外** ——
  #    否则下面的 git add 会污染项目索引（绝不对项目仓库 git add/commit）。
  case "$PROBE/" in "$ROOT/"*) printf '✗ SELF-TEST FAILED rule G38: 探针目录落在仓库内（%s），拒绝执行以免污染项目索引\n' "$PROBE" >&2; exit 2 ;; esac

  mkdir -p "$PROBE/.tmp-fake-leak/case1-probe"          # 探针 1 → 命中 rule G38-a
  printf 'alpha\r\nbeta\r\n' > "$PROBE/crlf-probe.txt"  # 探针 2 → 命中 rule G38-c
  ( cd "$PROBE" && git init -q . && git add crlf-probe.txt ) >/dev/null 2>&1

  if [[ ! -d "$PROBE/.tmp-fake-leak" || ! -f "$PROBE/crlf-probe.txt" ]]; then
    printf '✗ SELF-TEST FAILED rule G38: 探针未能构造 —— 闸门无法自证\n' >&2
    exit 2
  fi
  if ! git -C "$PROBE" ls-files --error-unmatch crlf-probe.txt >/dev/null 2>&1; then
    printf '✗ SELF-TEST FAILED rule G38: 探针文件未能进入探针仓库索引 —— git 不可用，无法自证\n' >&2
    exit 2
  fi

  _dbg="$(mktemp)"
  check_all "$PROBE" 1 >"$_dbg" 2>&1
  _rc=$?
  _out="$(cat "$_dbg")"
  rm -f "$_dbg"
  printf '%s\n' "$_out"

  # 🔴 判据 1：**返回值**才是检测逻辑的真实结论（输出文本只作佐证）。
  #    检测逻辑被破坏时它仍可能打印 "✗ rule ..." 再 return 0 —— 靠 grep 字样会误判。
  # 🔴 判据 2：断言**绑定到探针实际命中的两条规则**（G38-a / G38-c）。
  #    探针未构造 G38-b 违规，故**不得**对 G38-b 断言（否则真实闸门被误判为坏）。
  # 🔴 判据 3：断言输出里**点名了探针本体**，确保检出的是探针而非环境噪音。
  if (( _rc == 1 )) \
     && grep -qE 'rule G38-a' <<<"$_out" \
     && grep -qE 'rule G38-c' <<<"$_out" \
     && grep -qF '.tmp-fake-leak' <<<"$_out" \
     && grep -qF 'crlf-probe.txt' <<<"$_out"; then
    printf '  ✓ SELF-TEST OK rule G38: 探针被真实检出（rc=1 且点名 G38-a / G38-c 与探针路径）\n'
    exit 1
  fi

  printf '✗ SELF-TEST FAILED rule G38: 已知坏输入未被检出（rc=%d）—— 检测逻辑已失效，闸门无法自证\n' "$_rc" >&2
  exit 2
fi

# ── 正常模式 ────────────────────────────────────────────────────────────
printf '%s═══ G38 工作区卫生 ═══%s\n' "$CYAN" "$RESET"
printf '  扫描根：%s\n' "$SCAN_ROOT"
check_all "$SCAN_ROOT" "$GIT_OK"
rc=$?
case "$rc" in
  0) printf '%s✓ G38 通过（已检查项见上，非「未检查」）%s\n' "$GREEN" "$RESET" ;;
  2) printf '%s▲ G38 无法判定 —— 未判定 ≠ 通过，本闸门不计入通过%s\n' "$YELLOW" "$RESET" >&2 ;;
  *) printf '%s✗ G38 检出工作区卫生违规%s\n' "$RED" "$RESET" >&2 ;;
esac
exit $rc
