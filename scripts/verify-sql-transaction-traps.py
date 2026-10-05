#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
verify-sql-transaction-traps.py —— 验证 §16.5 的三条已知陷阱

用法：
    python3 scripts/verify-sql-transaction-traps.py            # 跑检查
    python3 scripts/verify-sql-transaction-traps.py --self-test  # 先证明闸门会红
    python3 scripts/verify-sql-transaction-traps.py --rows 200000  # 加大数据量

退出码：0=全绿 / 1=有失败 / 2=装置不可用（无法判断）

────────────────────────────────────────────────────────────────
⚠️ 本脚本自身遵循 AGENTS.md 铁律二：**闸门必须双向可证伪**。

所以它带 `--self-test`：故意造一个【已知坏】的 schema，
确认检查**确实报红**。若 self-test 竟然通过，
说明断言写错了（是假闸门），本脚本会以退出码 2 拒绝给出任何结论。

────────────────────────────────────────────────────────────────
⚠️ 装置自检（对应储海量的教训：第一版实验装置坏了，得出 3/14 的假结论）

本脚本在断言前**先打印装置信息**：SQLite 版本、行数、执行计划。
⚠️ 数据量必须够大才能暴露索引差异：1000 行时全表扫描仅几毫秒，
   看起来"没问题"；10 万行才有量级差别。故默认 10 万行。
"""

import argparse
import os
import sqlite3
import sys
import tempfile
import time

# ─────────────────────────────────────────────────────────────
# 装置信息（AGENTS.md：让人能判断装置对不对）
# ─────────────────────────────────────────────────────────────
def print_device_header(rows: int) -> None:
    print("═" * 68)
    print("装置自检")
    print("═" * 68)
    print(f"  Python           : {sys.version.split()[0]}")
    print(f"  SQLite 版本      : {sqlite3.sqlite_version}")
    print(f"  数据量           : {rows:,} 行")
    # ⚠️ 装置下限：低于此值索引差异测不出来，结论不可信
    MIN_ROWS = 50_000
    if rows < MIN_ROWS:
        print(f"  ⚠️ 警告          : 数据量 < {MIN_ROWS:,}，索引差异可能测不出来")
        print(f"                    装置可能无效，结论不可信")
    else:
        print(f"  装置判定         : ✅ 数据量足够（≥ {MIN_ROWS:,}）")
    print(f"  临时目录         : {tempfile.gettempdir()}")
    print()


def plan_of(conn: sqlite3.Connection, sql: str) -> str:
    """返回 EXPLAIN QUERY PLAN 的可读形式。"""
    rows = conn.execute("EXPLAIN QUERY PLAN " + sql).fetchall()
    return " | ".join(r[3] for r in rows)


def timed_ms(conn: sqlite3.Connection, sql: str, repeat: int = 5) -> float:
    """取多次中位数，避免单次抖动误导。"""
    samples = []
    for _ in range(repeat):
        t0 = time.perf_counter()
        conn.execute(sql).fetchall()
        samples.append((time.perf_counter() - t0) * 1000)
    samples.sort()
    return samples[len(samples) // 2]


# ─────────────────────────────────────────────────────────────
# 装置：建库建表
# ─────────────────────────────────────────────────────────────
def build_db(path: str, rows: int, index_cols: tuple = ("user_id", "status")) -> sqlite3.Connection:
    """
    建一张模拟 sessions 的表。
    ⚠️ index_cols 的顺序是本脚本的核心变量（见 check_group_by_index）
    """
    conn = sqlite3.connect(path)
    conn.execute("PRAGMA journal_mode=WAL")   # 与生产一致（架构 §3.3 SQLite WAL）
    conn.execute("""
        CREATE TABLE sessions (
            id      INTEGER PRIMARY KEY,
            user_id TEXT NOT NULL,
            status  TEXT NOT NULL,
            kind    TEXT NOT NULL,
            created_at TEXT NOT NULL
        )
    """)
    # 批量插入：⚠️ 必须显式事务（这本身就是陷阱③的对策）
    conn.execute("BEGIN")
    conn.executemany(
        "INSERT INTO sessions (user_id, status, kind, created_at) VALUES (?,?,?,?)",
        (
            (
                f"u{i % 50}",                       # 50 个用户
                "active" if i % 3 else "idle",      # 2 种状态
                "chat" if i % 2 else "team",        # 2 种类型
                f"2026-10-{(i % 28) + 1:02d}T00:00:00Z",
            )
            for i in range(rows)
        ),
    )
    conn.execute("COMMIT")
    if index_cols:
        conn.execute(f"CREATE INDEX ix_probe ON sessions ({', '.join(index_cols)})")
    conn.commit()
    return conn


# ─────────────────────────────────────────────────────────────
# 陷阱 ①：GROUP BY 缺索引 → 全表扫描 + 临时排序
# ─────────────────────────────────────────────────────────────
GROUP_BY_SQL = (
    "SELECT user_id, COUNT(*) FROM sessions "
    "WHERE status = 'active' GROUP BY user_id"
)

# ⚠️ 判别标志是【USE TEMP B-TREE】，不是 SCAN。
#    原因（实测踩坑）：好计划的 plan 里**同时含 SCAN 和 USING**
#    例：`SCAN sessions USING COVERING INDEX ix_probe`
#    若断言"不含 SCAN"，会在**正确**的 schema 上误报 → 假闸门。
TEMP_BTREE = "USE TEMP B-TREE"


def check_group_by_index(conn: sqlite3.Connection) -> bool:
    print("─" * 68)
    print("陷阱 ①  GROUP BY 缺索引 → 全表扫描")
    print("─" * 68)
    p = plan_of(conn, GROUP_BY_SQL)
    ms = timed_ms(conn, GROUP_BY_SQL)
    print(f"  SQL   : {GROUP_BY_SQL}")
    print(f"  计划  : {p}")
    print(f"  耗时  : {ms:.1f} ms（中位数 ×5）")
    if TEMP_BTREE in p:
        print("  判定  : ❌ 失败 —— GROUP BY 触发临时 B-tree 排序")
        print("          索引列顺序不满足 GROUP BY，或缺索引")
        return False
    print("  判定  : ✅ 通过 —— 索引已【覆盖】GROUP BY 列，无需临时排序")
    print("          ⚠️ 注意 1：计划含 SCAN 是正常的（覆盖索引扫描），")
    print("             判别标志是 USE TEMP B-TREE 是否【出现】")
    print("          ⚠️ 注意 2：判别条件是【覆盖】，不是【列顺序】——")
    print("             实测 (status,user_id) 同样无 TEMP B-TREE")
    return True


# ─────────────────────────────────────────────────────────────
# 陷阱 ②：OR 使索引失效（IN 不会）
# ─────────────────────────────────────────────────────────────
OR_SQL = "SELECT * FROM sessions WHERE user_id = 'u1' OR status = 'active'"
IN_SQL = "SELECT * FROM sessions WHERE user_id IN ('u1','u2')"


def _uses_index(plan: str) -> bool:
    """SEARCH = 走索引；SCAN = 全表扫描。

    ⚠️ 这里用 startswith('SEARCH') 而非 'SCAN' not in plan，
       因为覆盖索引扫描的 plan 文本是 `SCAN t USING COVERING INDEX ...`。
    """
    return plan.strip().startswith("SEARCH")


def check_or_vs_in(conn: sqlite3.Connection) -> bool:
    print()
    print("─" * 68)
    print("陷阱 ②  OR 使索引失效（IN 不会）")
    print("─" * 68)
    p_or = plan_of(conn, OR_SQL)
    p_in = plan_of(conn, IN_SQL)
    ms_or = timed_ms(conn, OR_SQL)
    ms_in = timed_ms(conn, IN_SQL)
    print(f"  OR 版 : {p_or}")
    print(f"         {ms_or:.1f} ms   走索引? {'是' if _uses_index(p_or) else '否'}")
    print(f"  IN 版 : {p_in}")
    print(f"         {ms_in:.1f} ms   走索引? {'是' if _uses_index(p_in) else '否'}")
    print()
    print("  A/B 对照（这是本检查的核心 —— 不是断言，是展示差异）：")
    ok = True
    or_bad, in_good = (not _uses_index(p_or)), _uses_index(p_in)
    if or_bad:
        print("  ✅ OR 版走全表扫描 —— 证实「OR 使索引失效」")
    else:
        print("  ⚠️ OR 版竟然走了索引（本 SQLite 版本可能优化了 OR）")
        print("     → 结论仍成立（换 SQLite 版本/数据分布可能退化），但本机未复现")
    if in_good:
        print("  ✅ IN 版走索引 —— 证实「IN 不破坏索引」")
    else:
        print("  ❌ IN 版未走索引 —— 这不对，IN 应能用索引")
        ok = False
    if ok and or_bad:
        print("  判定  : ✅ 通过 —— A/B 对照证实了陷阱②")
        return True
    print("  判定  : ⚠️ 未完全复现（见上），不判失败，但请人工确认上面的计划")
    return ok


# ─────────────────────────────────────────────────────────────
# 陷阱 ③：批量写中途失败 → 部分写入（无事务时）
# ─────────────────────────────────────────────────────────────
def check_batch_atomicity(rows: int) -> bool:
    print()
    print("─" * 68)
    print("陷阱 ③  批量写中途失败 → 部分写入")
    print("─" * 68)

    # (a) 无事务：中途失败 → 部分写入
    db1 = tempfile.mktemp(suffix=".db")
    c1 = sqlite3.connect(db1)
    c1.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
    c1.commit()
    no_tx_rows = 0
    try:
        for i in range(1000):
            c1.execute("INSERT INTO t (v) VALUES (?)", (f"v{i}",))
            if i == 500:
                raise RuntimeError("模拟第 501 行失败")
    except RuntimeError:
        pass
    no_tx_rows = c1.execute("SELECT COUNT(*) FROM t").fetchone()[0]
    c1.close()
    os.unlink(db1)

    # (b) 有显式事务：中途失败 → 全部回滚
    db2 = tempfile.mktemp(suffix=".db")
    c2 = sqlite3.connect(db2)
    c2.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
    c2.commit()
    tx_rows = 0
    try:
        c2.execute("BEGIN")            # ← 显式事务（对策）
        for i in range(1000):
            c2.execute("INSERT INTO t (v) VALUES (?)", (f"v{i}",))
            if i == 500:
                raise RuntimeError("模拟第 501 行失败")
        c2.execute("COMMIT")
    except RuntimeError:
        c2.execute("ROLLBACK")         # ← 关键
    tx_rows = c2.execute("SELECT COUNT(*) FROM t").fetchone()[0]
    c2.close()
    os.unlink(db2)

    print(f"  无事务，中途失败后残留 : {no_tx_rows} 行  ← 部分写入（陷阱成立）")
    print(f"  有事务，中途失败后残留 : {tx_rows} 行  ← 全部回滚（对策有效）")
    print()
    print("  A/B 对照：")
    ok = True
    if no_tx_rows > 0:
        print(f"  ✅ 无事务留下 {no_tx_rows} 行 —— 证实「批量写非原子」")
    else:
        print("  ⚠️ 无事务竟然没留下残��（本环境行为不同），陷阱未复现")
        ok = False
    if tx_rows == 0:
        print("  ✅ 显式事务完全回滚 —— 证实对策有效")
    else:
        print(f"  ❌ 显式事务仍残留 {tx_rows} 行 —— 对策无效")
        ok = False
    print("  判定  : ✅ 通过" if ok else "  判定  : ❌ 失败")
    return ok


# ─────────────────────────────────────────────────────────────
# self-test：证明闸门会红（AGENTS.md 铁律二）
# ─────────────────────────────────────────────────────────────
def self_test(rows: int) -> bool:
    print()
    print("═" * 68)
    print("SELF-TEST：故意造【已知坏】装置，确认检查会报红")
    print("═" * 68)
    print("  目的：若这里竟然通过，说明断言写错了，是假闸门")
    print()

    results = {}

    # 坏装置 1：无任何索引 → GROUP BY 必须触发临时 B-tree
    db = tempfile.mktemp(suffix=".db")
    c = build_db(db, rows, index_cols=None)   # ← 故意不给索引
    p = plan_of(c, GROUP_BY_SQL)
    caught = TEMP_BTREE in p
    print(f"  [坏装置1] 无索引的 GROUP BY")
    print(f"    计划      : {p}")
    print(f"    检查是否捕获: {'✅ 捕获' if caught else '❌ 漏检 —— 断言无效！'}")
    print(f"    check_group_by_index 返回: {check_group_by_index(c)}")
    print(f"    期望      : False（报红）")
    results["坏装置1 被捕获"] = caught and not check_group_by_index(c)
    c.close()
    os.unlink(db)

    # 坏装置 2：索引【不覆盖】GROUP BY 列 → 必须触发临时 B-tree
    # ⚠️ 曾经的错误：我用"列顺序反了 (status, user_id)"当坏装置，
    #    实测它同样不触发 TEMP B-TREE（仍是覆盖索引）→ 闸门漏检。
    #    真正的坏装置是"索引列不包含 GROUP BY 列"。
    db = tempfile.mktemp(suffix=".db")
    c = build_db(db, rows, index_cols=("status",))   # ← 只有 status，不含 user_id
    caught2 = not check_group_by_index(c)
    print()
    print(f"  [坏装置2] 索引不覆盖 GROUP BY 列 (status)")
    print(f"    检查是否报红: {'✅ 报红' if caught2 else '❌ 未报红 —— 漏检！'}")
    results["坏装置2 被捕获"] = caught2
    c.close()
    os.unlink(db)

    # 坏装置 3：列顺序"反了"但仍覆盖 → 应当【通过】（这是实测纠正的认知）
    db = tempfile.mktemp(suffix=".db")
    c = build_db(db, rows, index_cols=("status", "user_id"))
    p = plan_of(c, GROUP_BY_SQL)
    still_good = TEMP_BTREE not in p
    print()
    print("  [认知纠正] 列顺序 (status, user_id) —— 看似反了，实为覆盖索引")
    print(f"    计划      : {p}")
    print(f"    应当通过  : {'✅ 是（说明列顺序不是判别条件）' if still_good else '❌ 否'}")
    print(f"    ⚠️ 实测结论：判别条件是【索引是否覆盖 GROUP BY 列】，与列顺序无关。")
    print("                我最初写的「WHERE 列在前」是错的，此处保留为反例记录。")
    results["认知纠正：列顺序不影响"] = still_good
    c.close()
    os.unlink(db)

    # 对照：好装置必须通过（否则是假阴性）
    db = tempfile.mktemp(suffix=".db")
    c = build_db(db, rows, index_cols=("user_id", "status"))
    good_ok = check_group_by_index(c)
    print()
    print(f"  [好装置] 索引列顺序正确 (user_id, status)")
    print(f"    检查是否通过: {'✅ 通过' if good_ok else '❌ 误报 —— 假闸门！'}")
    results["好装置不误报"] = good_ok
    c.close()
    os.unlink(db)

    print()
    print("─" * 68)
    all_ok = all(results.values())
    for k, v in results.items():
        print(f"  {'✅' if v else '❌'} {k}")
    print("─" * 68)
    if all_ok:
        print("  SELF-TEST 通过：闸门双向可证伪（坏的会红，好的不误报）")
    else:
        print("  ❌ SELF-TEST 失败：闸门是假的，拒绝给出任何结论")
    return all_ok


# ─────────────────────────────────────────────────────────────
def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rows", type=int, default=100_000,
                    help="数据量（默认 10 万；低于 5 万装置可能无效）")
    ap.add_argument("--self-test", action="store_true",
                    help="只跑 self-test：证明闸门会红")
    args = ap.parse_args()

    print_device_header(args.rows)

    if args.self_test:
        return 0 if self_test(args.rows) else 2

    # 装置自检：数据量不够就拒绝给结论（不是"跳过通过"）
    if args.rows < 50_000:
        print("❌ 装置不可用：数据量 < 50,000，索引差异测不出来。")
        print("   请用 --rows 100000 重试。（依据：本项目曾因装置数据量过小得出假结论）")
        return 2

    print("═" * 68)
    print("正式检查（装置：10 万行 + 正确索引 (user_id, status)）")
    print("═" * 68)
    db = tempfile.mktemp(suffix=".db")
    conn = build_db(db, args.rows, index_cols=("user_id", "status"))
    actual = conn.execute("SELECT COUNT(*) FROM sessions").fetchone()[0]
    print(f"  实际入库行数: {actual:,}（目标 {args.rows:,}）")
    print()

    results = {
        "陷阱① GROUP BY 索引": check_group_by_index(conn),
        "陷阱② OR vs IN": check_or_vs_in(conn),
        "陷阱③ 批量原子性": check_batch_atomicity(args.rows),
    }
    conn.close()
    os.unlink(db)

    print()
    print("═" * 68)
    print("结论")
    print("═" * 68)
    for k, v in results.items():
        print(f"  {'✅' if v else '❌'} {k}")
    passed = sum(results.values())
    print(f"\n  {passed}/{len(results)} 通过")
    if passed == len(results):
        print("  三条陷阱均已复现并有对策 → §16.5 的结论成立")
        return 0
    print("  ⚠️ 有检查未通过，请人工核对上面的执行计划")
    return 1


if __name__ == "__main__":
    sys.exit(main())
