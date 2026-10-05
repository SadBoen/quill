#!/usr/bin/env bash
# =============================================================================
# check-doc-script-drift.sh —— 文档声明与脚本实际行为的一致性检查
# =============================================================================
# ★ 存在意义：闸门脚本在 scripts/ 下（单一真相源），文档只**引用**它。
#   于是漂移风险方向变了 —— 不再是「md 里的脚本是旧版」，
#   而是「**文档描述的闸门行为与脚本实际行为不一致**」。
#   例如：文档表格说 R2 判违规，脚本却 warn → 读者按文档理解错，实际行为也不同。
#   **状态必须由工具给出，不能靠"我记得我改了"。**
#
# 本检查做两件事（都从**文件**读，不靠人转述）：
#   ① 每个检查项 ID（R1..Rn）必须同时出现在文档与脚本中；
#   ② 文档声明的「期望退出码」必须与脚本实际行为一致。
#
# 退出码（三态，与仓库契约一致）：
#   0 = pass         一致
#   1 = fail         检出漂移
#   2 = undetermined 输入缺失，无法判定（**绝不计入通过**）
#
# ⚠️ 踩坑记录：初版用 `$(...)` 捕获多行内容做比较 ——
#   命令替换会**剥掉尾部换行** → 内容 md5 相同却判漂移（假红）。
#   必须用**临时文件 + cmp** 逐字节比较。
# =============================================================================
set -uo pipefail

DOC="${1:?usage: check-doc-script-drift.sh <doc.md> <gate_script>}"
GATE="${2:?usage: check-doc-script-drift.sh <doc.md> <gate_script>}"

BAD=0
bad(){ printf '  x %s\n' "$*"; BAD=$((BAD+1)); }
ok(){  printf '  o %s\n' "$*"; }

[ -f "$DOC" ]  || { printf '  ▲ 文档不存在：%s —— 无法判定（不等于通过）\n' "$DOC"; exit 2; }
[ -f "$GATE" ] || { printf '  ▲ 闸门脚本不存在：%s —— 无法判定（不等于通过）\n' "$GATE"; exit 2; }

printf '[检查项 ID 一致性]\n'
# 从脚本里取实际存在的检查项 ID（形如 "# [R1] xxx"）
mapfile -t SCRIPT_IDS < <(grep -oE '^\s*#\s*\[R[0-9]+\]' "$GATE" | grep -oE 'R[0-9]+' | sort -u)
[ "${#SCRIPT_IDS[@]}" -gt 0 ] || { printf '  ▲ 脚本里没有 [Rn] 形式的检查项 —— 无法判定（不等于通过）\n'; exit 2; }

for id in "${SCRIPT_IDS[@]}"; do
  if grep -qE "\b${id}\b" "$DOC"; then
    ok "$id 文档与脚本均存在"
  else
    bad "$id 只在脚本里存在，文档未提及 —— 读者会漏掉这项检查"
  fi
done

# 反向：文档提到但脚本没有的 R 编号（排除 R1-R9 范围外的误报）
for id in $(grep -oE '\*\*R[0-9]+\*\*|\| R[0-9]+ \|' "$DOC" | grep -oE 'R[0-9]+' | sort -u); do
  found=0
  for sid in "${SCRIPT_IDS[@]}"; do [ "$sid" = "$id" ] && found=1 && break; done
  if [ "$found" -eq 0 ]; then
    bad "$id 文档提到但脚本没有实现 —— 文档描述了不存在的检查"
  fi
done

printf '\n[三态契约一致性]\n'
if grep -q 'undetermined' "$GATE" && grep -qE '0 = pass|0=pass' "$GATE"; then
  ok "脚本遵循三态契约（pass/fail/undetermined）"
else
  bad "脚本未遵循三态契约 —— 与 run-gate-selftest.sh 冲突会导致闸门静默失效"
fi

if grep -qE '无法判定|▲' "$GATE"; then
  ok "rc=2 措辞合规（含 ▲ 或 无法判定）"
else
  bad "rc=2 措辞不合规 —— runner check_wording 会报警"
fi

printf '\n'
if [ "$BAD" -gt 0 ]; then
  printf '结论：检出 %d 处文档↔脚本漂移。\n' "$BAD"
  printf '修法：以 scripts/%s 为准，同步更新 %s\n' "$(basename "$GATE")" "$(basename "$DOC")"
  exit 1
fi
printf '结论：文档与脚本一致（无漂移）。\n'
exit 0
