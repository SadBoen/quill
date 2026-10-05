#!/usr/bin/env bash
# ==============================================================================
# test-check-crate-deps.sh —— G40 闸门的自检（双向：违规变红 + 干净放行）
#
# 🔴 判据（铁律十二/十三）：
#   只测违规不测豁免 → 闸门退化成"一律禁止"，逼后人绕过它；
#   只测通过不测违规 → 恒绿闸门。
#   **缺第②组的禁令等于半个假闸门**，故本脚本 4 组必测：
#     ① 四类坏输入各自变红（quill-adapters→quill-store / wiki→agent / →goose /
#        非 server crate 出现 unreachable_pub）+ 自环
#     ② 干净输入放行（防误报）
#     ③ 边界放行：quill-agent→quill-wiki（契约**允许**，别写反）
#                [dev-dependencies] 含 quill-*（dev 依赖不成环）
#                注释里出现 unreachable_pub 三个字（不是代码）
#     ④ --falsify 自证：rc=1 + SELF-TEST OK
#
# 🔴 全部断言都取**被测脚本的退出码**（契约 §1.2），grep 字样只作佐证。
# 🔴 所有临时仓库都在 mktemp -d 内，**绝不在项目仓库内**，绝不 git add。
#
# 退出码：0=全部通过 / 1=有用例未通过
# ==============================================================================
set -uo pipefail

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; DIM=''; RESET=''
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GATE="$SCRIPT_DIR/check-crate-deps.sh"
PASS=0
FAILED=()

if [[ ! -f "$GATE" ]]; then
  printf '%s\n' "${RED}❌ 闸门脚本不存在：$GATE${RESET}" >&2
  exit 2
fi

TMPROOT=$(mktemp -d)
trap 'rm -rf "$TMPROOT"' EXIT

# --- 用例登记：名字 / 期望 rc / 期望出现在输出里的 rule ID（空=不要求）--------
N=0
run_case() {
  # $1=用例名 $2=期望rc $3=期望rule片段 $4=crate 树构造函数名
  local name="$1" want="$2" ruletag="$3" builder="$4"
  N=$((N + 1))
  local tree="$TMPROOT/case$N"
  mkdir -p "$tree"
  "$builder" "$tree"
  local out rc
  out="$(bash "$GATE" "$tree/crates" 2>&1)"; rc=$?
  local ok=1 why=""
  if [[ "$rc" != "$want" ]]; then ok=0; why="rc=$rc，期望 $want"; fi
  if (( ok )) && [[ -n "$ruletag" ]] && ! printf '%s' "$out" | grep -q "$ruletag"; then
    ok=0; why="rc 正确但输出缺 '$ruletag'"
  fi
  if (( ok )) && [[ -z "$ruletag" ]] && [[ "$rc" == 1 ]]; then
    ok=0; why="期望放行却判红"
  fi
  if (( ok )); then
    PASS=$((PASS + 1))
    printf '  %s✓%-2d%s %-46s rc=%s（期望 %s）\n' "$GREEN" "$N" "$RESET" "$name" "$rc" "$want"
  else
    FAILED+=("[$name] $why")
    printf '  %s✗%-2d%s %-46s %s\n' "$RED" "$N" "$RESET" "$name" "$why"
    printf '%s\n' "$out" | sed 's/^/        /' | head -20
  fi
}

# --- crate 树构造器 --------------------------------------------------------
mk_pkg() { # $1=dir $2=name
  mkdir -p "$1/src"
  { echo '[package]'; echo "name = \"$2\""; echo 'version = "0.1.0"'; } > "$1/Cargo.toml"
  : > "$1/src/lib.rs"
}

b_adapters_to_store() { # G40-a
  local t="$1/crates"
  mk_pkg "$t/quill-store" quill-store
  mk_pkg "$t/quill-adapters" quill-adapters
  printf '\n[dependencies]\nquill-store = { path = "../quill-store" }\n' >> "$t/quill-adapters/Cargo.toml"
}
b_adapters_selfloop() { # G40-a 自环
  local t="$1/crates"
  mk_pkg "$t/quill-adapters" quill-adapters
  printf '\n[dependencies]\nquill-adapters = { path = "../quill-adapters" }\n' >> "$t/quill-adapters/Cargo.toml"
}
b_wiki_to_agent() { # G40-b
  local t="$1/crates"
  mk_pkg "$t/quill-agent" quill-agent
  mk_pkg "$t/quill-wiki" quill-wiki
  printf '\n[dependencies]\nquill-agent = { path = "../quill-agent" }\n' >> "$t/quill-wiki/Cargo.toml"
}
b_goose_cratesio() { # G40-c（crates.io 写法）
  local t="$1/crates"
  mk_pkg "$t/quill-store" quill-store
  printf '\n[dependencies]\ngoose = "0.1"\n' >> "$t/quill-store/Cargo.toml"
}
b_goose_vendor_path() { # G40-c（vendor path 写法）
  local t="$1/crates"
  mk_pkg "$t/quill-adapters" quill-adapters
  printf '\n[dependencies]\ngoose-core = { path = "../../vendor/goose" }\n' >> "$t/quill-adapters/Cargo.toml"
}
b_unreachable_pub() { # G40-d
  local t="$1/crates"
  mk_pkg "$t/quill-store" quill-store
  printf 'pub struct A;\npub fn leaked() {}\n' > "$t/quill-store/src/lib.rs"
  printf '\n[lib]\nname = "quill_store"\n' >> "$t/quill-store/Cargo.toml"
  echo '// 契约要求可见性收敛，下列 pub 不得为 unreachable' > "$t/quill-store/src/extra.rs"
  # 真正的违规：unreachable_pub 修饰真实代码
  printf 'unreachable_pub fn leaky() {}\n' > "$t/quill-store/src/leak.rs"
}
b_empty_tree() { # 全树零运行时依赖 → 必须 rc=2「恒绿嫌疑」而不是 rc=0
  local t="$1/crates"
  mk_pkg "$t/quill-domain" quill-domain
  mk_pkg "$t/quill-adapters" quill-adapters
}
b_clean() { # 干净输入必须放行
  local t="$1/crates"
  mk_pkg "$t/quill-domain" quill-domain
  mk_pkg "$t/quill-adapters" quill-adapters
  mk_pkg "$t/quill-wiki" quill-wiki
  mk_pkg "$t/quill-agent" quill-agent
  mk_pkg "$t/quill-store" quill-store
  mk_pkg "$t/quill-server" quill-server
  printf '\n[dependencies]\nquill-adapters = { path = "../quill-adapters" }\nquill-domain = { path = "../quill-domain" }\n' >> "$t/quill-wiki/Cargo.toml"
  printf '\n[dependencies]\nquill-adapters = { path = "../quill-adapters" }\nquill-domain = { path = "../quill-domain" }\nquill-wiki = { path = "../quill-wiki" }\n' >> "$t/quill-agent/Cargo.toml"
  printf '\n[dependencies]\nquill-adapters = { path = "../quill-adapters" }\n' >> "$t/quill-server/Cargo.toml"
  printf 'pub struct T;\n' > "$t/quill-store/src/lib.rs"
}
b_agent_to_wiki() { # 边界①：契约**允许** agent→wiki
  local t="$1/crates"
  mk_pkg "$t/quill-wiki" quill-wiki
  mk_pkg "$t/quill-agent" quill-agent
  printf '\n[dependencies]\nquill-wiki = { path = "../quill-wiki" }\n' >> "$t/quill-agent/Cargo.toml"
}
b_dev_deps_only() { # 边界②：dev 依赖不成环
  # ⚠️ 附带一条**合法的真实边**（wiki → adapters）：否则整棵树一条运行时依赖都没有，
  #    会撞上闸门的「0 条边 = 恒绿嫌疑 → rc=2」保护。该保护是**正确的生产行为**
  #    （解析器啥也没读到不能当通过），所以这里修的是 fixture，不是闸门。
  local t="$1/crates"
  mk_pkg "$t/quill-store" quill-store
  mk_pkg "$t/quill-adapters" quill-adapters
  mk_pkg "$t/quill-wiki" quill-wiki
  printf '\n[dependencies]\nquill-adapters = { path = "../quill-adapters" }\n' >> "$t/quill-wiki/Cargo.toml"
  printf '\n[dev-dependencies]\nquill-store = { path = "../quill-store" }\n' >> "$t/quill-adapters/Cargo.toml"
}
b_commented_pub() { # 边界③：注释里的 unreachable_pub 不是代码
  local t="$1/crates"
  mk_pkg "$t/quill-adapters" quill-adapters
  mk_pkg "$t/quill-wiki" quill-wiki
  mk_pkg "$t/quill-store" quill-store
  printf '\n[dependencies]\nquill-adapters = { path = "../quill-adapters" }\n' >> "$t/quill-wiki/Cargo.toml"
  {
    echo '// 禁止 unreachable_pub：可见性须收敛（注释中出现该词）'
    echo '/* 历史上曾用 unreachable_pub，这里只是说明 */
    pub struct T;'
    echo 'pub fn ok() {}'
  } > "$t/quill-store/src/lib.rs"
  printf '\n[dev-dependencies]\nquill-testkit = { path = "../quill-testkit" }\n' >> "$t/quill-store/Cargo.toml"
}

echo "G40 自检：$(basename "$GATE")"
echo
echo "第①组：违规必须变红（证明闸门能拦住）"
run_case "G40-a  quill-adapters → quill-store"        1 'rule G40-a' b_adapters_to_store
run_case "G40-a  quill-adapters 自环"                 1 'rule G40-a' b_adapters_selfloop
run_case "G40-b  quill-wiki → quill-agent"            1 'rule G40-b' b_wiki_to_agent
run_case "G40-c  crates.io 依赖 goose"                 1 'rule G40-c' b_goose_cratesio
run_case "G40-c  vendor path 依赖 goose"              1 'rule G40-c' b_goose_vendor_path
run_case "G40-d  非 server crate 出现 unreachable_pub" 1 'rule G40-d' b_unreachable_pub

echo
echo "第②组：干净输入必须放行（证明不误报 —— 缺这组就是半个假闸门）"
run_case "干净 crates 树（含 server）"                  0 '' b_clean

echo
echo "第②b组：恒绿保护 ——「0 与未检查必须可区分」"
run_case "全树零依赖 → 恒绿嫌疑须判无法判定"           2 '▲' b_empty_tree

echo
echo "第③组：边界放行（契约允许 / 非代码）"
run_case "边界① agent → wiki（契约**允许**）"          0 '' b_agent_to_wiki
run_case "边界② 仅 [dev-dependencies] 含 quill-*"      0 '' b_dev_deps_only
run_case "边界③ 注释里出现 unreachable_pub 三个字"      0 '' b_commented_pub

echo
echo "第④组：--falsify 自证（rc=1 + SELF-TEST OK）"
N=$((N + 1))
fout="$(bash "$GATE" --falsify 2>&1)"; frc=$?
if (( frc == 1 )) && printf '%s' "$fout" | grep -q 'SELF-TEST OK' \
   && printf '%s' "$fout" | grep -q 'rule G40-a'; then
  PASS=$((PASS + 1))
  printf '  %s✓%-2d%s %-46s rc=%s + SELF-TEST OK\n' "$GREEN" "$N" "$RESET" "--falsify 判红且带规则 ID" "$frc"
else
  FAILED+=("[--falsify] rc=$frc，期望 1 + SELF-TEST OK")
  printf '  %s✗%-2d%s %-46s rc=%s\n' "$RED" "$N" "$RESET" "--falsify" "$frc"
  printf '%s\n' "$fout" | sed 's/^/        /' | head -20
fi

echo
echo "第⑤组：误报反向断言（同一棵树下，豁免文件之外仍须判红）"
N=$((N + 1))
t="$TMPROOT/revassert"
mk_pkg "$t/crates/quill-store" quill-store
mk_pkg "$t/crates/quill-wiki" quill-wiki
# 豁免区：fixtures/ 下的故意违规样本 → 不参与 G40-d 扫描
mkdir -p "$t/crates/quill-store/fixtures"
printf 'unreachable_pub fn fixture_sample() {}\n' > "$t/crates/quill-store/fixtures/bad.rs"
# 豁免区之外：真源码 → 必须仍判红
printf 'unreachable_pub fn leaky() {}\n' > "$t/crates/quill-store/src/leak.rs"
rout="$(bash "$GATE" "$t/crates" 2>&1)"; rrc=$?
if (( rrc == 1 )) && printf '%s' "$rout" | grep -q 'rule G40-d'; then
  PASS=$((PASS + 1))
  printf '  %s✓%-2d%s %-46s rc=%s\n' "$GREEN" "$N" "$RESET" "fixtures/ 外仍判红（豁免未变宽）" "$rrc"
  printf '       %s\n' "$(printf '%s' "$rout" | grep -E '已豁免 fixtures' | head -1)"
else
  FAILED+=("[豁免反向断言] rc=$rrc，期望 1 + rule G40-d")
  printf '  %s✗%-2d%s %-46s rc=%s\n' "$RED" "$N" "$RESET" "fixtures/ 外仍判红" "$rrc"
fi

echo
echo "第⑥组：鉴别力故障注入 —— 闸门被改坏时 --falsify 必须【无法自证】"
# 🔴 依据铁律十九：判据必须取返回值。只看输出字样的话，这个破损版会**骗过**自证
#    —— 它照样打印 "✗ rule G40-a: ..."，只是把 return 1 改成了 return 0。
#    正确行为：rc=2 + SELF-TEST FAILED（= 检测逻辑失效），而不是 rc=1。
N=$((N + 1))
broken="$TMPROOT/gate-broken.sh"
cp "$GATE" "$broken"
sed -i 's/^    return 1$/    return 0/' "$broken"
bcount=$(grep -c '^    return 0$' "$broken")
bout="$(bash "$broken" --falsify 2>&1)"; brc=$?
if (( brc == 2 )) && printf '%s' "$bout" | grep -q 'SELF-TEST FAILED' \
   && ! printf '%s' "$bout" | grep -q 'SELF-TEST OK'; then
  PASS=$((PASS + 1))
  printf '  %s✓%-2d%s %-46s rc=2 + SELF-TEST FAILED\n' "$GREEN" "$N" "$RESET" "破损闸门（return 1→0）无法自证" "$brc"
  printf '       %s\n' "$(printf '%s' "$bout" | grep 'SELF-TEST FAILED' | head -1)"
  if (( bcount == 0 )); then
    printf '       %s⚠ 故障注入未生效（return 1 未替换）——本用例无效%s\n' "$YELLOW" "$RESET"
  fi
else
  FAILED+=("[故障注入] rc=$brc，期望 2 + SELF-TEST FAILED（破损闸门骗过了自证 = 判据取错了返回值）")
  printf '  %s✗%-2d%s %-46s rc=%s\n' "$RED" "$N" "$RESET" "破损闸门无法自证" "$brc"
  printf '%s\n' "$bout" | sed 's/^/        /' | head -6
fi

echo
TOTAL=$((N))
if (( ${#FAILED[@]} )); then
  printf '%s❌ G40 自检失败：%d/%d 项未通过%s\n' "$RED" "${#FAILED[@]}" "$TOTAL" "$RESET" >&2
  for f in "${FAILED[@]}"; do printf '   %s\n' "$f" >&2; done
  exit 1
fi
printf '%s✓ G40 自检通过：%d/%d（含 1 正向放行 + 3 边界放行 + 1 豁免反向断言）%s\n' \
       "$GREEN" "$PASS" "$TOTAL" "$RESET"
printf '%s\n' "  「违规变红」与「合法放行」两侧都被确认 —— 闸门既能拦住，也不误伤。"
exit 0
