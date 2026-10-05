#!/usr/bin/env bash
# 0001_init.sql 的可复现验证入口。
#   用法：bash crates/quill-store/verify/run.sh
#   退出码非 0 = 有断言未通过（不做任何吞异常兜底）
#
# 三份脚本均只用 Python 内置 sqlite3 模块，不依赖 sqlite3 / zstd 等 CLI（铁律三）。
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
cd "$REPO" || exit 1

command -v python3 >/dev/null 2>&1 || {
  echo "✗ 无法判定：python3 不存在 —— 缺工具必须失败，不许兜底跳过" >&2
  exit 2
}

rc=0
for s in verify.py retrieval_final.py assert_invites.py; do
  echo "──────────────────────────────────────────────────────────────"
  echo "▶ $s"
  echo "──────────────────────────────────────────────────────────────"
  python3 "$HERE/$s"
  r=$?
  echo "  ↳ $s rc=$r"
  if [ "$r" -ne 0 ]; then rc=1; fi
  echo
done

echo "══════════════════════════════════════════════════════════════"
if [ "$rc" -eq 0 ]; then
  echo "全部验证脚本通过"
else
  echo "存在未通过的验证脚本（见上方 rc≠0 行）"
fi
echo "══════════════════════════════════════════════════════════════"
exit "$rc"
