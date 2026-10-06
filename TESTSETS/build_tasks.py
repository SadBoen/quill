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
from collections import Counter

ROOT = pathlib.Path(__file__).resolve().parents[1]
RAW = ROOT / "TESTSETS" / "_raw"
OUT = ROOT / "TESTSETS"

# tasks.json 是入库文件，里面不能写死本机绝对路径。统一用这个占位符，
# 发送前由 runner 换成本机仓库根目录（换不掉就报错，不带着占位符发出去）。
REPO = "{REPO}"

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
            traj = row.get("TRAJECTORY") or ""
            if isinstance(traj, list):
                traj = json.dumps(traj, ensure_ascii=False)
            missing = referenced_inputs(traj)
            prompt = attach_atlas_source(
                prompt, p, int(r.get("row_idx", 0)), row["TASK"], missing
            )
            tasks.append(
                {
                    # **整个 TASK 都拿来做 id，不许截断。**
                    #
                    # MCP-Atlas 的 TASK 是 24 位十六进制，**前 12 位是分组前缀、
                    # 后 12 位才是序号**（实测：689bd255c0422b257e7dfca5 /
                    # …ca8 / …ca9 是三条**不同**的任务）。
                    # 原来这里截到 12 位，恰好只留下分组前缀、把唯一的那部分扔了：
                    # 500 行原始数据最后只剩 **32 个** id，498 行撞车。见 ISSUE-024。
                    "id": f"atlas-{row['TASK']}",
                    "source": "MCP-Atlas",
                    "source_id": row["TASK"],
                    "source_url": "https://huggingface.co/datasets/ScaleAI/MCP-Atlas",
                    "paper": "arXiv:2602.00933",
                    "license": "CC-BY-4.0",
                    "kind": "tool-use",
                    "prompt": prompt,
                    "required_tools": tools,
                    "expected_claims": claims,
                    "source_file": REPO + "/" + p.relative_to(ROOT).as_posix(),
                    "source_row": int(r.get("row_idx", 0)),
                    "referenced_inputs_missing": missing,
                }
            )
    return tasks


# 任务提示里引用、但上游数据集根本没随任务发布的输入路径。
# 只出现在参考轨迹 TRAJECTORY 里（它记录了当年在评测环境里怎么调的）。
# 把它们**明说**出来，是为了不让模型去凭空造数据 —— 这是诚实，不是给答案。
DATA_PATH = re.compile(r"/data/[A-Za-z0-9_./\- ]*?[A-Za-z0-9](?:\.[A-Za-z0-9]{1,8})")


def referenced_inputs(traj_text: str):
    found = []
    for m in DATA_PATH.finditer(traj_text or ""):
        s = m.group(0).strip().rstrip('.,;"\'')
        if s not in found:
            found.append(s)
    return sorted(found)


def attach_atlas_source(prompt: str, src_file: pathlib.Path, row_idx: int, task_id: str, missing):
    src_rel = src_file.relative_to(ROOT).as_posix()
    lines = ["", "---", "", "【本条任务的出处与输入现状】"]
    lines.append(f"数据集原始记录：{REPO}/{src_rel}  第 {row_idx} 行，TASK={task_id}")
    if missing:
        lines.append("")
        lines.append(
            "这条任务当年在评测环境里依赖下列输入文件。上游仓库只发布了任务文本与"
            "参考轨迹，**这些数据文件本身没有随数据集发布，本机也不存在**："
        )
        for s in missing[:MAX_LISTED_FIXTURES]:
            lines.append(f"  - {s}")
        if len(missing) > MAX_LISTED_FIXTURES:
            lines.append(f"  …… 共 {len(missing)} 个，只列了前 {MAX_LISTED_FIXTURES} 个")
        lines.append("")
        lines.append(
            "所以：不要编造这些文件的内容。如果任务必须依赖它们才能作答，"
            "请直接说明缺哪个、为什么缺，而不是猜一个答案。"
        )
    else:
        lines.append("")
        lines.append("这条任务不依赖任何随数据集发布的输入文件。")
    return prompt + "\n".join(lines) + "\n"


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


# ---------------------------------------------------------------- 输入夹具
#
# 这些任务包**确实带输入数据**（packets.pcap / wave.mseed / handbook.pdf /
# sensor_data.csv …）。早前只下载了 task.md 与 SKILL.md，于是每个任务看上去
# 「零输入夹具」，任务提示因此既没路径也没数据，模型只能回一句「缺少 xxx」——
# 那是在如实汇报环境，不算模型能力问题。路径必须写进任务提示。
#
# oracle/ 与 verifier/ 一律排除：里面是 solve.sh 和 ground_truths，
# 写进提示等于把标准答案抄给模型。

EXCLUDE_DIR_PARTS = ("/oracle/", "/verifier/", "/.git/")
EXCLUDE_NAMES = ("task.md", "Dockerfile", "solve.sh", "solve.py")
MAX_LISTED_FIXTURES = 40


def collect_fixtures(pkg_dir: pathlib.Path):
    """列出这个任务包真实存在的输入文件（仓库内相对路径 + 字节数）。

    路径用**仓库相对路径 + {REPO} 占位符**，不写死绝对路径：
    `tasks.json` 是入库文件，写死 `/mnt/d/96_CoderWorld/quill/...` 会让别的
    机器 clone 下来就指向一个不存在的目录。发送前由 runner 把 `{REPO}`
    换成本机仓库根目录，换不掉就直接报错，不让它带着占位符发出去。
    """
    out = []
    for p in sorted(pkg_dir.rglob("*")):
        if not p.is_file():
            continue
        rel = p.relative_to(pkg_dir).as_posix()
        posix = "/" + rel
        if any(x in posix for x in EXCLUDE_DIR_PARTS):
            continue
        if p.name in EXCLUDE_NAMES or rel in EXCLUDE_NAMES:
            continue
        if "/skills/" in posix and ("/scripts/" in posix or "/references/" in posix):
            continue  # 技能自带脚本，随技能正文走，不算任务输入
        try:
            size = p.stat().st_size
        except OSError:
            continue
        out.append({
            "rel": rel,
            "path": REPO + "/" + p.relative_to(ROOT).as_posix(),
            "size": size,
        })
    return out


def human_size(n: int) -> str:
    if n >= 1048576:
        return f"{n / 1048576:.1f} MB"
    if n >= 1024:
        return f"{n / 1024:.1f} KB"
    return f"{n} B"


def attach_paths(prompt: str, pkg_dir: pathlib.Path, skills, fixtures):
    """把真实存在的文件路径追加到任务提示末尾。"""
    if not fixtures and not skills:
        return prompt

    pkg_rel = pkg_dir.relative_to(ROOT).as_posix()
    lines = ["", "---", "", "【本机可读的真实文件路径】"]

    if skills:
        lines.append("技能正文（已剥掉 frontmatter，与 quill 的技能文件格式一致）：")
        for s in skills:
            lines.append(
                f"  - {REPO}/TESTSETS/skills/{s['slug']}.md  <- 技能 {s['slug']}"
            )

    if fixtures:
        lines.append("任务输入文件（真实存在，可直接打开）：")
        shown = fixtures[:MAX_LISTED_FIXTURES]
        for f in shown:
            lines.append(f"  - {f['path']}  ({human_size(f['size'])})")
        if len(fixtures) > MAX_LISTED_FIXTURES:
            lines.append(
                f"  …… 本任务包共 {len(fixtures)} 个输入文件，上面只列了前 "
                f"{MAX_LISTED_FIXTURES} 个；目录 {REPO}/{pkg_rel}/ 下是完整的那一份。"
            )
    else:
        lines.append(f"本任务包没有输入文件；技能与参考材料都在 {REPO}/{pkg_rel}/ 下。")

    lines.append("")
    lines.append(
        "以上路径是本机真实路径，可直接读取；不要另造路径，也不要凭想象编造文件内容。"
    )
    return "\n".join(lines) + "\n"


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

        fixtures = collect_fixtures(d)
        prompt = attach_paths(prompt, d, skills, fixtures)

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
                "fixtures": fixtures,
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

    # id 必须真的唯一。**这条断言是被真实事故逼出来的**：MCP-Atlas 那边
    # 一度把 TASK 截到 12 位，500 行只剩 32 个 id，于是「100 条任务」里有
    # 34 条两两无法区分 —— 按 id 记进度会**虚增**，按 id 跑某一条会跑到
    # 另一条身上。这种错静默通过，跑得越多越看不出来，所以在这里当场拦住，
    # 不写进 tasks.json。见 ISSUE-024。
    dup_ids = [i for i, n in Counter(t["id"] for t in picked).items() if n > 1]
    if dup_ids:
        print(
            f"id 不唯一，共 {len(dup_ids)} 组冲突（{', '.join(sorted(dup_ids)[:5])} …）—— "
            f"如实报错，不写出 tasks.json。",
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
    for k, v in Counter(t["source"] for t in picked).items():
        print(f"  {k}: {v}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
