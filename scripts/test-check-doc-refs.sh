#!/usr/bin/env bash
# ==============================================================================
# test-check-doc-refs.sh —— G39 的自检（双向用例）
#
# 契约（docs/FALSIFY_CONTRACT.md）：
#   能力① 正向：坏输入必须判红（rc=1）
#   能力② 反向：干净输入必须放行（rc=0）—— **防误报闸门**，缺这条本自检不算数
#   能力③ --falsify 必须能自证
#   能力④ 探针未检出 ⇒ rc=2 + SELF-TEST FAILED
#
# 🔴 每条用例都在 **mktemp -d 的临时仓库**里跑，不碰项目仓库，
#   也不对项目仓库做任何写操作（AGENTS.md 正被同事并行重写）。
#
# 退出码：0 = 全部通过 / 1 = 有用例失败
# ==============================================================================
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GATE="$HERE/check-doc-refs.sh"

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; DIM=$'\033[2m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; DIM=''; RESET=''
fi

PASS=0
FAIL=0
FAILED_CASES=()

TMPROOT="$(mktemp -d)"
trap 'rm -rf "$TMPROOT"' EXIT

# --- 断言工具 --------------------------------------------------------------
# 🔴 判据一律取**返回值** + 关键措辞，绝不只看输出里有没有某个词。
ok() { PASS=$((PASS+1)); printf '  %s✓%s %s\n' "$GREEN" "$RESET" "$1"; }
bad() {
  FAIL=$((FAIL+1)); FAILED_CASES+=("$1")
  printf '  %s✗%s %s\n' "$RED" "$RESET" "$1"
  printf '%s\n' "$2" | sed 's/^/      /'
}

# 新建一个临时"仓库"：mk <名字> → 打印路径
mk() { local d="$TMPROOT/$1"; mkdir -p "$d"; printf '%s' "$d"; }

# 断言：命令应以 rc=$2 结束，且输出满足 expect（正则，可为空）
assert_rc() {
  local name="$1" want="$2" expect="$3"; shift 3
  local out rc
  out="$("$@" 2>&1)"; rc=$?
  if [[ "$rc" != "$want" ]]; then
    bad "$name" "期望 rc=$want，实际 rc=$rc
--- 输出 ---
$out"
    return 1
  fi
  if [[ -n "$expect" ]] && ! printf '%s' "$out" | grep -qE "$expect"; then
    bad "$name" "rc 正确但输出缺少 /$expect/
--- 输出 ---
$out"
    return 1
  fi
  ok "$name"
  return 0
}

echo "${DIM}── G39 自检 ──${RESET}"

# ══════════════════════════════════════════════════════════════════════
# 1. --falsify 自证（能力③④）
# ══════════════════════════════════════════════════════════════════════
echo "[能力③④] --falsify 注入坏输入必须被判红"
assert_rc "--falsify 检出注入的假引用并自证" 1 'SELF-TEST OK' \
  bash "$GATE" --falsify

# 破损闸门检测：把 check_all 的返回改成 0（模拟"打印警告但返回成功"），
# 真实的 falsify 必须判它**无法自证**（rc=2），而不是被输出字样骗过。
echo "[陷阱3] 返回值 vs 输出字样：假闸门不得通过自证"
BROKEN="$TMPROOT/broken-gate.sh"
sed 's/^  if (( _g_viol )); then return 1; fi$/  if (( 0 )); then return 1; fi/' "$GATE" > "$BROKEN"
if grep -q 'if (( 0 )); then return 1; fi' "$BROKEN"; then
  assert_rc "检测逻辑失效的副本必须 rc=2 且措辞含 SELF-TEST FAILED" 2 'SELF-TEST FAILED' \
    bash "$BROKEN" --falsify
else
  bad "构造破损闸门副本" "sed 未命中 check_all 的返回语句，自检自身失效（不能证明判据有效）"
fi

# ══════════════════════════════════════════════════════════════════════
# 2. 能力① 坏输入必须判红
# ══════════════════════════════════════════════════════════════════════
echo "[能力①] 坏输入判红"
R1="$(mk r1)"; mkdir -p "$R1/scripts" "$R1/crates/demo"
: > "$R1/scripts/real.sh"
printf 'x\n' > "$R1/crates/demo/src.rs"
cat > "$R1/bad.md" <<'EOF'
引用 `scripts/definitely-not-here.sh`。
EOF
assert_rc "G39-a 不存在的仓库路径 → rc=1 且含 rule G39-a" 1 'rule G39-a' \
  bash "$GATE" "$R1/bad.md"

R2="$(mk r2)"; mkdir -p "$R2/scripts"; : > "$R2/scripts/real.sh"
cat > "$R2/bad2.md" <<'EOF'
自检请跑 `bash scripts/also-not-here.sh`。
EOF
assert_rc "G39-a 解释器调用不存在的脚本 → rc=1" 1 'rule G39-a' \
  bash "$GATE" "$R2/bad2.md"

R3="$(mk r3)"; mkdir -p "$R3/scripts"; : > "$R3/scripts/real.sh"
cat > "$R3/bad3.md" <<'EOF'
未实现的命令 `cargo definitely-not-a-subcommand-zzz`。
EOF
if bash -c 'command -v cargo >/dev/null 2>&1 || [ -x "$HOME/.cargo/bin/cargo" ]'; then
  assert_rc "G39-b 不存在的 cargo 子命令 → rc=1 且含 rule G39-b" 1 'rule G39-b' \
    bash "$GATE" "$R3/bad3.md"
else
  printf '  %s▲%s 跳过 G39-b 用例：本机无 cargo（无法判定 ≠ 通过，闸门自身会判 rc=2）\n' \
    "$YELLOW" "$RESET"
fi

R4="$(mk r4)"; mkdir -p "$R4/scripts"; : > "$R4/scripts/real.sh"
cat > "$R4/bad4.md" <<'EOF'
路径带行号后缀 `scripts/ghost.sh:42`。
EOF
assert_rc "G39-a 路径带 :行号 后缀且文件不存在 → rc=1" 1 'rule G39-a' \
  bash "$GATE" "$R4/bad4.md"

# ══════════════════════════════════════════════════════════════════════
# 3. 能力② 干净输入必须放行（🔴 防误报闸门，缺这条自检不算数）
# ══════════════════════════════════════════════════════════════════════
echo "[能力②] 干净输入放行（防误报）"
C1="$(mk c1)"; mkdir -p "$C1/scripts" "$C1/crates/demo" "$C1/docs" "$C1/deploy"
: > "$C1/scripts/real.sh"
printf 'x\n' > "$C1/crates/demo/src.rs"
printf 'x\n' > "$C1/docs/PLAN.md"
printf 'x\n' > "$C1/deploy/quill.env"
cat > "$C1/clean.md" <<'EOF'
# 干净文档
真实存在的引用：`scripts/real.sh`、`crates/demo/src.rs`、`docs/PLAN.md`、
`deploy/quill.env`、`crates/demo/src.rs:12`、目录 `scripts/`。
命令 `cargo test --workspace` 与 `bash scripts/real.sh` 都存在。
不是路径的：`UserId`、`AdapterError`、见铁律十四、`schema.org`、
`crates/*`（通配示意）、`/var/lib/quill`（绝对路径）、`~/.cargo/registry`、
`$PWD`、`..`、`<user>`、`06_xxx.md`（占位）。
EOF
assert_rc "干净文档全部放行 rc=0" 0 '通过' \
  bash "$GATE" "$C1/clean.md"

C2="$(mk c2)"; mkdir -p "$C2/scripts"; : > "$C2/scripts/real.sh"
cat > "$C2/fenced.md" <<'EOF'
# 围栏用例
下面这段里的假引用**在代码围栏内**，必须被跳过：

```bash
bash scripts/this-does-not-exist.sh
`scripts/also-missing.sh`
cargo totally-not-a-subcommand
```

围栏外这句里的假引用**必须**被判红：
`scripts/still-missing.sh`
EOF
assert_rc "围栏内假引用不误报、围栏外假引用仍判红 rc=1" 1 'rule G39-a:.*still-missing' \
  bash "$GATE" "$C2/fenced.md"

C3="$(mk c3)"; mkdir -p "$C3/crates/quill-store/tests" "$C3/scripts"
: > "$C3/scripts/real.sh"
cat > "$C3/resolved.md" <<'EOF'
仓库内可达但非根相对：`crates/quill-store/tests`。
存在但不是文件路径的示例：`crates/*`、`target/release/`。
EOF
assert_rc "构建产物/通配符/非根相对路径不误报 rc=0" 0 '通过' \
  bash "$GATE" "$C3/resolved.md"

# 防误报专项：三态记法与裸域名都长得像"路径"，必须放行
C3B="$(mk c3b)"; mkdir -p "$C3B/scripts"; : > "$C3B/scripts/real.sh"
cat > "$C3B/lookalike.md" <<'EOF'
三态退出码记法 `0/1/2`、比例记法 `2/3/4`。
裸域名（无 scheme）：`gist.github.com/karpathy/442a6bf`、`schema.org`。
EOF
assert_rc "三态记法与裸域名不误报 rc=0" 0 '通过' \
  bash "$GATE" "$C3B/lookalike.md"

# 反向断言：域名排除规则不得扩成"带点的一律放过"
C3C="$(mk c3c)"; mkdir -p "$C3C/scripts" "$C3C/crates"
: > "$C3C/scripts/real.sh"
cat > "$C3C/lookalike-bad.md" <<'EOF'
看起来像域名但确实是仓库内缺失路径：`crates/ghost-crate/src/lib.rs`。
EOF
assert_rc "反向断言：带点的仓库内缺失路径仍判红 rc=1" 1 'rule G39-a.*ghost-crate' \
  bash "$GATE" "$C3C/lookalike-bad.md"

# ══════════════════════════════════════════════════════════════════════
# 4. 恒绿检测：一个候选都没提取到 ⇒ 无法判定（rc=2），不是通过
# ══════════════════════════════════════════════════════════════════════
echo "[恒绿] 空文档必须判 rc=2（▲）而不是通过"
C4="$(mk c4)"
cat > "$C4/empty.md" <<'EOF'
# 空文档
这里没有任何反引号引用，也没有路径。
EOF
assert_rc "无任何候选引用 → rc=2 且措辞含 ▲" 2 '▲' \
  bash "$GATE" "$C4/empty.md"

C5="$(mk c5)"
assert_rc "目标文件不存在 → rc=2 且措辞含 ▲" 2 '▲' \
  bash "$GATE" "$C5/no-such-file.md"

# ══════════════════════════════════════════════════════════════════════
# 5. 缺工具必须 rc=2（禁止 || true 兜底 —— E5 铁律）
# ══════════════════════════════════════════════════════════════════════
echo "[缺工具] 缺 find 时必须判 rc=2（▲），不得判绿"
C6="$(mk c6)"; mkdir -p "$C6/scripts"; : > "$C6/scripts/real.sh"
printf '引用 `scripts/real.sh`。\n' > "$C6/clean6.md"
STUB="$TMPROOT/stubbin"; mkdir -p "$STUB"
# 造一个只含 awk/grep/mktemp、**不含 find** 的 PATH
for t in awk grep mktemp; do
  real="$(command -v "$t")"
  printf '#!/bin/sh\nexec %s "$@"\n' "$real" > "$STUB/$t"
  chmod +x "$STUB/$t"
done
# 🔴 用 bash 的**绝对路径**调用：PATH 已被抽空，裸 `bash` 会 command not found（rc=127），
#    那样测的是"找不到 bash"，不是"缺 find 时闸门会怎样"。
BASH_ABS="$(command -v bash)"
out=$(PATH="$STUB" "$BASH_ABS" "$GATE" "$C6/clean6.md" 2>&1); rc=$?
if [[ "$rc" == 2 ]] && printf '%s' "$out" | grep -q '▲'; then
  ok "缺 find → rc=2 且含 ▲"
else
  bad "缺 find 必须 rc=2 且含 ▲" "实际 rc=$rc
--- 输出 ---
$out"
fi

# ══════════════════════════════════════════════════════════════════════
# 6. 豁免机制：g39:waive 必须生效，且**豁免之外仍须判红**（反向断言）
# ══════════════════════════════════════════════════════════════════════
echo "[豁免] 显式豁免生效 + 豁免之外仍判红（反向断言）"
C7="$(mk c7)"; mkdir -p "$C7/scripts"; : > "$C7/scripts/real.sh"
cat > "$C7/waived.md" <<'EOF'
<!-- g39:waive token=scripts/intentionally-absent.sh reason=作为幻觉引用的反例，正文已声明其不存在 -->
这里故意引用一个不存在的文件 `scripts/intentionally-absent.sh` 来说明幻觉引用。
EOF
assert_rc "带 g39:waive 的引用不再判红 rc=0" 0 '显式豁免' \
  bash "$GATE" "$C7/waived.md"

# 豁免 token **含空格**时的解析（曾经按"非空格"截断，导致豁免静默失效）
C7B="$(mk c7b)"; mkdir -p "$C7B/scripts"; : > "$C7B/scripts/real.sh"
cat > "$C7B/waived-spaces.md" <<'EOF'
<!-- g39:waive token=cargo no-such-subcmd-zzz reason=负向断言：反例，命令不存在 -->
<!-- g39:waive token=scripts/absent too reason=理由里也含空格 -->
含空格的豁免 token：`cargo no-such-subcmd-zzz` 与 `scripts/absent too`。
EOF
assert_rc "含空格的豁免 token 能被正确解析并放行 rc=0" 0 '显式豁免' \
  bash "$GATE" "$C7B/waived-spaces.md"

C7C="$(mk c7c)"; mkdir -p "$C7C/scripts"; : > "$C7C/scripts/real.sh"
cat > "$C7C/waived-spaces-bad.md" <<'EOF'
<!-- g39:waive token=cargo no-such-subcmd-zzz reason=反例 -->
豁免项 `cargo no-such-subcmd-zzz` 放行，但 `cargo other-bogus-subcmd-yyy` 必须仍判红。
EOF
assert_rc "反向断言：含空格豁免之外的假子命令仍判红 rc=1" 1 'rule G39-b' \
  bash "$GATE" "$C7C/waived-spaces-bad.md"

cat > "$C7/waived-then-bad.md" <<'EOF'
<!-- g39:waive token=scripts/intentionally-absent.sh reason=反例 -->
豁免项 `scripts/intentionally-absent.sh` 放行，
但**豁免之外**的 `scripts/should-still-be-red.sh` 必须仍判红。
EOF
assert_rc "反向断言：豁免之外的违规仍判红 rc=1" 1 'rule G39-a:.*should-still-be-red' \
  bash "$GATE" "$C7/waived-then-bad.md"

# ══════════════════════════════════════════════════════════════════════
# 【分档】2026-10-05 新增。分档是**判定逻辑的改动**，不配自检就是新的假闸门。
#   必须证明三件事：① 分档真的改变 rc（否则等于没实现）
#                    ② 缺状态行**从严**判 authoritative（不得默认放过）
#                    ③ 放宽只作用于 reference/archived，authoritative 一条都不放过
printf '\n[分档] authoritative 判红 / reference 与 archived 只计数（2026-10-05）\n'

T1="$(mk t1)"; mkdir -p "$T1/scripts"; : > "$T1/scripts/real.sh"
cat > "$T1/auth.md" <<'EOF'
<!-- doc-status: authoritative -->
权威文档引用了不存在的脚本：`scripts/absent-in-authoritative.sh`。
EOF
assert_rc "authoritative 档的假引用必须 rc=1" 1 'rule G39-a' \
  bash "$GATE" "$T1/auth.md"

T2="$(mk t2)"; mkdir -p "$T2/scripts"; : > "$T2/scripts/real.sh"
cat > "$T2/ref.md" <<'EOF'
<!-- doc-status: reference -->
设计详稿快照引用了当时规划但未落地的 `scripts/absent-in-reference.sh`。
EOF
assert_rc "reference 档的假引用只计数、rc=0" 0 'reference 冻结档 1' \
  bash "$GATE" "$T2/ref.md"

T3="$(mk t3)"; mkdir -p "$T3/scripts"; : > "$T3/scripts/real.sh"
cat > "$T3/arch.md" <<'EOF'
<!-- doc-status: archived -->
已作废留档，引用 `scripts/absent-in-archived.sh` 不判定。
EOF
assert_rc "archived 档不判定、rc=0" 0 'archived 已作废 1' \
  bash "$GATE" "$T3/arch.md"

T4="$(mk t4)"; mkdir -p "$T4/scripts"; : > "$T4/scripts/real.sh"
cat > "$T4/nostatus.md" <<'EOF'
没有写 doc-status 状态的文档，引用了 `scripts/absent-no-status.sh`。
EOF
assert_rc "缺状态行从严判 authoritative → rc=1（不得默认放过）" 1 'rule G39-a' \
  bash "$GATE" "$T4/nostatus.md"

# 反向断言：reference 档里的**未标注**违规不得被档位一起放过
cat > "$T4/mixed.md" <<'EOF'
<!-- doc-status: reference -->
冻结档里的 `scripts/absent-frozen-ok.sh` 计入冻结档，
`cargo definitely-not-a-subcommand-zzz` 同样计入 —— 不得被静默丢弃。
EOF
assert_rc "反向断言：reference 档内违规仍被计数，不得静默丢弃" 0 'reference 冻结档 2' \
  bash "$GATE" "$T4/mixed.md"

printf '\n[CIDR] 网段记法不误报，但相邻形态仍判红（2026-10-05）\n'
T5="$(mk t5)"; mkdir -p "$T5/scripts"; : > "$T5/scripts/real.sh"
cat > "$T5/cidr.md" <<'EOF'
<!-- doc-status: authoritative -->
允许的回环与链路本地网段：`127.0.0.1/8`、`10/8`、`169.254.0.0/16`，均不是仓库路径。
EOF
assert_rc "CIDR 网段记法不误报 rc=0" 0 'authoritative 违规 0' \
  bash "$GATE" "$T5/cidr.md"

# 🔴 相邻形态：加了后缀就不再是 CIDR，必须重新判红
#   （修「误报」最常见的坑就是排除规则写太宽，把真幻觉一起放过）
cat > "$T5/cidr-adjacent.md" <<'EOF'
<!-- doc-status: authoritative -->
形似 CIDR 但不是网段记法，仍须判红：`scripts/127.0.0.1/8-notes.md`。
EOF
assert_rc "相邻形态：形似 CIDR 的缺失路径仍判红 rc=1" 1 'rule G39-a' \
  bash "$GATE" "$T5/cidr-adjacent.md"

printf '\n[通配符] 含 * 的命令不是可执行引用，但不含 * 的仍判红（2026-10-05）\n'
T6="$(mk t6)"; mkdir -p "$T6/scripts"; : > "$T6/scripts/real.sh"
cat > "$T6/wildcard.md" <<'EOF'
<!-- doc-status: authoritative -->
整族幻觉命令的通配写法 `cargo xtask *` 不该被判红（带 * 的命令跑不起来）。
EOF
assert_rc "含通配符的命令 token 放行 rc=0" 0 'authoritative 违规 0' \
  bash "$GATE" "$T6/wildcard.md"

# 🔴 反向断言：放行通配符**不能**顺带放过不含通配符的真幻觉命令
cat > "$T6/wildcard-adjacent.md" <<'EOF'
<!-- doc-status: authoritative -->
通配写法 `cargo xtask *` 放行，但具体子命令 `cargo xtask boundary` 必须仍判红。
EOF
assert_rc "反向断言：去掉通配符的假子命令仍判红 rc=1" 1 'rule G39-b' \
  bash "$GATE" "$T6/wildcard-adjacent.md"

printf '\n[git 引用] origin/<分支> 被**真验证**而非排除（2026-10-05）\n'
T7="$(mk t7)"; mkdir -p "$T7/scripts"; : > "$T7/scripts/real.sh"
# 造一个真 git 仓库并建一个远端 ref，用来验证「真引用」那一半
git -C "$T7" init -q
git -C "$T7" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
git -C "$T7" update-ref refs/remotes/origin/main HEAD
cat > "$T7/ref.md" <<'EOF'
<!-- doc-status: authoritative -->
达成后推 `origin/main` 启用 CI —— 这是真实存在的远端引用。
EOF
assert_rc "真实存在的 origin/main 放行 rc=0" 0 'authoritative 违规 0' \
  bash "$GATE" "$T7/ref.md"

# 🔴 反向断言：若当初的"修复"是把 origin/* 加进排除表，这条就会红 ——
#   排除 = 用消除误报换鉴别力；本闸门选择**验证**，故幻觉引用必须判红。
cat > "$T7/ghost.md" <<'EOF'
<!-- doc-status: authoritative -->
幻觉的远端引用 `origin/definitely-not-a-branch-zzz` 必须判红。
EOF
assert_rc "反向断言：幻觉的 origin/<分支> 仍判红 rc=1" 1 'rule G39-a' \
  bash "$GATE" "$T7/ghost.md"

# 🔴 相邻形态：`origin/a/b` 多层不是 ref，退回路径判定 → 判红
cat > "$T7/deep.md" <<'EOF'
<!-- doc-status: authoritative -->
多层的 `origin/a/b` 不是 git ref，退回路径判定，须判红。
EOF
assert_rc "相邻形态：多层 origin/a/b 仍判红 rc=1" 1 'rule G39-a' \
  bash "$GATE" "$T7/deep.md"

# 🔴 顺序用例：`origin/*` 是"这一整族引用"，不是某一条具体 ref。
#   若 git 引用分支被插在通配符判断**之前**，它会匹配 ^origin/[^/]+$
#   并被真的去 verify → 判红 → 与既有"含通配符不是可执行引用"规则冲突。
#   这条钉住的是**插入位置**，不是分类本身（顺序也是语义的一部分）。
cat > "$T7/family.md" <<'EOF'
<!-- doc-status: authoritative -->
泛指一族远端引用的 `origin/*` 不是可执行引用，不该判红。
EOF
assert_rc "顺序用例：origin/* 通配放行 rc=0" 0 'authoritative 违规 0' \
  bash "$GATE" "$T7/family.md"

# ══════════════════════════════════════════════════════════════════════
echo
printf '%s\n' "${DIM}────────────────────────────${RESET}"
if (( FAIL )); then
  printf '%s\n' "${RED}❌ G39 自检失败：${PASS} 通过 / ${FAIL} 失败${RESET}" >&2
  for c in "${FAILED_CASES[@]}"; do printf '   · %s\n' "$c" >&2; done
  exit 1
fi
printf '%s\n' "${GREEN}✓ G39 自检通过：${PASS}/${PASS}（含 1 破损闸门反证 + 5 防误报 + 分档/CIDR/通配符/git引用 双向用例）${RESET}"
exit 0
