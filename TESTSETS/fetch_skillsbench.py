#!/usr/bin/env python3
"""Download all files of the first N SkillsBench task packages, including input fixtures.

Why this exists: the previous download only fetched ``task.md`` and ``SKILL.md``,
which made every task look like it had zero input files. That was wrong --
the upstream repo really does ship the input data (``packets.pcap``, ``wave.mseed``,
``handbook.pdf``, ``sensor_data.csv`` and so on). Only the download was broken.

Enumeration goes through the HuggingFace tree API with cursor pagination; every
path is fetched from ``resolve/main``, which redirects LFS objects to the CDN
(urllib follows it). No token required.
"""
import json
import pathlib
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed

API = "https://huggingface.co/api/datasets/benchflow/skillsbench/tree/main?recursive=true"
RAW = "https://huggingface.co/datasets/benchflow/skillsbench/resolve/main"
UA = {"User-Agent": "quill-testsets"}

N_TASKS = 50
WORKERS = 8
RETRIES = 3


def fetch_json(url):
    req = urllib.request.Request(url, headers=UA)
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read().decode("utf-8")), r.headers.get("Link", "")


def list_all_files():
    """Walk the tree API across all pages. Returns [(path, size)]."""
    url = API
    files = []
    page = 0
    while url and page < 60:
        body, link = fetch_json(url)
        for e in body:
            if e.get("type") == "file":
                files.append((e["path"], int(e.get("size") or 0)))
        page += 1
        nxt = ""
        for part in link.split(","):
            part = part.strip()
            if 'rel="next"' in part:
                nxt = part.split(";")[0].strip().lstrip("<").rstrip(">")
        url = nxt
        print(f"  page {page}: {len(files)} files so far", flush=True)
    return files


def download(rel, dest_root):
    """Download one file. Returns (rel, bytes, error_or_None)."""
    dest = dest_root / rel
    if dest.exists() and dest.stat().st_size >= 0:
        # Re-download only when the local file is empty but should not be.
        return rel, dest.stat().st_size, None
    dest.parent.mkdir(parents=True, exist_ok=True)
    url = f"{RAW}/{urllib.request.quote(rel)}"
    last = None
    for attempt in range(RETRIES):
        try:
            req = urllib.request.Request(url, headers=UA)
            with urllib.request.urlopen(req, timeout=180) as r:
                data = r.read()
            dest.write_bytes(data)
            return rel, len(data), None
        except (urllib.error.URLError, urllib.error.HTTPError, OSError) as e:
            last = e
            time.sleep(1.5 * (attempt + 1))
    return rel, 0, f"{type(last).__name__}: {last}"


def main():
    raw = pathlib.Path(__file__).resolve().parent / "_raw"
    out = raw / "skillsbench"
    out.mkdir(parents=True, exist_ok=True)

    print("== enumerating SkillsBench tree ==")
    files = list_all_files()
    (raw / "skillsbench-files.json").write_text(
        json.dumps([{"path": p, "size": s} for p, s in files], ensure_ascii=False, indent=1),
        encoding="utf-8",
    )
    total_bytes = sum(s for _, s in files)
    tasks = sorted({p.split("/")[0] for p, _ in files if p.endswith("/task.md")})
    print(f"  files={len(files)} ({total_bytes / 1048576:.1f} MB), task packages={len(tasks)}")

    picked = set(tasks[:N_TASKS])
    want = [(p, s) for p, s in files if p.split("/")[0] in picked]
    want_bytes = sum(s for _, s in want)
    print(f"== downloading first {len(picked)} task packages ==")
    print(f"  {len(want)} files, {want_bytes / 1048576:.1f} MB")

    ok = 0
    got = 0
    failed = []
    done_bytes = 0
    with ThreadPoolExecutor(max_workers=WORKERS) as pool:
        futs = {pool.submit(download, rel, out): (rel, size) for rel, size in want}
        for fut in as_completed(futs):
            rel, size, err = fut.result()
            if err:
                failed.append((rel, err))
            else:
                ok += 1
                done_bytes += size
            if (ok + len(failed)) % 100 == 0:
                print(f"  {ok + len(failed)}/{len(want)}  {done_bytes / 1048576:.1f} MB", flush=True)

    print(f"== done: {ok} ok, {len(failed)} failed, {done_bytes / 1048576:.1f} MB ==")
    if failed:
        print("  failures (first 20):", file=sys.stderr)
        for rel, err in failed[:20]:
            print(f"    {rel}  {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())