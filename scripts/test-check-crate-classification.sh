#!/bin/bash
# ==============================================================================
# check-crate-classification.sh 闸门自检（规则 9）
#
# 核心：证明「反向断言」真的会红 —— 否则豁免就是无人看管的空白（第 1 类假闸门）。
# ==============================================================================
set -uo pipefail
# ⚠️ 可移植性（2026-10-05 qa-engineer 修正）：此前 GATE 硬编码为**本机绝对路径**
#    /mnt/d/96_CoderWorld/quill/... → CI 干净 checkout 上闸门**不可达**，
#    而"不可达"极易被误读成"通过"（第 1 类假闸门）。
#    改为：可用 GATE_SELFTEST_GATE 覆盖，缺省由脚本自身位置推算仓库根。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="${GATE_SELFTEST_GATE:-$ROOT/scripts/check-crate-classification.sh}"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
pass=0; fail=0

# 前置：闸门必须存在且语法正确，否则全部用例都会拿到无意义的 127
[ -f "$GATE" ] || { printf '\033[31m▲ 无法判定：闸门脚本不存在：%s\033[0m\n' "$GATE" >&2
                    printf '  ⚠️ 未判定 ≠ 通过。\n' >&2; exit 2; }
bash -n "$GATE" 2>/dev/null || { printf '\033[31m▲ 无法判定：闸门语法错误：%s\033[0m\n' "$GATE" >&2
                               printf '  ⚠️ 未判定 ≠ 通过。\n' >&2; exit 2; }

expect() { local want="$1" name="$2" dir="$3" got
  bash "$GATE" "$dir" >/dev/null 2>&1; got=$?
  if [ "$got" = "$want" ]; then printf '  \033[32m✓\033[0m %-48s 退出码=%s\n' "$name" "$got"; pass=$((pass+1))
  else printf '  \033[31m✗\033[0m %-48s 退出码=%s（期望 %s）\n' "$name" "$got" "$want"; fail=$((fail+1)); fi
}

mk() { mkdir -p "$1"; }

# --- 1. 合规：testkit 只在 dev-dependencies ------------------------------
mk "$T/ok"
mk "$T/ok/quill-testkit"
cat > "$T/ok/quill-testkit/Cargo.toml" <<'EOF'
[package]
name = "quill-testkit"
publish = false
EOF
mk "$T/ok/quill-wiki"
cat > "$T/ok/quill-wiki/Cargo.toml" <<'EOF'
[package]
name = "quill-wiki"
[dependencies]
serde = "1"
[dev-dependencies]
quill-testkit = { path = "../quill-testkit" }
EOF

# --- 2. ★反向断言：产品把 testkit 放进 [dependencies] → 必须红 -----------
mk "$T/bad_dep"
mk "$T/bad_dep/quill-testkit"
cat > "$T/bad_dep/quill-testkit/Cargo.toml" <<'EOF'
[package]
name = "quill-testkit"
publish = false
EOF
mk "$T/bad_dep/quill-agent"
cat > "$T/bad_dep/quill-agent/Cargo.toml" <<'EOF'
[package]
name = "quill-agent"
[dependencies]
quill-testkit = { path = "../quill-testkit" }
EOF

# --- 3. 未知分类的目录 → 必须红（不静默放过）---------------------------
mk "$T/unknown"
mk "$T/unknown/quill-mystery"
cat > "$T/unknown/quill-mystery/Cargo.toml" <<'EOF'
[package]
name = "quill-mystery"
EOF

# --- 4. ★反向断言：testkit 缺 publish = false → 必须红 -----------------
mk "$T/no_publish"
mk "$T/no_publish/quill-testkit"
cat > "$T/no_publish/quill-testkit/Cargo.toml" <<'EOF'
[package]
name = "quill-testkit"
EOF

# --- 5. ★不误报：[dev-dependencies] 里的 testkit 不能被误判 ---------------
mk "$T/nofalse"
mk "$T/nofalse/quill-testkit"
cat > "$T/nofalse/quill-testkit/Cargo.toml" <<'EOF'
[package]
name = "quill-testkit"
publish = false
EOF
mk "$T/nofalse/quill-backup"
cat > "$T/nofalse/quill-backup/Cargo.toml" <<'EOF'
[package]
name = "quill-backup"
[dev-dependencies]
quill-testkit = { path = "../quill-testkit" }
# 注释里提到 quill-testkit 不该被误判
EOF

# --- 6. ★封住新增：真实三层清单（13 个）应全绿 --------------------------
# 复刻 crates/ 的 13 个目录，其中 quill-bridge 已于 2026-10-05 归产品层。
# 作用：证明加入 quill-bridge **没有**把闸门放宽成「什么目录都放行」。
mk "$T/real13"
for c in quill-adapters quill-agent quill-backup quill-bridge quill-control \
         quill-domain quill-ext-hub quill-server quill-store quill-upgrade \
         quill-wiki quill-xtask; do
  mk "$T/real13/$c"
  printf '[package]\nname = "%s"\n' "$c" > "$T/real13/$c/Cargo.toml"
done
mk "$T/real13/quill-testkit"
printf '[package]\nname = "quill-testkit"\npublish = false\n' > "$T/real13/quill-testkit/Cargo.toml"

# --- 7. ★反向断言：清单之外新增第 14 个 crate → 必须红 ------------------
# 🔴 铁律三十四的防线：白名单若只「列允许项」而不封住新增，
#    将来新建一个没分类的 crate 就会静默逃过 → 恒绿闸门。
#    此用例证明：补 quill-bridge 进清单**没有**削弱「未知分类 → 必红」。
mk "$T/real13_plus_new"
cp -r "$T/real13/." "$T/real13_plus_new/"
mk "$T/real13_plus_new/quill-brandnew"
printf '[package]\nname = "quill-brandnew"\n' > "$T/real13_plus_new/quill-brandnew/Cargo.toml"

echo "########## 规则 9 闸门自检 ##########"
expect 0 "合规：testkit 仅 dev-dependencies"          "$T/ok"
expect 1 "★反向断言：testkit 进 [dependencies]"      "$T/bad_dep"
expect 1 "未知分类目录（不静默放过）"               "$T/unknown"
expect 1 "★testkit 缺 publish = false"               "$T/no_publish"
expect 0 "★不误报：dev-dependencies + 注释"         "$T/nofalse"
expect 0 "★真实 13 目录清单（含 quill-bridge）"     "$T/real13"
expect 1 "★封住新增：清单外第 14 个 crate 仍判红"   "$T/real13_plus_new"

bash "$GATE" "$T/nope" >/dev/null 2>&1; got=$?
if [ "$got" = "2" ]; then printf '  \033[32m✓\033[0m %-48s 退出码=2\n' "目录不存在"; pass=$((pass+1))
else printf '  \033[31m✗\033[0m %-48s 退出码=%s（期望 2）\n' "目录不存在" "$got"; fail=$((fail+1)); fi

echo
echo "########## 对照：naive 做法的失效 ##########"
echo "  naive：只检查 crates/ 目录是否都在白名单里"
echo "  漏报场景：quill-agent 的 [dependencies] 里塞了 quill-testkit"
echo "            → 目录名合法（是产品 crate），naive 检查通过"
echo "            → 但测试代码已进 release 二进制"
echo "  本脚本用 awk 精确提取 [dependencies] 段（不含 dev-dependencies）"

echo
echo "########## 结果：通过 $pass / 失败 $fail ##########"
# 🔴 收尾必须带 exit（2026-10-05 qa-engineer 修正，第 2 类假成功）：
#    此前只有一行 `[ "$fail" = "0" ] && echo PASS || echo FAIL`，
#    **末尾没有 exit** → 无论通过还是失败脚本都退出 0 →
#    run-gate-selftest.sh 按返回值分类 → G27 在结构上**永远判不了红**。
#    所有此前的「自检全绿」报告都不成立。
printf 'SELFTEST_%s\n' "$([ "$fail" -eq 0 ] && echo PASS || echo FAIL)"
[ "$fail" -eq 0 ] || exit 1
exit 0
