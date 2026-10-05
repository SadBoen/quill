#!/usr/bin/env python3
"""把两个真实数据集合成 100 条测试任务。

- MCP-Atlas（Scale AI，CC-BY-4.0）：真实用户 prompt + 启用工具 + 标准答案断言
- SkillsBench v1.1（Apache-2.0）：真实任务包 + 真实 SKILL.md 技能正文

产物：
  TESTSETS/tasks.json      100 条任务清单（含出处与许可）
  TESTSETS/skills/*.md     从 SKILL.md 剥掉 frontmatter 后的正文（quill 的技能文件格式）

**不编造**：每条任务都带 source / source_id / url，原始件留在 _raw/ 可逐条回溯。
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
RAW = ROOT / "TESTSETS" / "_raw"
OUT = ROOT / "TESTSETS"

ATLAS_N = 50
SKILL_N = 50

# ---------------------------------------------------------------- MCP-Atlas


def load_atlas():
    tasks = []
    for off in (0, 100, 200, 300, 400):
        p = RAW / f"mcp-atlas-{off}.json"
        if not p.exists():
            print(f"缺 {p.name}，跳过", file=sys.stderr)
            continue
        blob = json.loads(p.read_text(encoding="utf-8"))
        for r in blob.get("rows", []):
            row = r["row"]
            prompt = (row.get("PROMPT") or "").strip()
            if not prompt:
                continue
            try:
                tools = json.loads(row.get("ENABLED_TOOLS") or "[]")
            except json.JSONDecodeError:
                tools = []
            try:
                claims = row.get("GTFA_CLAIMS") or []
                if isinstance(claims, str):
                    claims = json.loads(claims.replace("'", '"'))
            except (json.JSONDecodeError, ValueError):
                claims = []
            tasks.append(
                {
                    "id": f"atlas-{row['TASK'][:12]}",
                    "source": "MCP-Atlas",
                    "source_id": row["TASK"],
                    "source_url": "https://huggingface.co/datasets/ScaleAI/MCP-Atlas",
                    "paper": "arXiv:2602.00933",
                    "license": "CC-BY-4.0",
                    "kind": "tool-use",
                    "prompt": prompt,
                    "required_tools": tools,
                    "expected_claims": claims,
                }
            )
    return tasks


# -------------------------------------------------------------- SkillsBench

FM = re.compile(r"\A---\s*\n(.*?)\n---\s*\n?", re.S)

# task.md 的元数据都嵌在 `metadata:` 下面（带缩进），而且部分是列表：
#     metadata:
#       difficulty: hard
#       interface:
#       - terminal
#       - python
# 只认行首无缩进的键会一个都取不到，category/difficulty 全是空串。
# 缩进的键要连同它下面的 `- item` 一起收。
KEY = re.compile(r"^(\s*)([A-Za-z_][\w-]*):\s*(.*)$")


def frontmatter(text):
    m = FM.match(text)
    if not m:
        return {}, text
    raw = m.group(1)
    body = text[m.end():]

    meta = {}
    last = None  # 当前正在收集列表的键
    for line in raw.splitlines():
        km = KEY.match(line)
        if not km:
            if last and re.match(r"^\s*-\s+", line):
                meta.setdefault(last, []).append(line.split("-", 1)[1].strip().strip("'\""))
            continue
        indent, key, val = km.group(1), km.group(2), km.group(3).strip()
        if indent:
            # 缩进键：只收我们要的那几个，免得把 environment.* 也拖进来
            if key in ("difficulty", "category", "subcategory", "skill_type", "interface"):
                if val and not val.startswith(("[", "|", ">")):
                    meta[key] = val.strip("'\"")
                    last = None
                elif not val:
                    meta[key] = []
                    last = key
            continue
        if val and not val.startswith(("[", "|", ">")):
            meta[key] = val.strip("'\"")
            last = None
        elif not val:
            meta[key] = []
            last = key
        else:
            last = None
    return meta, body


def as_list(v):
    """frontmatter 值可能是标量也可能是列表，统一成列表。"""
    if v is None or v == "":
        return []
    if isinstance(v, list):
        return [str(x) for x in v if str(x)]
    return [str(v)]


def scalar(v):
    if v is None or isinstance(v, list):
        return ""
    return str(v)


def load_skillsbench():
    root = RAW / "skillsbench"
    out_skills = OUT / "skills"
    out_skills.mkdir(parents=True, exist_ok=True)
    tasks = []
    for d in sorted(p for p in root.iterdir() if p.is_dir()):
        tm = d / "task.md"
        if not tm.exists():
            continue
        text = tm.read_text(encoding="utf-8", errors="replace")
        meta, body = frontmatter(text)
        prompt = body.strip()
        if not prompt:
            continue

        # 这个任务包自带的真实技能。SKILL.md 的 frontmatter 正好是 quill
        # skills 表的 name / description，正文落盘即 quill 的技能文件格式。
        skills = []
        for sk in sorted(d.rglob("SKILL.md")):
            sk_text = sk.read_text(encoding="utf-8", errors="replace")
            sk_meta, sk_body = frontmatter(sk_text)
            slug = (sk_meta.get("name") or sk.parent.name).strip()
            if not re.fullmatch(r"[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?", slug):
                continue  # 不合 quill 的 name 形状就跳过，不要硬塞
            (out_skills / f"{slug}.md").write_text(sk_body.strip() + "\n", encoding="utf-8")
            skills.append({"slug": slug, "description": sk_meta.get("description", "")})

        tasks.append(
            {
                "id": f"sb-{d.name}",
                "source": "SkillsBench",
                "source_id": d.name,
                "source_url": f"https://huggingface.co/datasets/benchflow/skillsbench/tree/main/{d.name}",
                "paper": "arXiv:2602.12670",
                "license": "Apache-2.0",
                "kind": "skill-use",
                "prompt": prompt,
                "difficulty": scalar(meta.get("difficulty")),
                "category": scalar(meta.get("category")),
                "interface": as_list(meta.get("interface")),
                "skills": skills,
                "expected_claims": [],
                "required_tools": [],
            }
        )
    return tasks


def main():
    atlas = load_atlas()
    skills = load_skillsbench()
    print(f"读到 MCP-Atlas {len(atlas)} 条 / SkillsBench {len(skills)} 条")

    # MCP-Atlas 那边只取 prompt 够长、断言够实的：太短的任务测不出东西，
    # 没有 expected_claims 的也测不了对错。
    atlas_ok = [
        t
        for t in atlas
        if len(t["prompt"]) >= 80 and len(t["expected_claims"]) >= 1 and t["required_tools"]
    ]
    print(f"MCP-Atlas 里 prompt>=80 字、且有断言与工具的: {len(atlas_ok)} 条")

    skills_ok = [t for t in skills if len(t["prompt"]) >= 80]
    print(f"SkillsBench 里 prompt>=80 字的: {len(skills_ok)} 条")

    picked = atlas_ok[:ATLAS_N] + skills_ok[:SKILL_N]
    if len(picked) < 100:
        print(
            f"只有 {len(picked)} 条，不足 100 —— 如实报错，不拿凑数的东西补。",
            file=sys.stderr,
        )
        return 1

    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "tasks.json").write_text(
        json.dumps(picked, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    n_skill_files = len(list((OUT / "skills").glob("*.md")))
    print(f"写出 tasks.json: {len(picked)} 条")
    print(f"写出 skills/: {n_skill_files} 个真实 SKILL.md")
    from collections import Counter

    for k, v in Counter(t["source"] for t in picked).items():
        print(f"  {k}: {v}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
