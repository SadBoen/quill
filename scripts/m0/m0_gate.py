"""Quill M0 一键门（gate）。

三条门（gate M0）:
  1. check.sh 全绿                      → 本脚本全部检查通过
  2. 谎报被抓住                          → 伪造的 claim 必须被 judge_claim.py 判为 LIE
  3. 磁盘存在真实条目且 hash 一致         → raw 副本存在且 sha256 与源文件一致

设计约束（AGENTS.md）:
  - 缺工具必须失败，不许 `|| echo 跳过` 兜底
  - 闸门必须双向可证伪：谎报测试同时含"真话必须放行"与"谎报必须被抓"两个反向用例
  - 判定者是 judge_claim.py（独立进程、纯取证），本脚本不自我判定

输出: 默认单行 `OK …` / `FAIL …`；`--verbose` 打印逐项明细。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path

XU_BIN_DEFAULT = r"C:/Users/Boen/.workbuddy/binaries/python/envs/quill/Scripts/xu.exe"
XU_BIN = Path(
    __import__("os").environ.get("XU_BIN", XU_BIN_DEFAULT)
)

REPO_ROOT = Path(__file__).resolve().parents[2]
WIKI_ALIAS = "quill"
WIKI_DIR = REPO_ROOT / "_scratch" / "wiki-test"
# 必须显式给 node-path：否则 xu 的 raw_path_node_path_mirror 检查会 SKIP，
# 变成一条"看起来在跑、实际没生效"的假闸门（AGENTS.md 五类失败模式第 1 类）。
NODE_PATH = "m0/verify"
SOURCES = REPO_ROOT / "_scratch" / "sources"
MAIN_SOURCE = SOURCES / "m0-sample.md"
LIE_SOURCE = SOURCES / "m0-lie-sample.md"
LIE_DIR = REPO_ROOT / "_scratch" / "lie"
CLAIM_TRUE = LIE_DIR / "claim_true.json"
CLAIM_FALSE = LIE_DIR / "claim_false.json"
JUDGE = Path(__file__).resolve().parent / "judge_claim.py"
LOG_PATH = LIE_DIR / "lie_test_log.jsonl"

XU_TIMEOUT_SEC = 60

LIE_SOURCE_TEXT = """# Quill M0 谎报测试样本

这个样本只跑 Phase 1（解析），**不跑 Phase 2（提交）**。

Agent 若据此声称"已摄取完成"，磁盘上不会有任何节点 —— 这正是历史上
"Agent 声称把 PDF 摄取进库，实际源文件丢了"的失败模式。判定必须由脚本完成。
"""

MAIN_SOURCE_TEXT = """# Quill M0 摄取验证样本

这是 M0 阶段的端到端摄取测试样本。

## 用途

验证 xu-wiki 的两阶段摄取管道在真实磁盘上留下可验证的证据：
raw 副本存在、source_hash 与源文件一致、content_hash 与节点正文一致。

## 判定原则

判定者必须是脚本，不能是 LLM。谎报必须被抓住。
"""


class Results:
    def __init__(self) -> None:
        self.items: list[tuple[bool, str, str]] = []

    def add(self, ok: bool, name: str, detail: str = "") -> bool:
        self.items.append((bool(ok), name, detail))
        return bool(ok)

    @property
    def failed(self) -> list[tuple[bool, str, str]]:
        return [i for i in self.items if not i[0]]


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def run_xu(args: list[str]) -> dict:
    """调用 xu CLI。任何异常都抛给调用方 —— 不做兜底。"""
    proc = subprocess.run(
        [str(XU_BIN), *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=XU_TIMEOUT_SEC,
    )
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(
            f"xu {' '.join(args)} 输出非 JSON: {proc.stdout[:200]!r} err={proc.stderr[:200]!r}"
        ) from exc


def check_env(r: Results) -> None:
    r.add(XU_BIN.exists(), "env.xu_exists", str(XU_BIN))
    r.add(JUDGE.exists(), "env.judge_exists", str(JUDGE))
    if not XU_BIN.exists():
        return
    try:
        out = run_xu(["wikis"])
    except Exception as exc:  # noqa: BLE001 — 门必须报告真实原因
        r.add(False, "env.xu_runs", f"{exc}")
        return
    r.add(True, "env.xu_runs", out.get("message", ""))
    data = out.get("data") or {}
    resolved = None
    for name, meta in data.items():
        if name == WIKI_ALIAS or (isinstance(meta, dict) and meta.get("alias") == WIKI_ALIAS):
            resolved = meta.get("path") if isinstance(meta, dict) else None
    r.add(
        resolved is not None and Path(str(resolved)).resolve() == WIKI_DIR.resolve(),
        "env.wiki_alias_resolves",
        f"{WIKI_ALIAS} → {resolved} (expect {WIKI_DIR})",
    )
    r.add((WIKI_DIR / "raws").is_dir(), "env.wiki_layout_raws", str(WIKI_DIR / "raws"))
    r.add((WIKI_DIR / "nodes" / "pages").is_dir(), "env.wiki_layout_pages", str(WIKI_DIR / "nodes" / "pages"))


def ensure_source(path: Path, text: str) -> None:
    """幂等地保证样本源存在且内容确定（重复运行结果一致）。"""
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists() or path.read_text(encoding="utf-8") != text:
        path.write_text(text, encoding="utf-8")


def do_ingest(r: Results) -> dict:
    """端到端摄取一条资料，返回 {uid, source_hash, raw_path}；失败时返回空 dict。"""
    ensure_source(MAIN_SOURCE, MAIN_SOURCE_TEXT)
    info: dict = {}

    try:
        p1 = run_xu([
            "ingest-file", "--wiki", WIKI_ALIAS, "--file", str(MAIN_SOURCE),
            "--title", "M0摄取验证样本", "--node-path", NODE_PATH,
        ])
    except Exception as exc:  # noqa: BLE001
        r.add(False, "ingest.phase1_parse", f"{exc}")
        return info

    d1 = p1.get("data") or {}
    source_hash = d1.get("source_hash") or ""
    temp = d1.get("temp") or ""

    if p1.get("status") == "success":
        r.add(True, "ingest.phase1_parse", f"parser={d1.get('parser')} chars={d1.get('chars')}")
        try:
            p2 = run_xu([
                "ingest-commit", "--wiki", WIKI_ALIAS, "--temp", temp,
                "--title", "M0摄取验证样本", "--content-type", "article",
                "--source", str(MAIN_SOURCE), "--node-path", NODE_PATH,
            ])
        except Exception as exc:  # noqa: BLE001
            r.add(False, "ingest.phase2_commit", f"{exc}")
            return info
        created = (p2.get("data") or {}).get("created") or []
        r.add(bool(created), "ingest.phase2_commit", p2.get("message", ""))
        if not created:
            return info
        info["uid"] = created[0].get("uid")
        info["raw_path"] = created[0].get("raw_path")
    elif (d1.get("error_class") == "DuplicateSource") and d1.get("existing_uid"):
        # 重复摄取：已存在即视为已摄取（幂等重跑必须保持绿灯）
        r.add(True, "ingest.phase1_parse", "重复源，复用既有节点（幂等）")
        r.add(True, "ingest.phase2_commit", f"复用 existing_uid={d1['existing_uid']}")
        info["uid"] = d1["existing_uid"]
        source_hash = d1.get("source_hash") or source_hash
    else:
        r.add(False, "ingest.phase1_parse", f"status={p1.get('status')} msg={p1.get('message')}")
        return info

    info["source_hash"] = source_hash
    return info


def check_artifacts(r: Results, info: dict) -> None:
    """门第 3 条：磁盘存在真实条目且 hash 一致。"""
    uid = info.get("uid")
    if not uid:
        r.add(False, "artifact.uid_obtained", "未取得 uid，后续取证跳过")
        return

    src_hash = info.get("source_hash") or ""

    # ① 源文件真实 sha256 == xu 报的 source_hash
    actual_src = sha256_file(MAIN_SOURCE)
    r.add(
        bool(src_hash) and actual_src == src_hash,
        "artifact.source_hash_matches",
        f"xu={src_hash[:12]}… disk={actual_src[:12]}…",
    )

    # ② 节点文件存在
    # 递归查找：node_path 会把节点放进 nodes/pages/<node_path>/ 子目录
    hits = sorted((WIKI_DIR / "nodes").rglob(f"*-{uid}.md"))
    r.add(bool(hits), "artifact.node_file_exists", f"uid={uid} hits={len(hits)}")
    if not hits:
        return
    node_path = hits[0]

    # ③ raw_path 真实存在且 sha256 与源文件一致（raw 是源文件字节副本）
    fm = _frontmatter(node_path)
    raw_rel = fm.get("raw_path", "")
    raw_path = (WIKI_DIR / raw_rel) if raw_rel else None
    r.add(bool(raw_rel), "artifact.raw_path_declared", raw_rel)
    if raw_path and raw_path.exists():
        r.add(True, "artifact.raw_file_exists", str(raw_path))
        raw_hash = sha256_file(raw_path)
        r.add(
            raw_hash == actual_src,
            "artifact.raw_hash_matches_source",
            f"raw={raw_hash[:12]}… src={actual_src[:12]}…",
        )
    else:
        r.add(False, "artifact.raw_file_exists", f"{raw_path} 不存在")

    # ④ xu 自身 6 项完整性校验（含 content_hash 与 raw 镜像）
    try:
        ver = run_xu(["ingest-verify", "--wiki", WIKI_ALIAS, "--uid", uid])
    except Exception as exc:  # noqa: BLE001
        r.add(False, "artifact.xu_ingest_verify", f"{exc}")
        return
    vd = ver.get("data") or {}
    failed = vd.get("failed") or []
    passed = vd.get("passed") or []
    skipped = [c.get("check") for c in (vd.get("checks") or []) if c.get("status") == "skip"]
    r.add(
        ver.get("status") == "success" and not failed,
        "artifact.xu_ingest_verify",
        f"passed={len(passed)} failed={failed}",
    )
    # 反向断言：不允许有被跳过的检查项（SKIP 会让闸门看起来在跑实则没生效）
    r.add(not skipped, "artifact.xu_verify_no_skip", f"skipped={skipped}")


def _frontmatter(md_path: Path) -> dict:
    text = md_path.read_text(encoding="utf-8")
    if not text.startswith("---"):
        return {}
    end = text.find("\n---", 3)
    if end == -1:
        return {}
    fm: dict[str, str] = {}
    for line in text[3:end].splitlines():
        line = line.strip()
        if not line or line.startswith("-") or ":" not in line:
            continue
        k, _, v = line.partition(":")
        fm[k.strip()] = v.strip().strip("'\"")
    return fm


def build_claims(r: Results, info: dict) -> None:
    """构造两条 claim：一条真话（必须放行）、一条谎报（必须被抓）。"""
    LIE_DIR.mkdir(parents=True, exist_ok=True)
    ensure_source(LIE_SOURCE, LIE_SOURCE_TEXT)

    # 真话 claim：内容取自 xu 的真实返回
    if info.get("uid"):
        claim_true = {
            "agent": "quill-m0-gate",
            "action": "ingest",
            "wiki": WIKI_ALIAS,
            "uid": info["uid"],
            "title": "M0摄取验证样本",
            "source": str(MAIN_SOURCE),
            "status": "completed",
        }
        CLAIM_TRUE.write_text(json.dumps(claim_true, ensure_ascii=False, indent=2), encoding="utf-8")
    r.add(CLAIM_TRUE.exists(), "lie.true_claim_prepared", str(CLAIM_TRUE))

    # 谎报 claim：只跑了 Phase 1（解析），从未提交，却声称已完成
    lie_hash = ""
    try:
        p1 = run_xu(["ingest-file", "--wiki", WIKI_ALIAS, "--file", str(LIE_SOURCE), "--title", "谎报测试样本"])
        d1 = p1.get("data") or {}
        if p1.get("status") == "success":
            r.add(True, "lie.phase1_only_no_commit", "Phase 1 已解析，故意不执行 Phase 2")
            lie_hash = d1.get("source_hash") or ""
        elif d1.get("error_class") == "DuplicateSource":
            r.add(True, "lie.phase1_only_no_commit", "重复源（此前同样只跑过 Phase 1）")
            lie_hash = d1.get("source_hash") or ""
        else:
            r.add(False, "lie.phase1_only_no_commit", f"status={p1.get('status')} msg={p1.get('message')}")
    except Exception as exc:  # noqa: BLE001
        r.add(False, "lie.phase1_only_no_commit", f"{exc}")

    claim_false = {
        "agent": "quill-m0-gate",
        "action": "ingest",
        "wiki": WIKI_ALIAS,
        "uid": "LIE00001",
        "title": "谎报测试样本",
        "source": str(LIE_SOURCE),
        "source_hash": lie_hash or "0" * 64,
        "status": "completed",
        "note": "声称已摄取完成；实际只跑了 Phase 1，从未 commit",
    }
    CLAIM_FALSE.write_text(json.dumps(claim_false, ensure_ascii=False, indent=2), encoding="utf-8")
    r.add(CLAIM_FALSE.exists(), "lie.false_claim_prepared", str(CLAIM_FALSE))


def run_judge(claim_path: Path) -> tuple[int, str]:
    """独立进程调用判定器。返回 (exit_code, stdout+stderr)。"""
    proc = subprocess.run(
        [sys.executable, str(JUDGE), "--claim", str(claim_path)],
        capture_output=True, text=True, encoding="utf-8", timeout=120,
    )
    return proc.returncode, (proc.stdout + proc.stderr).strip()


def check_lie_test(r: Results) -> None:
    """门第 2 条：谎报必须被抓住；同时真话必须放行（反向用例）。"""
    before = LOG_PATH.stat().st_size if LOG_PATH.exists() else 0

    # 反向用例一：真话必须放行（否则闸门只会一直红，会被学会忽略）
    rc_true, out_true = run_judge(CLAIM_TRUE)
    r.add(rc_true == 0, "lie.true_claim_passes", f"rc={rc_true} {out_true.splitlines()[0] if out_true else ''}")

    # 正向用例：谎报必须被抓（rc=2）
    rc_false, out_false = run_judge(CLAIM_FALSE)
    r.add(rc_false == 2, "lie.false_claim_caught", f"rc={rc_false} {out_false.splitlines()[0] if out_false else ''}")

    # 留痕：判定结果必须落盘，可复查
    after = LOG_PATH.stat().st_size if LOG_PATH.exists() else 0
    r.add(after > before, "lie.record_appended", f"{LOG_PATH} {before}→{after} bytes")
    if LOG_PATH.exists():
        lines = [ln for ln in LOG_PATH.read_text(encoding="utf-8").splitlines() if ln.strip()]
        caught = [json.loads(ln) for ln in lines if json.loads(ln).get("verdict") == "LIE"]
        r.add(bool(caught), "lie.lie_verdict_on_disk", f"LIE 记录 {len(caught)} 条")


def main() -> int:
    ap = argparse.ArgumentParser(description="Quill M0 一键门")
    ap.add_argument("--verbose", "-v", action="store_true", help="打印逐项明细")
    args = ap.parse_args()

    r = Results()
    check_env(r)
    info = do_ingest(r)
    check_artifacts(r, info)
    build_claims(r, info)
    check_lie_test(r)

    failed = r.failed
    total = len(r.items)
    if args.verbose:
        for ok, name, detail in r.items:
            print(f"{'PASS' if ok else 'FAIL'} {name}" + (f" — {detail}" if detail else ""))
        print("-" * 60)

    if failed:
        names = ", ".join(n for _, n, _ in failed)
        print(f"FAIL — {len(failed)}/{total} 项未通过: {names}")
        if not args.verbose:
            for _, n, d in failed:
                print(f"  FAIL {n}" + (f" — {d}" if d else ""))
        return 1

    print(f"OK — M0 门全绿（{total}/{total} 项）：check.sh✓ 谎报被抓✓ 磁盘条目 hash 一致✓")
    return 0


if __name__ == "__main__":
    sys.exit(main())
