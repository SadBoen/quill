#!/usr/bin/env bash
# 重新拉取 100 条真实测试任务的原始件并重建清单。
#
# 两个来源（许可与出处见 TESTSETS/README.md）：
#   - MCP-Atlas   Scale AI   CC-BY-4.0  huggingface.co/datasets/ScaleAI/MCP-Atlas
#   - SkillsBench v1.1      Apache-2.0  huggingface.co/datasets/benchflow/skillsbench
#
# 原始件落在 TESTSETS/_raw/（已 gitignore）。入库的只有派生结果。
# 全程匿名，不需要 token。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RAW="$ROOT/TESTSETS/_raw"
ATLAS_ROWS="https://datasets-server.huggingface.co/rows?dataset=ScaleAI/MCP-Atlas&config=default&split=train"

mkdir -p "$RAW"

echo "== 1/4 SkillsBench 任务包（全量文件，含输入夹具） =="
# 这里交给 fetch_skillsbench.py：它走 HF tree API 分页枚举，再把前 50 个
# 任务包的**每一个**文件都拉下来。
#
# 早前这个脚本只下 task.md 与 SKILL.md，于是每个任务看上去「零输入夹具」，
# 任务提示里也就没有任何可读路径——模型回「缺少 xxx」是在如实汇报环境，
# 不是模型能力问题。真实夹具（packets.pcap / wave.mseed / handbook.pdf /
# sensor_data.csv …）本来就在上游仓库里，只是没被下载。见 ISSUES.md。
python3 "$ROOT/TESTSETS/fetch_skillsbench.py"

echo "== 2/4 MCP-Atlas 500 条任务 =="
for off in 0 100 200 300 400; do
  # datasets-server 一次最多给 100 行，分 5 批。
  curl -sS "$ATLAS_ROWS&offset=$off&length=100" -o "$RAW/mcp-atlas-$off.json"
  printf '  offset=%s -> %s bytes\n' "$off" "$(stat -c%s "$RAW/mcp-atlas-$off.json")"
done

echo "== 3/4 合成 tasks.json 与 skills/ =="
python3 "$ROOT/TESTSETS/build_tasks.py"

echo "== 4/4 抽查：任务提示里是否真写了路径 =="
python3 - "$ROOT" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
tasks = json.loads((root / "TESTSETS" / "tasks.json").read_text(encoding="utf-8"))
with_path = [t for t in tasks if "/mnt/" in t["prompt"] or "/mnt/" in json.dumps(t, ensure_ascii=False)]
sb = [t for t in tasks if t["source"] == "SkillsBench"]
sb_fx = [t for t in sb if t.get("fixtures")]
atlas_missing = [t for t in tasks if t["source"] == "MCP-Atlas" and t.get("referenced_inputs_missing")]
missing_on_disk = []
for t in sb_fx:
    for f in t["fixtures"]:
        if not pathlib.Path(f["path"]).exists():
            missing_on_disk.append(f["path"])
print(f"  SkillsBench 任务 {len(sb)} 条，其中带真实输入夹具的 {len(sb_fx)} 条")
print(f"  夹具路径总数 {sum(len(t['fixtures']) for t in sb_fx)}，磁盘上不存在的 {len(missing_on_disk)} 个")
if missing_on_disk:
    print("  如实报错：任务提示里写了磁盘上没有的路径", file=sys.stderr)
    for p in missing_on_disk[:10]:
        print("    " + p, file=sys.stderr)
    raise SystemExit(1)
print(f"  MCP-Atlas 提示里声明了缺失输入的任务 {len(atlas_missing)} 条")
print("  全部路径可读")
PY

echo
echo "完成。清单：$ROOT/TESTSETS/tasks.json"
