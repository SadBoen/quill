"""Quill M0 闸门自检 —— 证明门「会变红」。

只全绿的门等于没有门（AGENTS.md 五类失败模式第 1 类：假闸门）。
本脚本用已知坏输入反向证伪，四条用例：

  T1 门禁环境缺失   : XU_BIN 指向不存在 → check.sh 必须 rc=127 且输出 FAIL
  T2 判定器不许兜底 : XU_BIN 指向不存在 → 判定器必须 rc=1(ERROR)，绝不能 rc=0(PASS)
  T3 谎报必被抓     : 伪造 uid 的 claim → 判定器必须 rc=2(LIE)
  T4 源文件丢失必被抓: 真实提交过、但 raw 副本被删掉的 claim → 判定器必须 rc=2(LIE)
                      （这正是历史上「Agent 声称摄取了、实际源文件丢了」的失败模式）

T4 在独立的临时 wiki 上进行，不污染主 wiki；结束即清理。

退出码: 0=四条全过, 1=有用例未通过
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
XU_BIN = os.environ.get(
    "XU_BIN", r"C:/Users/Boen/.workbuddy/binaries/python/envs/quill/Scripts/xu.exe"
)
PY = sys.executable
CHECK_SH = REPO_ROOT / "check.sh"
JUDGE = Path(__file__).resolve().parent / "judge_claim.py"

BOGUS_XU = "C:/nonexistent/xu-does-not-exist.exe"

results: list[tuple[bool, str, str]] = []


def add(ok: bool, name: str, detail: str = "") -> None:
    results.append((bool(ok), name, detail))
    print(f"{'PASS' if ok else 'FAIL'} {name}" + (f" — {detail}" if detail else ""))


def run(cmd: list[str], env: dict | None = None, cwd: str | None = None) -> subprocess.CompletedProcess:
    e = dict(os.environ)
    # 强制 UTF-8：Git Bash 的原生 bash.exe 会按控制台代码页(GBK)输出中文，
    # 直接按 utf-8 解码会炸，导致"跑不起来"被误读成"没输出"。
    e.setdefault("LC_ALL", "C.UTF-8")
    e.setdefault("PYTHONIOENCODING", "utf-8")
    e.setdefault("PYTHONUTF8", "1")
    if env:
        e.update(env)
    return subprocess.run(
        cmd, capture_output=True, text=True, encoding="utf-8", errors="replace",
        env=e, cwd=cwd or str(REPO_ROOT), timeout=180,
    )


def xu(args: list[str]) -> dict:
    p = run([str(XU_BIN), *args])
    try:
        return json.loads(p.stdout)
    except json.JSONDecodeError:
        return {"status": "error", "message": p.stdout[:200] + p.stderr[:200]}


def t1_gate_fails_when_xu_missing() -> None:
    # 用 cwd 相对路径调用：实测被 Windows 原生进程拉起的 Git Bash 解析不了
    # 绝对盘符路径（报 No such file or directory），相对路径正常。
    p = run(["bash", "check.sh"], cwd=str(REPO_ROOT), env={"XU_BIN": BOGUS_XU})
    out = (p.stdout or "") + (p.stderr or "")
    ok = p.returncode == 127 and "FAIL" in out
    add(ok, "T1 check.sh 环境被破坏时必变红（不只是不绿）", f"rc={p.returncode} out={out.strip()[:80]}")


def t2_judge_never_defaults_to_pass() -> None:
    claim = Path(tempfile.mkdtemp()) / "c.json"
    claim.write_text(json.dumps({"wiki": "quill", "uid": "ANY00000", "status": "completed"}), encoding="utf-8")
    p = run([PY, str(JUDGE), "--claim", str(claim)], env={"XU_BIN": BOGUS_XU})
    # rc=1 是「判定器自身故障」，必须与 rc=0(PASS) 严格区分
    ok = p.returncode == 1
    add(ok, "T2 判定器缺 xu 时报 ERROR 而非 PASS", f"rc={p.returncode} err={p.stderr.strip()[:80]}")


def t3_fabricated_uid_caught() -> None:
    claim = Path(tempfile.mkdtemp()) / "c.json"
    claim.write_text(
        json.dumps({"wiki": "quill", "uid": "NOPE0001", "title": "凭空捏造", "status": "completed"}),
        encoding="utf-8",
    )
    p = run([PY, str(JUDGE), "--claim", str(claim)])
    ok = p.returncode == 2
    add(ok, "T3 伪造 uid 被判 LIE", f"rc={p.returncode} {p.stdout.strip().splitlines()[0] if p.stdout.strip() else ''}")


def t4_lost_raw_caught() -> None:
    """真实提交 → 删掉 raw 副本 → 声称已完成 → 必须被抓。"""
    tmp_root = REPO_ROOT / "_scratch" / "wiki-lietest"
    name = "quill-lietest"
    if tmp_root.exists():
        shutil.rmtree(tmp_root)
    xu(["unregister", "--name", name])
    xu(["create", "--name", name, "--path", str(tmp_root)])

    src = REPO_ROOT / "_scratch" / "sources" / "m0-lietest.md"
    src.parent.mkdir(parents=True, exist_ok=True)
    src.write_text("# 临时取证样本\n\n用于验证 raw 丢失会被判定器抓住。\n", encoding="utf-8")

    p1 = xu(["ingest-file", "--wiki", name, "--file", str(src), "--title", "临时取证样本", "--node-path", "t"])
    d1 = p1.get("data") or {}
    if p1.get("status") != "success":
        add(False, "T4 raw 丢失被判 LIE", f"临时 wiki 摄取失败: {p1.get('message')}")
        return

    p2 = xu(["ingest-commit", "--wiki", name, "--temp", d1["temp"], "--title", "临时取证样本",
             "--content-type", "article", "--source", str(src), "--node-path", "t"])
    created = (p2.get("data") or {}).get("created") or []
    if not created:
        add(False, "T4 raw 丢失被判 LIE", f"临时 wiki 提交失败: {p2.get('message')}")
        return
    uid = created[0]["uid"]

    # 先确认：raw 还在时判定应为 PASS（反向用例，证明判定器不是无脑判 LIE）
    good = Path(tempfile.mkdtemp()) / "ok.json"
    good.write_text(json.dumps({"wiki": name, "uid": uid, "source": str(src), "status": "completed"}), encoding="utf-8")
    p_ok = run([PY, str(JUDGE), "--claim", str(good)])
    add(p_ok.returncode == 0, "T4a raw 完好时判 PASS（反向用例）", f"rc={p_ok.returncode}")

    # 删掉 raw 副本 —— 模拟「源文件丢了」
    raw_rel = created[0].get("raw_path", "")
    raw_path = tmp_root / raw_rel if raw_rel else None
    deleted = False
    if raw_path and raw_path.exists():
        raw_path.unlink()
        deleted = True

    bad = Path(tempfile.mkdtemp()) / "bad.json"
    bad.write_text(json.dumps({"wiki": name, "uid": uid, "source": str(src), "status": "completed"}), encoding="utf-8")
    p_bad = run([PY, str(JUDGE), "--claim", str(bad)])
    detail = p_bad.stdout.strip().splitlines()[0] if p_bad.stdout.strip() else p_bad.stderr.strip()[:80]
    add(deleted and p_bad.returncode == 2, "T4b raw 丢失被判 LIE", f"deleted={deleted} rc={p_bad.returncode} {detail[:90]}")

    # 清理临时 wiki，避免污染注册表
    xu(["unregister", "--name", name])
    if tmp_root.exists():
        shutil.rmtree(tmp_root, ignore_errors=True)


def main() -> int:
    print("Quill M0 闸门自检（反向证伪：证明门会变红）")
    print("-" * 60)
    t1_gate_fails_when_xu_missing()
    t2_judge_never_defaults_to_pass()
    t3_fabricated_uid_caught()
    t4_lost_raw_caught()
    print("-" * 60)
    failed = [n for ok, n, _ in results if not ok]
    if failed:
        print(f"FAIL — 自检未通过: {', '.join(failed)}")
        return 1
    print(f"OK — 闸门自检 {len(results)}/{len(results)} 通过（门确实会变红）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
