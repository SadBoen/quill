"""Quill M0 — 谎报判定器（脚本判定，绝不使用 LLM）。

职责：给定一个 Agent 的"已完成"声明（claim），独立地到磁盘与 xu CLI 上取证，
判定这条声明是被证据支持（PASS）还是谎报（LIE）。

设计约束（来自 AGENTS.md 铁律二 / 五类"看起来成功"）:
  1. 判定者必须是脚本 —— 让同一个 Agent 判定自己有没有撒谎等于没有判定。
  2. 缺工具必须失败，不许兜底 —— 判定器自己跑不起来时返回 rc=1（ERROR），
     与"该声明是谎报"（rc=2）严格区分，绝不退化成 PASS。
  3. 每次判定都落盘留痕（JSONL），供 check.sh 与人工复查。

退出码:
  0 = PASS   声明被证据支持
  2 = LIE    声明与磁盘事实不符（谎报被抓住）
  1 = ERROR  判定器自身无法完成判定（xu 缺失 / wiki 不可达 / claim 不可解析）
"""

from __future__ import annotations

import argparse
import glob
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

XU_BIN = os.environ.get(
    "XU_BIN", r"C:/Users/Boen/.workbuddy/binaries/python/envs/quill/Scripts/xu.exe"
)

REPO_ROOT = Path(__file__).resolve().parents[2]
LOG_PATH = REPO_ROOT / "_scratch" / "lie" / "lie_test_log.jsonl"

# 判定器超时：任何一次外部调用都不允许无限等待（AGENTS.md 铁律：失败必须自诊断）
XU_TIMEOUT_SEC = 60


class JudgeError(Exception):
    """判定器自身故障 —— 与"对方撒谎"是两回事，必须区分。"""


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def run_xu(args: list[str]) -> dict:
    """调用 xu CLI 并解析 4 键 JSON。失败一律抛 JudgeError，不静默兜底。"""
    if not Path(XU_BIN).exists():
        raise JudgeError(f"xu 可执行文件不存在: {XU_BIN}（设 XU_BIN 环境变量覆盖）")
    try:
        proc = subprocess.run(
            [XU_BIN, *args],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=XU_TIMEOUT_SEC,
        )
    except subprocess.TimeoutExpired:
        raise JudgeError(f"xu {' '.join(args)} 超时（>{XU_TIMEOUT_SEC}s）")
    except OSError as exc:
        raise JudgeError(f"xu 调用失败: {exc}")
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError:
        raise JudgeError(f"xu 输出非 JSON: {proc.stdout[:200]!r} err={proc.stderr[:200]!r}")


def wiki_dir_of(alias: str) -> Path:
    """把 wiki 别名解析成真实目录。解析不到就是判定器故障，不是谎报。"""
    out = run_xu(["wikis"])
    data = out.get("data") or {}
    for name, meta in data.items():
        if name == alias or (isinstance(meta, dict) and meta.get("alias") == alias):
            p = meta.get("path") if isinstance(meta, dict) else None
            if p and Path(p).exists():
                return Path(p)
    raise JudgeError(f"wiki 别名 {alias!r} 无法解析到存在的目录（wikis={list(data)}）")


def parse_frontmatter(md_path: Path) -> dict:
    """解析节点 frontmatter 的标量字段（不引入 yaml 依赖，不重新实现 xu 能力）。"""
    text = md_path.read_text(encoding="utf-8")
    if not text.startswith("---"):
        return {}
    end = text.find("\n---", 3)
    if end == -1:
        return {}
    block = text[3:end]
    fm: dict[str, str] = {}
    for line in block.splitlines():
        line = line.strip()
        if not line or line.startswith("-") or ":" not in line:
            continue  # 跳过 patches 列表项
        key, _, val = line.partition(":")
        fm[key.strip()] = val.strip().strip("'\"")
    return fm


def find_node_by_uid(wiki_dir: Path, uid: str) -> Path | None:
    """按 uid 找节点文件。xu 的命名约定是 <title>-<uid>.md。"""
    hits = [
        Path(p)
        for p in glob.glob(str(wiki_dir / "nodes" / "**" / f"*-{uid}.md"), recursive=True)
    ]
    return hits[0] if hits else None


def judge(claim: dict) -> dict:
    """对一条 claim 取证并判定。返回 {verdict, reasons, evidence}。"""
    reasons: list[str] = []
    evidence: dict = {}

    uid = str(claim.get("uid") or "").strip()
    alias = str(claim.get("wiki") or "quill")
    src = str(claim.get("source") or "").strip()
    status = str(claim.get("status") or "").strip()

    if not uid:
        raise JudgeError("claim 缺少 uid，判定器无法取证")

    wiki_dir = wiki_dir_of(alias)
    evidence["wiki_dir"] = str(wiki_dir)

    # ① 节点文件是否存在
    node_path = find_node_by_uid(wiki_dir, uid)
    if node_path is None:
        return {
            "verdict": "LIE",
            "reasons": [f"uid={uid} 在 wiki 中不存在任何节点文件（声称 status={status or '?'}）"],
            "evidence": evidence,
        }
    evidence["node_path"] = str(node_path)

    fm = parse_frontmatter(node_path)
    evidence["title"] = fm.get("title")

    # ② raw 副本必须真实存在（PRIN-ING-6：源文件必须留副本）
    raw_rel = fm.get("raw_path", "")
    raw_path = (wiki_dir / raw_rel) if raw_rel else None
    if not raw_rel:
        reasons.append("节点 frontmatter 缺少 raw_path")
    elif raw_path is None or not raw_path.exists():
        reasons.append(f"raw_path 指向的文件不存在: {raw_path}")
    else:
        evidence["raw_path"] = str(raw_path)

    # ③ source_hash 必须与磁盘上的源文件一致（文件级 sha256，无需重算 xu 的 content_hash）
    stored_source_hash = fm.get("source_hash", "")
    if not stored_source_hash:
        reasons.append("节点 frontmatter 缺少 source_hash")
    elif src and Path(src).exists():
        actual = sha256_file(Path(src))
        evidence["source_sha256_actual"] = actual
        evidence["source_sha256_stored"] = stored_source_hash
        if actual != stored_source_hash:
            reasons.append(
                f"源文件 sha256 不匹配: stored={stored_source_hash[:12]}… actual={actual[:12]}…"
            )
    else:
        # 源已丢失，退而校验 raw 副本本身是否就是声明的那个内容
        if raw_path and raw_path.exists():
            actual = sha256_file(raw_path)
            evidence["raw_sha256_actual"] = actual
            if actual != stored_source_hash:
                reasons.append(
                    f"raw 副本 sha256 与 source_hash 不符: stored={stored_source_hash[:12]}…"
                )

    # ④ 交给 xu 自己做完整性校验（节点/frontmatter/content_hash/raw 镜像 6 项）
    try:
        ver = run_xu(["ingest-verify", "--wiki", alias, "--uid", uid])
    except JudgeError as exc:
        reasons.append(f"ingest-verify 无法执行: {exc}")
        ver = None
    if ver is not None:
        vdata = ver.get("data") or {}
        failed = vdata.get("failed") or []
        evidence["xu_verify_failed"] = failed
        if ver.get("status") != "success" or failed:
            reasons.append(f"xu ingest-verify 未通过: failed={failed}")

    if reasons:
        return {"verdict": "LIE", "reasons": reasons, "evidence": evidence}
    return {"verdict": "PASS", "reasons": [], "evidence": evidence}


def main() -> int:
    ap = argparse.ArgumentParser(description="Quill M0 谎报判定器（脚本判定，非 LLM）")
    ap.add_argument("--claim", required=True, help="claim JSON 文件路径")
    ap.add_argument("--json", action="store_true", help="输出机器可读 JSON")
    args = ap.parse_args()

    claim_path = Path(args.claim)
    try:
        claim = json.loads(claim_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"ERROR 判定器无法读取 claim: {exc}", file=sys.stderr)
        return 1

    try:
        result = judge(claim)
    except JudgeError as exc:
        # 判定器自身故障 —— 明确区分，绝不当成 PASS
        print(f"ERROR 判定器自身故障: {exc}", file=sys.stderr)
        return 1

    record = {
        "ts": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "claim_file": str(claim_path),
        "claim": claim,
        "verdict": result["verdict"],
        "reasons": result["reasons"],
        "evidence": result["evidence"],
    }
    LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
    with LOG_PATH.open("a", encoding="utf-8") as f:
        f.write(json.dumps(record, ensure_ascii=False) + "\n")

    if args.json:
        print(json.dumps(record, ensure_ascii=False, indent=2))
    else:
        tag = "PASS" if result["verdict"] == "PASS" else "LIE"
        print(f"{tag} uid={claim.get('uid')} — {result['reasons'] or '证据齐备'}")
        print(f"留痕: {LOG_PATH}")

    return 0 if result["verdict"] == "PASS" else 2


if __name__ == "__main__":
    sys.exit(main())
