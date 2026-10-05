#!/usr/bin/env bash
# ==============================================================================
# check-no-external-refs.sh 的闸门自检
#
# 依据 AGENTS.md「新增闸门必须自检」铁律：
# **未自检的闸门不算闸门** —— 一个从未在"已知坏输入"上验证过变红的校验，
# 与没有校验同等危险（它提供虚假的安全感）。
#
# 本测试证明三件事：
#   1. 含外链的输入 → 变红（退出码 1）
#   2. 干净的产物 → 放行（退出码 0）
#   3. 注释 / schema.org 等白名单 → 不误报
# ==============================================================================
set -uo pipefail
# ⚠️ 可移植性（2026-10-05 qa-engineer 修正）：此前用 `dirname "$0"`，
#    当以裸名（经 PATH 找到）或从其他 cwd 调用时 $0 不含目录 → 路径解析失败 →
#    闸门"不可达"。改用 BASH_SOURCE + 仓库根推算，并允许环境变量覆盖。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="${GATE_SELFTEST_GATE:-$ROOT/scripts/check-no-external-refs.sh}"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

pass=0; fail=0

# 前置：闸门必须存在且语法正确，否则全部用例都会拿到无意义的 127
if [ ! -f "$GATE" ]; then
  printf '  \033[33m▲ 无法判定：闸门脚本不存在：%s\033[0m\n' "$GATE" >&2
  printf '    ⚠️ 未判定 ≠ 通过。\n' >&2; exit 2
fi
if ! bash -n "$GATE" 2>/dev/null; then
  printf '  \033[33m▲ 无法判定：闸门语法错误：%s\033[0m\n' "$GATE" >&2
  printf '    ⚠️ 未判定 ≠ 通过。\n' >&2; exit 2
fi
check() { # check <期望码> <说明>
  local want="$1" name="$2" got
  shift 2
  bash "$GATE" "$@" >/dev/null 2>&1; got=$?
  if [ "$got" = "$want" ]; then
    printf '  \033[32m✓\033[0m %-46s 退出码=%s\n' "$name" "$got"; pass=$((pass+1))
  else
    printf '  \033[31m✗\033[0m %-46s 退出码=%s（期望 %s）\n' "$name" "$got" "$want"; fail=$((fail+1))
  fi
}

# --- 1. 干净产物（应放行）-------------------------------------------------
mkdir -p "$T/clean"
cat > "$T/clean/index.html" <<'EOF'
<!doctype html><html><head>
<link rel="stylesheet" href="/assets/index-a1b2c3.css">
</head><body><div id="root"></div><script src="/assets/index-d4e5f6.js"></script></body></html>
EOF
cat > "$T/clean/app.js" <<'EOF'
const API = "/api/backup/health";
fetch(API).then(r => r.json());
EOF

# --- 2. 含外链的产物（应变红）---------------------------------------------
mkdir -p "$T/dirty"
cat > "$T/dirty/index.html" <<'EOF'
<!doctype html><html><head>
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/antd@5/dist/reset.css">
</head><body><script src="https://unpkg.com/react@19/umd/react.production.min.js"></script></body></html>
EOF

# --- 3. CSS @import 外链（应变红）-----------------------------------------
mkdir -p "$T/css"
cat > "$T/css/main.css" <<'EOF'
@import url("https://fonts.googleapis.com/css2?family=Inter");
@font-face { font-family: Inter; src: url(https://cdn.example.com/inter.woff2); }
EOF

# --- 4. 白名单：注释 / schema.org（不应误报）------------------------------
mkdir -p "$T/clean2"
cat > "$T/clean2/index.html" <<'EOF'
<!doctype html><html lang="zh">
<head>
<!-- <link href="https://example.com/legacy.css"> 历史遗留，已移除 -->
<script type="application/ld+json">{"@context":"https://schema.org"}</script>
<meta property="og:image" content="https://schema.org/og/image.png">
</head><body><script src="/assets/x.js"></script></body></html>
EOF
cat > "$T/clean2/app.js" <<'EOF'
// 旧版本曾用 https://cdn.example.com/legacy.js，现已改为本地打包
/* 许可证说明见 https://opensource.org/licenses/MIT */
export const V = "1.0.0";
EOF

echo "########## check-no-external-refs.sh 闸门自检 ##########"
check 0 "干净产物（相对路径）"              "$T/clean"
check 1 "含 CDN 外链（应变红）"              "$T/dirty"
check 1 "CSS @import + url() 外链（应变红）" "$T/css"
check 0 "白名单：注释/schema.org/许可证"      "$T/clean2"

# 5. 目录不存在 → 退出码 2
bash "$GATE" "$T/nonexistent" >/dev/null 2>&1; got=$?
if [ "$got" = "2" ]; then printf '  \033[32m✓\033[0m %-46s 退出码=2\n' "目录不存在"; pass=$((pass+1))
else printf '  \033[31m✗\033[0m %-46s 退出码=%s（期望 2）\n' "目录不存在" "$got"; fail=$((fail+1)); fi

echo
echo "########## 对照：naive 写法（只看目录存在性）的失效 ##########"
echo "  naive 检查：'if [ -d dist ]; then PASS; fi'  →  任何目录都通过"
echo "  这就是 20MB 首屏门槛的同构错误：门槛设在够不着的地方 = 没有门槛"

echo
echo "########## 结果：通过 $pass / 失败 $fail ##########"
# 🔴 收尾必须带 exit（2026-10-05 qa-engineer 修正，第 2 类假成功）：
#    此前末尾没有 exit → 无论成败都退出 0 → run-gate-selftest.sh 按返回值分类
#    → G29 在结构上**永远判不了红**。
printf 'SELFTEST_%s\n' "$([ "$fail" -eq 0 ] && echo PASS || echo FAIL)"
[ "$fail" -eq 0 ] || exit 1
exit 0
