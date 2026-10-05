#!/bin/bash
# ==============================================================================
# check-boundary-singletons.sh 闸门自检
#
# 关键：不误报用例比变红用例更重要。
# 规则类闸门最常见的失效是**误报**——包括 xtask 自己写着这些符号而把自己判违规。
# ==============================================================================
set -uo pipefail
# 🔴 闸门路径**不得**硬编码某台开发机的 WSL 挂载路径（主理人 2026-10-05 实测
#    发现本仓库多支自检脚本有此问题）：CI 的干净 checkout 上没有该路径，会让
#    自检**永远找不到闸门** —— 而"找不到闸门"若无判据就等于恒绿（第 1 类假闸门）。
#    改为：优先环境变量覆盖 → 否则由**脚本自身位置**推算仓库根。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="${QUILL_G26_GATE:-$ROOT/scripts/check-boundary-singletons.sh}"
[[ -f "$GATE" ]] || { printf '✗ 闸门不存在：%s（无法判定，不计入通过）\n' "$GATE" >&2; exit 2; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
pass=0; fail=0

expect() { # expect <期望码> <说明> <目录>
  local want="$1" name="$2" dir="$3" got
  bash "$GATE" "$dir" >/dev/null 2>&1; got=$?
  if [ "$got" = "$want" ]; then
    printf '  \033[32m✓\033[0m %-44s 退出码=%s\n' "$name" "$got"; pass=$((pass+1))
  else
    printf '  \033[31m✗\033[0m %-44s 退出码=%s（期望 %s）\n' "$name" "$got" "$want"; fail=$((fail+1))
  fi
}

# 1. 干净：全是注入式构造
mkdir -p "$T/clean"
cat > "$T/clean/lib.rs" <<'EOF'
// 用注入式构造函数，禁止 static 全局入口
use goose::config::Config;
use goose::session::SessionManager;
pub fn build(p: &Path) -> anyhow::Result<()> {
    let _cfg = Config::new(p, "quill")?;
    let _sm = SessionManager::new(p.to_path_buf());
    Ok(())
}
EOF

# 2. 违规：Config::global()
mkdir -p "$T/bad_global"
cat > "$T/bad_global/lib.rs" <<'EOF'
pub fn f() {
    let cfg = Config::global();   // ← 违规
}
EOF

# 3. 违规：SessionManager::instance()
mkdir -p "$T/bad_instance"
cat > "$T/bad_instance/lib.rs" <<'EOF'
pub fn g() {
    let sm = SessionManager::instance();   // ← 违规
}
EOF

# 4. ★不误报：注释 / 字符串里提到这些名字（最常见误报源）
mkdir -p "$T/no_false_positive"
cat > "$T/no_false_positive/lib.rs" <<'EOF'
// 禁止使用 Config::global()，请用 Config::new()
/*
   历史上有人写过 SessionManager::instance()，已改用 new(data_dir)。
   那会导致所有用户共享同一 session 存储 → 静默串数据。
*/
/// 文档：内部曾用 Config::global() 读取配置，现已移除。
pub fn h() { let _ = 1; }
EOF

# 5. ★不误报：vendor/ 下的上游代码（上游必然有这些调用，不能算我们违规）
mkdir -p "$T/with_vendor/vendor/goose"
cat > "$T/with_vendor/clean.rs" <<'EOF'
pub fn k() { let _ = 2; }
EOF
cat > "$T/with_vendor/vendor/goose/upstream.rs" <<'EOF'
pub fn u() { let _cfg = Config::global(); }
EOF

# 6. ★不误报：显式豁免文件（fals.rs 存"已知坏样本"字符串）→ 不误报
mkdir -p "$T/selfexempt/quill-testkit/src"
cat > "$T/selfexempt/quill-testkit/src/fals.rs" <<'EOF'
// FALS 自检辅助：存放"已知坏样本"
pub const SAMPLE: &str = "fn f() { let c = Config::global(); }";
EOF

# 7. 🔴反向断言：豁免不能扩大到其他文件 —— 同树下的产品代码仍须判红
#    （单独目录，避免与用例 6 混在同一个 crates/ 里）
mkdir -p "$T/exempt_leak/quill-testkit/src" "$T/exempt_leak/quill-wiki"
cat > "$T/exempt_leak/quill-testkit/src/fals.rs" <<'EOF'
pub const SAMPLE: &str = "Config::global()";
EOF
cat > "$T/exempt_leak/quill-wiki/leak.rs" <<'EOF'
pub fn bad() { let _c = Config::global(); }
EOF

echo "########## 规则 7~8 闸门自检 ##########"
expect 0 "干净：注入式构造"                        "$T/clean"
expect 1 "违规：Config::global()"                 "$T/bad_global"
expect 1 "违规：SessionManager::instance()"       "$T/bad_instance"
expect 0 "★不误报：注释/文档提到这些名字"          "$T/no_false_positive"
expect 0 "★不误报：vendor/ 上游代码"              "$T/with_vendor"
expect 0 "★不误报：显式豁免文件 fals.rs"          "$T/selfexempt"
expect 1 "🔴反向：豁免外的文件仍须判红"            "$T/exempt_leak"

# 6a. 🔴🔴 零命中路径（主理人 2026-10-05 实测缺陷的直接回归用例）
#      背景：原写法 `total=$(grep -cE ... || echo 0)` 在**零命中**时
#      得到 "0\n0" → `for ((k=lit;k<total;k++))` 抛 syntax error，
#      而脚本仍 rc=0 报"通过"（错误信号存在但**不进入判定**）。
#      ⚠️ 23 项自检全绿也盖不住它：fixture 树里每个 lib.rs 都含 include!，
#         **零命中分支从未被执行过** ——「测试跑了」≠「测了东西」。
#      本用例构造**完全不含 include!** 的干净 crate，并额外断言 stderr 干净。
mkdir -p "$T/zero_inc/crates/q/src"
cat > "$T/zero_inc/crates/q/src/lib.rs" <<'EOF'
// 本文件**故意不含任何 include!**，且注入式构造、无被禁符号。
use goose::config::Config;
pub fn ok(p: &Path) -> anyhow::Result<()> {
    let _cfg = Config::new(p, "quill")?;
    Ok(())
}
EOF
expect 0 "★零命中：无 include! 的干净 crate"        "$T/zero_inc/crates"

# 6a-2. ★★ 零命中时 **stderr 必须无任何噪音**（判读原则）
#      闸门脚本自己报的 shell 错误（syntax error / command not found）
#      即使 rc=0 也是**实现有 bug** 的信号，必须让自检看见它。
#      ⚠️ 上一条 expect() 把 stderr 丢进 /dev/null，正是噪音被掩盖的根源。
z_out=$(bash "$GATE" "$T/zero_inc/crates" 2>&1 >/dev/null)
z_noise=$(printf '%s\n' "$z_out" \
  | grep -nE 'syntax error|command not found|: line [0-9]+:|unbound variable|integer expression' || true)
if [ -z "$z_noise" ]; then
  printf '  \033[32m✓\033[0m %-44s stderr 无噪音\n' "★零命中：无 shell 错误噪音"; pass=$((pass+1))
else
  printf '  \033[31m✗\033[0m %-44s stderr 有噪音\n' "★零命中：无 shell 错误噪音"; fail=$((fail+1))
  printf '%s\n' "$z_noise" | head -4 | sed 's/^/        /'
fi

# 6a-3. ★★ 真实形状回归：多文件、**全部**零命中 + 含 vendor/ 子目录
#      这一条复刻主理人在真实 crates/ 上看到的 22 次重复报错
#      （真实 crates/ 里大部分 .rs 都不含 include!）。
mkdir -p "$T/zero_many/crates/a/src" "$T/zero_many/crates/b/src" "$T/zero_many/crates/vendor/up"
for i in 1 2 3 4 5 6; do
  printf 'pub fn f%d() -> u8 { %d }\n' "$i" "$i" > "$T/zero_many/crates/a/src/m$i.rs"
done
printf 'pub fn g() -> u8 { 1 }\n' > "$T/zero_many/crates/b/src/lib.rs"
printf 'pub fn u() -> u8 { 2 }\n' > "$T/zero_many/crates/vendor/up/up.rs"
m_out=$(bash "$GATE" "$T/zero_many/crates" 2>&1 >/dev/null)
m_noise=$(printf '%s\n' "$m_out" \
  | grep -cE 'syntax error|command not found|: line [0-9]+:' || true)
bash "$GATE" "$T/zero_many/crates" >/dev/null 2>&1; m_rc=$?
if [ "$m_rc" = "0" ] && [ "$m_noise" = "0" ]; then
  printf '  \033[32m✓\033[0m %-44s rc=0 且噪音 %s 条\n' "★零命中×9 文件：rc=0 且零噪音" "$m_noise"; pass=$((pass+1))
else
  printf '  \033[31m✗\033[0m %-44s rc=%s 噪音 %s 条（期望 0/0）\n' "★零命中×9 文件" "$m_rc" "$m_noise"; fail=$((fail+1))
fi

# 7b. ★G26-3：include! 逃出 crates/ → 必须红
#     注意层级：从 <root>/crates/q/src/ 出发，../../../ 才到 <root>（= crates 外）
mkdir -p "$T/inc_esc/crates/q/src" "$T/inc_esc/elsewhere"
cat > "$T/inc_esc/crates/q/src/lib.rs" <<'EOF'
mod inner;
include!("../../../elsewhere/hidden.rs");
EOF
printf 'pub fn f() -> &%sstatic str { "Config::global()" }\n' "'" > "$T/inc_esc/elsewhere/hidden.rs"
expect 1 "G26-3: include! 逃出 crates/"           "$T/inc_esc/crates"

# 7b-2. ★目标**在 crates 外但路径只到 crates/**（层级差一级）→ 必须放行
#      这是 BND-08 原版的形态：../../elsewhere 从 src/ 出发 = crates/elsewhere，仍在界内
mkdir -p "$T/inc_inside/crates/q/src" "$T/inc_inside/crates/elsewhere"
cat > "$T/inc_inside/crates/q/src/lib.rs" <<'EOF'
include!("../../elsewhere/hidden.rs");
EOF
echo 'pub fn h() -> u8 { 1 }' > "$T/inc_inside/crates/elsewhere/hidden.rs"
expect 0 "G26-3: 差一级（仍在 crates 内）"        "$T/inc_inside/crates"

# 7c. ★不误报：include! 目标在 crates/ 内且无违规 → 放行
#      （include! 本身是合法组织方式，不该被一刀切禁掉）
mkdir -p "$T/inc_ok/crates/q/src"
cat > "$T/inc_ok/crates/q/src/lib.rs" <<'EOF'
mod shared;
include!("../shared.rs");
pub fn ok() -> u8 { 1 }
EOF
echo 'pub fn helper() -> u8 { 2 }' > "$T/inc_ok/crates/q/src/shared.rs"
expect 0 "G26-3 不误报：include! 在 crates/ 内"  "$T/inc_ok/crates"

# 7c-2. ★★ 路径不变性（qa-engineer 报告的缺陷根因）
#      **同一份代码，仅调用路径不同 → 结果必须完全相同**
#      原缺陷：$CRATES 为相对值时，G26-3 误报合法 include!
mkdir -p "$T/pathvar/crates/q/src"
cat > "$T/pathvar/crates/q/src/lib.rs" <<'EOF'
mod shared;
include!("./shared.rs");
EOF
echo 'pub fn helper() -> u8 { 2 }' > "$T/pathvar/crates/q/src/shared.rs"

r_abs=$(bash "$GATE" "$T/pathvar/crates" >/dev/null 2>&1; echo $?)
r_rel=$(cd "$T" && bash "$GATE" pathvar/crates >/dev/null 2>&1; echo $?)
r_dot=$(cd "$T/pathvar" && bash "$GATE" ./crates >/dev/null 2>&1; echo $?)
if [ "$r_abs" = "$r_rel" ] && [ "$r_rel" = "$r_dot" ] && [ "$r_abs" = "0" ]; then
  printf '  \033[32m✓\033[0m %-44s 绝对/相对/点号 均=%s\n' "G26-3 路径不变性（合法 include!）" "$r_abs"; pass=$((pass+1))
else
  printf '  \033[31m✗\033[0m %-44s abs=%s rel=%s dot=%s（应全为 0）\n' "G26-3 路径不变性" "$r_abs" "$r_rel" "$r_dot"; fail=$((fail+1)); fi

# 7c-3. ★路径不变性（违规侧）：逃逸的 include! 在三种调用下都必须报红
mkdir -p "$T/pathvar2/crates/q/src" "$T/pathvar2/elsewhere"
cat > "$T/pathvar2/crates/q/src/lib.rs" <<'EOF'
include!("../../../elsewhere/hidden.rs");
EOF
echo 'pub fn e() -> u8 { 1 }' > "$T/pathvar2/elsewhere/hidden.rs"
r1=$(bash "$GATE" "$T/pathvar2/crates" >/dev/null 2>&1; echo $?)
r2=$(cd "$T" && bash "$GATE" pathvar2/crates >/dev/null 2>&1; echo $?)
r3=$(cd "$T/pathvar2" && bash "$GATE" ./crates >/dev/null 2>&1; echo $?)
if [ "$r1" = "$r2" ] && [ "$r2" = "$r3" ] && [ "$r1" = "1" ]; then
  printf '  \033[32m✓\033[0m %-44s 绝对/相对/点号 均=%s\n' "G26-3 路径不变性（逃逸）" "$r1"; pass=$((pass+1))
else
  printf '  \033[31m✗\033[0m %-44s abs=%s rel=%s dot=%s（应全为 1）\n' "G26-3 路径不变性（逃逸）" "$r1" "$r2" "$r3"; fail=$((fail+1)); fi

# 7c-4. ★目标目录**不存在**但仍逃出边界 → 必须报红
#      这是词法归约与 cd 法的关键差异：cd 法会失败→跳过→漏报。
#      ../../../../ 会弹到 / 之外（目标根本不存在），仍必须判红。
mkdir -p "$T/outdir/crates/q/src"
cat > "$T/outdir/crates/q/src/lib.rs" <<'EOF'
include!("../../../../nonexistent-root/hidden.rs");
EOF
expect 1 "G26-3: 目标不存在但逃出边界"       "$T/outdir/crates"

# 7c-5. ★路径含插值 → 明确告警，不静默放过
mkdir -p "$T/interp/crates/q/src"
cat > "$T/interp/crates/q/src/lib.rs" <<'EOF'
include!(concat!(env!("OUT_DIR"), "/gen.rs"));
EOF
o=$(bash "$GATE" "$T/interp/crates" 2>&1 >/dev/null || true)
if printf '%s' "$o" | grep -q "无法静态解析"; then
  printf '  \033[32m✓\033[0m %-44s 提示无法解析而非静默\n' "G26-3 含插值的路径"; pass=$((pass+1))
else
  printf '  \033[31m✗\033[0m %-44s 未提示无法解析\n' "G26-3 含插值的路径"; fail=$((fail+1)); fi

# 7d. ★G26-4：符号链接指向 crates/ 外 → 必须红
mkdir -p "$T/sym_esc/real_src" "$T/sym_esc/crates"
printf 'pub fn f() -> &%sstatic str { "Config::global()" }\n' "'" > "$T/sym_esc/real_src/lib.rs"
if ln -s ../real_src "$T/sym_esc/crates/linked" 2>/dev/null; then
  # 🔴 必须验证"真的是符号链接"，不能只看 ln 的退出码
  #   实测（qa-engineer，2026-10-04）：**Windows Git Bash 下 `ln -s` 会创建
  #   一个真实的目录副本并返回 0** —— 命令"成功"不等于目的达成。
  #   若不验证，`ln` 成功但实际是普通目录 → 下面的"期望判红"必然失败，
  #   而根因是环境不支持，**不是闸门坏了** → 会产生无法解释的红。
  if [[ -L "$T/sym_esc/crates/linked" ]]; then
    expect 1 "G26-4: 符号链接指向 crates/ 外"       "$T/sym_esc/crates"
    # 反向：软链目标**在 crates/ 内**（注意路径要写成 ./inner，不能是 ../inner）
    mkdir -p "$T/sym_ok/crates/inner"
    echo 'pub fn f() -> u8 { 1 }' > "$T/sym_ok/crates/inner/lib.rs"
    ln -s ./inner "$T/sym_ok/crates/linked" 2>/dev/null
    if [[ -L "$T/sym_ok/crates/linked" ]]; then
      expect 0 "G26-4 不误报：软链在 crates/ 内"     "$T/sym_ok/crates"
    else
      printf '  \033[33m—\033[0m %-44s 无法创建真软链（跳过）\n' "G26-4 不误报"
    fi
  else
    # ⚠️ 明确标记为「无法判定」，不计入通过（第 6/7 类失效）
    #    ln 返回 0 但建出的是普通目录副本 → 目的未达成
    printf '  \033[33m▲\033[0m %-44s ln 返回 0 但非真链接 → 无法判定\n' "G26-4: 符号链接"
    printf '     %s\n' "     （Windows Git Bash 会复制目录；需开发者模式）—— 不计入通过"
  fi
else
  printf '  \033[33m▲\033[0m %-44s 无法创建软链 → 无法判定\n' "G26-4: 符号链接"
  printf '     %s\n' "     不计入通过（对齐 qa runner 的「无法判定」语义）"
fi

# 7e. ★G26-5：build.rs 含被禁符号 → 必须红
mkdir -p "$T/brs/crates/q/src"
echo 'pub fn ok() -> u8 { 1 }' > "$T/brs/crates/q/src/lib.rs"
cat > "$T/brs/crates/q/build.rs" <<'EOF'
fn main() {
    let g = r#" pub fn s() -> &'static str { "Config::global()" } "#;
    std::fs::write("src/generated.rs", g).unwrap();
}
EOF
expect 1 "G26-5: build.rs 含被禁符号"              "$T/brs/crates"

# 7f. ★G26-5 不误报：干净 build.rs → 放行
mkdir -p "$T/brs_ok/crates/q/src"
echo 'pub fn ok() -> u8 { 1 }' > "$T/brs_ok/crates/q/src/lib.rs"
echo 'fn main() { println!("cargo:rerun-if-changed=build.rs"); }' > "$T/brs_ok/crates/q/build.rs"
expect 0 "G26-5 不误报：干净 build.rs"              "$T/brs_ok/crates"

# 6. 目录不存在 → 2
bash "$GATE" "$T/nope" >/dev/null 2>&1; got=$?
if [ "$got" = "2" ]; then printf '  \033[32m✓\033[0m %-44s 退出码=2\n' "目录不存在"; pass=$((pass+1))
else printf '  \033[31m✗\033[0m %-44s 退出码=%s（期望 2）\n' "目录不存在" "$got"; fail=$((fail+1)); fi

# 8. ★输出契约：违规时 stderr 必须含 `rule <ID>`，且只含那一个
#    （G08 runner 依赖此格式做精确断言；这是接口契约，必须自检）
out=$(bash "$GATE" "$T/bad_global" 2>&1 >/dev/null || true)
if printf '%s' "$out" | grep -q "rule G26-1" && ! printf '%s' "$out" | grep -q "rule G26-2"; then
  printf '  \033[32m✓\033[0m %-44s 含 rule G26-1 且不含 G26-2\n' "H. 输出契约：rule ID 精确指向"; pass=$((pass+1))
else
  printf '  \033[31m✗\033[0m %-44s 输出=[%s]\n' "H. 输出契约：rule ID 精确指向" "$(printf '%s' "$out" | head -1)"; fail=$((fail+1)); fi

# 8b. ★fixtures 豁免：故意违规的 fixture 目录不得让真实扫描变红
mkdir -p "$T/fx/quill-testkit/fixtures/bndX/src"
cat > "$T/fx/quill-testkit/fixtures/bndX/src/bad.rs" <<'EOF'
pub fn bad() { let s = "Config::global()"; let _ = s; }
EOF
expect 0 "★不误报：fixtures/（故意违规样本）" "$T/fx"

# 8c. 🔴反向：fixtures 之外仍须判红（豁免没扩大到整个 crates/）
mkdir -p "$T/fx2/quill-testkit/fixtures/bndX/src" "$T/fx2/quill-wiki"
cat > "$T/fx2/quill-testkit/fixtures/bndX/src/bad.rs" <<'EOF'
pub const S: &str = "Config::global()";
EOF
cat > "$T/fx2/quill-wiki/leak.rs" <<'EOF'
pub fn bad() { let _c = Config::global(); }
EOF
expect 1 "🔴反向：fixtures 外仍须判红" "$T/fx2"

# 9. ★豁免可见性：有豁免时必须打印豁免数量（豁免不能是无人看管的空白）
#    构造：豁免文件(fals.rs) + 一个干净文件 → 应豁免 1 个、整体通过
mkdir -p "$T/vis/quill-testkit/src"
cat > "$T/vis/quill-testkit/src/fals.rs" <<'EOF'
pub const S: &str = "Config::global()";
EOF
cat > "$T/vis/quill-testkit/src/lib.rs" <<'EOF'
pub fn clean() -> u8 { 1 }
EOF
out2=$(bash "$GATE" "$T/vis" 2>&1 >/dev/null || true)
if printf '%s' "$out2" | grep -qE "已豁免 [0-9]+ 个"; then
  n=$(printf '%s' "$out2" | grep -oE "已豁免 [0-9]+" | head -1)
  printf '  \033[32m✓\033[0m %-44s %s\n' "I. 豁免范围可见" "$n"; pass=$((pass+1))
else
  printf '  \033[31m✗\033[0m %-44s 输出=[%s]\n' "I. 豁免范围可见" "$(printf '%s' "$out2" | head -2)"; fail=$((fail+1)); fi

# 7. 对照：naive 写法的失效 —— 若不排除 vendor，上游 232 处会全红
naive=$(grep -rn --include='*.rs' -E "Config::global[[:space:]]*\(" "$T/with_vendor" 2>/dev/null | wc -l | tr -d ' ')
echo
echo "########## 对照 ##########"
echo "  naive（不排除 vendor）命中 $naive 处 → 会把上游代码判为我们违规"
echo "  本脚本用 --exclude-dir=vendor 避免误报"

echo
# 🔴🔴 收尾必须**真的 exit**（主理人 2026-10-05 实测缺陷）：
#   原收尾 `[ "$fail" = "0" ] && echo PASS || echo FAIL` **没有 exit**，
#   无论成败都退出 0 ⇒ run-gate-selftest.sh 按返回值分类 ⇒
#   G26 的 self_test **永远被判成 pass**，这道闸门结构上无法变红。
#   （决定性实验：注入真实违规后「通过 1 / 失败 25 + SELFTEST_FAIL」，
#     但自检退出码仍是 0。）
#   正确形态：对齐 test-quill-doctor-roaming.sh —— 失败 exit 1，通过 exit 0。
printf '\n########## 结果：通过 %d / 失败 %d ##########\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || { echo "SELFTEST_FAIL"; exit 1; }
echo "SELFTEST_PASS"
exit 0
