#!/usr/bin/env bash
# 门禁本身的门禁：拿合成的 cargo 输出喂 `.wsl-verify-persona.sh` 用的那段判定，
# 确认四种情况的结论都对。
#
# **为什么它入库、而门禁本身不入库**：`.gitignore` 第 69 行把 `/.wsl-*.sh`
# 归为「本地临时验证脚本」，所以门禁脚本本身从来没有进过版本库 ——
# 也就是说那道把关全项目「0 failed」纪律的检查，**从未被第二个人看过一眼**。
# 门禁可以按约定留在本地，但它「必须怎么判」这件事得留在仓库里，
# 否则下一个写门禁的人会再写一次那个恒等于 0 的解析。
#
# **为什么需要这个自测**：旧门禁用 `awk -F'[ ;]' '{f+=$6}'` 数失败，而 "-F'[ ;]'"
# 把 "; " 当成两个分隔符、中间多出一个空字段，失败数其实在 $7 —— 于是 $6 恒为空，
# `failed` 恒等于 0。那个门禁从写出来那天起就报不出任何非零失败数，跑多少个红
# 它都印「failed=0」。这种 bug 不会自己暴露：它只在真有测试失败时才说错话，
# 而那时候人往往正忙着修别的东西。
#
# 跑法（WSL 侧）：wsl bash .scripts/gate-selftest.sh
set -uo pipefail

# —— 与 .wsl-verify-persona.sh 里那段判定逐字一致 ——
decide() {
  LOG="$1"; CARGO_RC="$2"
  read -r PASSED FAILED < <(grep -E '^test result' "$LOG" \
    | awk -F'[ ;]' '{for(i=1;i<NF;i++){if($(i+1)=="passed")p+=$i; if($(i+1)=="failed")f+=$i}} END {print p+0, f+0}')
  echo "  解析出 passed=$PASSED failed=$FAILED cargo_rc=$CARGO_RC"
  if [ "$CARGO_RC" -ne 0 ] && [ "$FAILED" -eq 0 ]; then
    echo "  结论：失败（运行不完整）"; return 1
  fi
  if [ "$CARGO_RC" -ne 0 ] || [ "$FAILED" -gt 0 ]; then
    echo "  结论：失败"; return 1
  fi
  echo "  结论：通过"; return 0
}

T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fail=0

# 场景 1 的正确结论是「通过」（退出 0），其余三个的正确结论都是「失败」（退出非 0）。
# 写反了这个方向，自测就会把「判对」报成「判错」—— 门禁自测自己也可能说谎。
expect_pass() { decide "$1" "$2" || { echo "  !! 这里应当判通过，却判了失败"; fail=1; }; }
expect_fail() { decide "$1" "$2" && { echo "  !! 这里应当判失败，却判了通过"; fail=1; }; return 0; }

echo "场景1：全跑完且全过"
cat >"$T/a.log" <<'EOF'
test result: ok. 152 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.04s
test result: ok. 787 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 41.2s
EOF
expect_pass "$T/a.log" 0

echo "场景2：真的红了 —— 失败数必须被数出来，不能是 0"
cat >"$T/b.log" <<'EOF'
test result: ok. 151 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.00s
test result: FAILED. 151 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.04s
test some::test::it_broke ... FAILED
EOF
expect_fail "$T/b.log" 101
# 除了「判失败」，还要钉住**数字本身**：数不出 1 而只是碰巧判了失败，门禁照样没修好。
# 注意不能写成 `decide ... | grep`：decide 在判失败时退出码非 0，`pipefail` 会把
# 整条管道判成失败，`!` 一反转就成了「没数出来」—— 自测会误报自己。
OUT2="$(decide "$T/b.log" 101)"
printf '%s\n' "$OUT2" | grep -q 'passed=302 failed=1' || {
  echo "  !! 失败数没被数出来，实际输出：$OUT2"
  fail=1
}

echo "场景3：中途 bail，只跑到一半且 0 失败 —— 旧脚本在这里印「TOTAL passed=300 failed=0」"
cat >"$T/c.log" <<'EOF'
test result: ok. 300 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.0s
error: test failed, to rerun pass `-p quill-server --lib`
EOF
expect_fail "$T/c.log" 101

echo "场景4：连 test result 都没打出来就炸了"
printf 'error: could not compile\n' >"$T/d.log"
expect_fail "$T/d.log" 101

# —— 前端 lint 那一道（2026-10-07 接入）——
#
# 为什么要给 lint 也写自测：它是用 `if npx eslint .` 的**退出码**判成败的，
# 而 eslint 的退出码在「只有 warning」时是 0、在「有 error」时才是 1。
# 也就是说，如果哪天 eslint 配置被改坏（比如 rule 全被关掉），这条门禁
# 会变成一个恒真的检查 —— 与本文件开头记的那个 `failed` 恒等于 0 的事故
# 同一类。所以这里拿合成的 eslint 输出，把三种情况都钉住：
# 只有 warning → 通过；有 error → 失败；连 problems 行都没打 → 失败。

# 与 gates.sh 里那段判定同源：退出码说话，输出只用来报给人看。
decide_lint() {
  LOG="$1"; ESLINT_RC="$2"
  echo "  eslint_rc=$ESLINT_RC"
  if [ "$ESLINT_RC" -ne 0 ]; then
    echo "  结论：失败"; return 1
  fi
  echo "  结论：通过"; return 0
}

count_errors() {
  grep -oE '[0-9]+ errors?' "$1" | head -1 | grep -oE '[0-9]+' || echo 0
}

echo "场景5：lint 只有 warning —— 必须判通过（warning 不计入退出码）"
cat >"$T/lint-warn.log" <<'EOF'
/mnt/d/quill/ui/web/src/models/ModelsPage.tsx
  42:14  warning  Fast refresh only works when a file only exports components  react-refresh/only-export-components

✖ 11 problems (0 errors, 11 warnings)
EOF
expect_pass_decide() { decide_lint "$1" "$2" || { echo "  !! 这里应当判通过，却判了失败"; fail=1; }; }
expect_fail_decide() { decide_lint "$1" "$2" && { echo "  !! 这里应当判失败，却判了通过"; fail=1; }; return 0; }
expect_pass_decide "$T/lint-warn.log" 0
# 顺带钉住「warning 数读得出来」：读不出来的话那句「warning N 条」就是死的。
N5="$(count_errors "$T/lint-warn.log")"
[ "$N5" = "0" ] || { echo "  !! eslint 输出里 0 errors 应读出 0，实际读出 $N5"; fail=1; }

echo "场景6：lint 有 error —— 必须判失败，且 0 error 那条不能被当成通过"
cat >"$T/lint-err.log" <<'EOF'
/mnt/d/quill/ui/web/src/chat/ChatPage.tsx
  123:7   error    Calling setState synchronously within an effect  react-hooks/set-state-in-effect

✖ 13 problems (2 errors, 11 warnings)
EOF
expect_fail_decide "$T/lint-err.log" 1

echo "场景7：eslint 自己炸了（配置坏了 / 插件加载失败）—— 不能当成通过"
printf 'Oops! Something went wrong!\n' >"$T/lint-broken.log"
expect_fail_decide "$T/lint-broken.log" 2

echo
if [ "$fail" -eq 0 ]; then
  echo "门禁自测：七个场景全对"
  exit 0
fi
echo "门禁自测：有场景判错了 —— 门禁本身不可信，先修它再谈别的"
exit 1
