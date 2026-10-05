#!/usr/bin/env bash
# =============================================================================
# check-workspace-hygiene.sh（G38）闸门自检
#
# 依据：AGENTS.md「闸门必须双向可证伪」+ FALSIFY_CONTRACT
#   · 变红用例  —— 造已知坏输入，确认它真的判红（否则是第 1 类假闸门）
#   · 不误报用例 —— 造干净输入，确认它放行（误报闸门比假闸门**更常见**）
#
# 🔴 硬性约束（AGENTS.md 铁律：绝不在项目仓库里 git add）
#   **所有 fixture 都在 mktemp -d 造，绝不在仓库内建 git 索引。**
#   自检脚本每次运行会 assert 这一点 —— 若哪天有人改成在仓库内造 fixture，
#   本脚本会当场判红，而不是悄悄污染项目索引。
# =============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT/scripts/check-workspace-hygiene.sh"

RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
pass=0; fail=0

T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT

# 🔴 fixture 必须落在仓库之外 —— 否则 git add 会污染项目索引
case "$T/" in "$ROOT/"*)
  printf '%s✗ 自检自身错误：fixture 目录落在仓库内（%s）—— 拒绝执行，绝不在项目仓库 git add%s\n' \
         "$RED" "$T" "$RESET" >&2
  exit 1 ;;
esac

# --- 小工具 ---------------------------------------------------------------
mk_repo() {  # $1=名字 → 建一个干净的 fixture 仓库（含 1 个被跟踪的干净文件）
  local d="$T/$1"
  mkdir -p "$d"
  git -C "$d" init -q .
  # ⚠️ 显式关掉 autocrlf / eol 归一：否则各人 git 版本与全局配置不同，
  #    「CRLF 违规」这条用例会在别人机器上变成恒绿（第 7 类失效）。
  git -C "$d" config core.autocrlf false
  git -C "$d" config core.eol lf
  printf 'alpha\nbeta\n' > "$d/clean.txt"
  printf 'pub fn ok() -> u8 { 1 }\n' > "$d/lib.rs"
  git -C "$d" add clean.txt lib.rs
  printf '%s' "$d"
}

run_gate() {  # $1=被扫目录 → 打印 rc，其余写全局 RUN_OUT
  RUN_OUT="$(bash "$GATE" "$1" 2>&1)"; RUN_RC=$?
}

expect_rc() {  # $1=期望码 $2=说明
  local want="$1" name="$2"
  if [ "$RUN_RC" = "$want" ]; then
    printf '  %s✓%s %-46s rc=%s\n' "$GREEN" "$RESET" "$name" "$RUN_RC"; pass=$((pass+1))
  else
    printf '  %s✗%s %-46s rc=%s（期望 %s）\n' "$RED" "$RESET" "$name" "$RUN_RC" "$want"; fail=$((fail+1))
    printf '%s\n' "$RUN_OUT" | head -4 | sed 's/^/        /'
  fi
}

expect_has() {  # $1=子串 $2=说明   —— 判据取**输出**，用于措辞/覆盖断言
  local kw="$1" name="$2"
  if printf '%s' "$RUN_OUT" | grep -qF -- "$kw"; then
    printf '  %s✓%s %-46s 含「%s」\n' "$GREEN" "$RESET" "$name" "$kw"; pass=$((pass+1))
  else
    printf '  %s✗%s %-46s 缺「%s」\n' "$RED" "$RESET" "$name" "$kw"; fail=$((fail+1))
    printf '%s\n' "$RUN_OUT" | head -4 | sed 's/^/        /'
  fi
}

expect_lacks() {  # $1=子串 $2=说明  —— 反向：不该出现的字样（防误报/防噪音）
  local kw="$1" name="$2"
  if printf '%s' "$RUN_OUT" | grep -qF -- "$kw"; then
    printf '  %s✗%s %-46s 不该出现「%s」\n' "$RED" "$RESET" "$name" "$kw"; fail=$((fail+1))
    printf '%s\n' "$RUN_OUT" | grep -F -- "$kw" | head -3 | sed 's/^/        /'
  else
    printf '  %s✓%s %-46s 不含「%s」\n' "$GREEN" "$RESET" "$name" "$kw"; pass=$((pass+1))
  fi
}

# 🔴 环境依赖：缺工具必须**失败**，不许"跳过通过"（AGENTS.md 铁律二）
for t in git find; do
  command -v "$t" >/dev/null 2>&1 || {
    printf '%s▲ 无法判定：缺工具 %s —— 未检查 ≠ 通过，本自检不计入通过%s\n' "$YELLOW" "$t" "$RESET" >&2
    exit 2
  }
done
git ls-files --eol >/dev/null 2>&1 || {
  printf '%s▲ 无法判定：git ls-files --eol 不可用（git 过旧）—— 不计入通过%s\n' "$YELLOW" "$RESET" >&2
  exit 2
}

echo "########## G38 工作区卫生闸门自检 ##########"
echo "  fixture 根（仓库外）：$T"
echo

# =============================================================================
# 1. 基线：干净输入 → 必须通过
#    ⚠️ 必须放行 —— 这是**误报闸门**的唯一防线
# =============================================================================
CLEAN="$(mk_repo clean)"
run_gate "$CLEAN"; expect_rc 0 "基线：干净 fixture 仓库"
# 🔴 「0 条」与「未检查」必须可区分：断言它**确实检查了** N 个被跟踪文件
n_checked=$(printf '%s\n' "$RUN_OUT" | grep -oE '已检查被跟踪文件 [0-9]+' | grep -oE '[0-9]+' | head -1)
if [ -n "$n_checked" ] && [ "$n_checked" -gt 0 ]; then
  printf '  %s✓%s %-46s 已检查 %s 个\n' "$GREEN" "$RESET" "★非恒绿：报告里带实检数" "$n_checked"; pass=$((pass+1))
else
  printf '  %s✗%s %-46s 未报告实检数（0 与未检查不可区分）\n' "$RED" "$RESET" "★非恒绿：报告里带实检数"; fail=$((fail+1))
fi

# =============================================================================
# 2. 坏输入 A：`.tmp-*` 残留目录 → 必须判红（rule G38-a）
#    这是本闸门的原型事故：自检临时目录从不清理，实测累积到 242 个
# =============================================================================
A="$(mk_repo bad_tmpdir)"
mkdir -p "$A/.tmp-leftover/case1" "$A/nested/.tmp-deep"
run_gate "$A"
expect_rc 1 "坏输入 A：.tmp-* 残留目录"
expect_has "rule G38-a" "坏输入 A：输出点名 rule G38-a"
expect_has ".tmp-leftover" "坏输入 A：点名具体残留目录（不是泛泛而谈）"

# 2b. 🔴 反向用例：非 `.tmp-*` 的普通目录**绝不能**被误报 / 被删
#     防误报闸门（误报比假闸门更常见：一直红的门槛会被学会忽略）
B="$(mk_repo normal_dirs)"
mkdir -p "$B/tmp" "$B/.tmp" "$B/nottmp" "$B/mytmp-x" "$B/.tmpdir" "$B/target" "$B/src"
printf 'x\n' > "$B/src/lib.rs"
run_gate "$B"
expect_rc 0 "★不误报：tmp/.tmp/nottmp/mytmp-x/.tmpdir"
expect_lacks "rule G38-a" "★不误报：不得报 rule G38-a"
# 🔴 闸门只报不删：确认这些目录**真的还在**（"误删源码"是不可逆事故）
missing=""
for d in tmp .tmp nottmp mytmp-x .tmpdir target src; do
  [[ -d "$B/$d" ]] || missing="$missing $d"
done
if [ -z "$missing" ]; then
  printf '  %s✓%s %-46s 6/6 仍在\n' "$GREEN" "$RESET" "★不误报：闸门未删除任何目录"; pass=$((pass+1))
else
  printf '  %s✗%s %-46s 丢失:%s\n' "$RED" "$RESET" "★不误报：闸门未删除任何目录" "$missing"; fail=$((fail+1))
fi

# 2c. ★不误报：`.scratch/` 是成员临时工作区，闸门必须**显式剪掉**
#     （否则误报，且会与 runner 的 sweep_tmp 打架）
S="$(mk_repo scratch_dir)"
mkdir -p "$S/.scratch/probe" "$S/.tmp-real"
run_gate "$S"
expect_has "rule G38-a" "2c：.scratch 被剪掉、.tmp-real 仍被抓"
expect_lacks "rule G38-a: 自检临时目录残留 2" "2c：不得把 .scratch 算进残留数"

# =============================================================================
# 3. 坏输入 B：被 git 跟踪的调试残留（`_out*.txt`）→ 判红（rule G38-b）
# =============================================================================
C="$(mk_repo bad_debug)"
mkdir -p "$C/src"
printf 'noise from a stray redirect\n' > "$C/_out.txt"
printf 'more noise\n' > "$C/src/_out2.txt"
git -C "$C" add _out.txt src/_out2.txt
run_gate "$C"
expect_rc 1 "坏输入 B：被跟踪的 _out*.txt"
expect_has "rule G38-b" "坏输入 B：输出点名 rule G38-b"
expect_has "_out.txt" "坏输入 B：点名具体文件"

# 3b. 🔴 反向：名字**相似但不该算**的路径必须放行（防模式表过宽）
D="$(mk_repo lookalike)"
mkdir -p "$D/src"
printf 'pub fn o() -> u8 { 1 }\n' > "$D/src/output.rs"      # output.rs ≠ _out*.txt
printf 'pub fn t() -> u8 { 2 }\n'  > "$D/src/_test.rs"       # _test.rs 不在模式表内
printf 'note\n' > "$D/src/_out.txt"                            # 未跟踪 → 另一条规则
run_gate "$D"
expect_rc 0 "★不误报：output.rs / _test.rs / 未跟踪 _out.txt"
expect_lacks "rule G38-b" "★不误报：不得报 rule G38-b"

# =============================================================================
# 4. 坏输入 C：被跟踪的 CRLF 文件 → 判红（rule G38-c）
#    判据落在 **index blob**，不是工作区 —— 免受各人 autocrlf 差异影响
# =============================================================================
E="$(mk_repo bad_crlf)"
printf 'alpha\r\nbeta\r\ngamma\r\n' > "$E/crlf.txt"
git -C "$E" add crlf.txt
# 🔴 造完坏输入必须先验证它**真的进了 index 且真的是 CRLF**
#    （"命令返回 0"不等于"目的达成"—— AGENTS.md 铁律）
if git -C "$E" ls-files --error-unmatch crlf.txt >/dev/null 2>&1 \
   && git -C "$E" ls-files --eol | grep -qE '^i/crlf[[:space:]]'; then
  printf '  %s✓%s %-46s i/crlf\n' "$GREEN" "$RESET" "前提核验：CRLF 确实进了 index"; pass=$((pass+1))
else
  printf '  %s✗%s %-46s 未进 index 或非 CRLF —— 用例前提不成立\n' \
         "$RED" "$RESET" "前提核验：CRLF 确实进了 index"; fail=$((fail+1))
fi
run_gate "$E"
expect_rc 1 "坏输入 C：被跟踪的 CRLF 文件"
expect_has "rule G38-c" "坏输入 C：输出点名 rule G38-c"
expect_has "crlf.txt" "坏输入 C：点名具体文件"

# 4b. ★不误报：符号链接 / 二进制（index 报 i/none、i/-text）**不算违规**
#     脚本注释自述「本库实测 363 条，不排除就是 363 条误报」
F="$(mk_repo symlink_bin)"
ln -s clean.txt "$F/link.txt"
printf '\x00\x01\x02binary\xff' > "$F/blob.bin"
git -C "$F" add link.txt blob.bin
run_gate "$F"
expect_rc 0 "★不误报：符号链接 / 二进制文件"
expect_lacks "rule G38-c" "★不误报：不得报 rule G38-c"

# =============================================================================
# 5. 措辞合规（runner 的硬要求）
#    rc=2 必须含 ▲ / 「无法判定」/「未判定 ≠ 通过」—— 否则人读日志会误判为通过
# =============================================================================
G="$(mk_repo empty_idx)"
# 清空索引 → `git ls-files` 返回 0 条 → 脚本自己判「无法判定」（rc=2）
git -C "$G" rm -q --cached clean.txt lib.rs
run_gate "$G"
expect_rc 2 "措辞：索引 0 条 → 无法判定"
expect_has "▲" "措辞：rc=2 输出含 ▲"
# 🔴 反向断言：rc=2 **不得**输出「通过」字样（否则人读会误判）
expect_lacks "G38 通过" "措辞：rc=2 不得出现「通过」"

# =============================================================================
# 6. ★双向自证：把闸门的检测逻辑**破坏**掉，闸门必须不再判红
#    —— 证明「变红用例」测的是检测逻辑，而不是环境噪音
#    手法：复制一份副本，把 G38-a 的模式改成永不匹配（等价于「恒绿闸门」）
# =============================================================================
BROKEN="$T/broken-gate.sh"
sed -e "s/-type d -name '\.tmp-\*' -print/-type d -name '.never-matches-xyz' -print/" \
    "$GATE" > "$BROKEN"
if ! grep -q 'never-matches-xyz' "$BROKEN"; then
  printf '  %s✗%s %-46s sed 未命中（闸门实现已变）—— 用例失效\n' \
         "$RED" "$RESET" "★反向：检测逻辑被破坏后必须不再报 G38-a"; fail=$((fail+1))
else
  RUN_OUT="$(bash "$BROKEN" "$A" 2>&1)"; RUN_RC=$?
  if [ "$RUN_RC" = "0" ] && ! printf '%s' "$RUN_OUT" | grep -qF 'rule G38-a'; then
    printf '  %s✓%s %-46s rc=0 且不报 G38-a\n' "$GREEN" "$RESET" "★反向：破坏后不再报 G38-a"; pass=$((pass+1))
  else
    printf '  %s✗%s %-46s rc=%s（期望 0）\n' "$RED" "$RESET" "★反向：破坏后不再报 G38-a" "$RUN_RC"; fail=$((fail+1))
  fi
fi
rm -f "$BROKEN"

# --- 汇总 -------------------------------------------------------------------
echo
echo "########## 结果：通过 $pass / 失败 $fail ##########"
if [ "$fail" = "0" ]; then
  echo "SELFTEST_PASS"
  exit 0
fi
echo "SELFTEST_FAIL"
exit 1
