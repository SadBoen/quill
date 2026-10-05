#!/usr/bin/env bash
# 重新拉取 100 条真实测试任务的原始件并重建清单。
#
# 两个来源（许可与出处见 TESTSETS/README.md）：
#   - MCP-Atlas   Scale AI   CC-BY-4.0  huggingface.co/datasets/ScaleAI/MCP-Atlas
#   - SkillsBench v1.1      Apache-2.0  huggingface.co/datasets/benchflow/skillsbench
#
# 原始件落在 TESTSETS/_raw/（已 gitignore，71MB）。入库的只有派生结果。
# 全程匿名，不需要 token。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RAW="$ROOT/TESTSETS/_raw"
ATLAS_ROWS="https://datasets-server.huggingface.co/rows?dataset=ScaleAI/MCP-Atlas&config=default&split=train"
SB_RAW="https://huggingface.co/datasets/benchflow/skillsbench/resolve/main"

mkdir -p "$RAW/skillsbench"

echo "== 1/4 SkillsBench 文件树 =="
curl -sS "https://huggingface.co/api/datasets/benchflow/skillsbench" -o "$RAW/skillsbench-tree.json"
python3 - "$RAW" <<'PY'
import json, pathlib, sys
raw = pathlib.Path(sys.argv[1])
d = json.loads((raw / "skillsbench-tree.json").read_text(encoding="utf-8"))
files = [s["rfilename"] for s in d.get("siblings", [])]
tasks = sorted({f.split("/")[0] for f in files if f.endswith("task.md")})
skills = [f for f in files if f.endswith("SKILL.md")]
(raw / "skillsbench-tasks.txt").write_text("\n".join(tasks), encoding="utf-8")
(raw / "skillsbench-skillfiles.txt").write_text("\n".join(skills), encoding="utf-8")
print(f"  任务包 {len(tasks)} 个 / SKILL.md {len(skills)} 份")
PY

echo "== 2/4 取前 50 个 SkillsBench 任务包 =="
N=0
while IFS= read -r t; do
  [ "$N" -ge 50 ] && break
  mkdir -p "$RAW/skillsbench/$t"
  curl -sfL "$SB_RAW/$t/task.md" -o "$RAW/skillsbench/$t/task.md"
  while IFS= read -r sf; do
    rel="${sf#${t}/}"
    mkdir -p "$RAW/skillsbench/$t/$(dirname "$rel")"
    curl -sfL "$SB_RAW/$sf" -o "$RAW/skillsbench/$t/$rel"
  done < <(grep "^${t}/" "$RAW/skillsbench-skillfiles.txt" || true)
  N=$((N+1))
done < "$RAW/skillsbench-tasks.txt"
echo "  拉到 $N 个任务包，SKILL.md $(find "$RAW/skillsbench" -name SKILL.md | wc -l) 份"

echo "== 3/4 MCP-Atlas 500 条任务 =="
for off in 0 100 200 300 400; do
  # datasets-server 一次最多给 100 行，分 5 批。
  curl -sS "$ATLAS_ROWS&offset=$off&length=100" -o "$RAW/mcp-atlas-$off.json"
  printf '  offset=%s → %s 字节\n' "$off" "$(stat -c%s "$RAW/mcp-atlas-$off.json")"
done

echo "== 4/4 合成 tasks.json 与 skills/ =="
python3 "$ROOT/TESTSETS/build_tasks.py"

echo
echo "完成。清单：$ROOT/TESTSETS/tasks.json"
