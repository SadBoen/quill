"""隔离性与约束行为实测 —— 每一条断言都对应文档里的一条设计声明。

运行：python3 crates/quill-store/verify/verify.py   退出码 0 = 全部通过
路径全部由 __file__ 推算，因此可在任意 cwd、任意 clone 位置运行。
被测对象是**真实迁移文件** migrations/0001_init.sql，不是它的副本 ——
副本一旦漂移，全部 41 项断言会静默地测错文件。
"""
import sqlite3, sys, time
from pathlib import Path

SCHEMA = Path(__file__).resolve().parent.parent / 'migrations' / '0001_init.sql'

con = sqlite3.connect(':memory:')
con.execute("PRAGMA foreign_keys=ON")
con.executescript(SCHEMA.read_text(encoding='utf-8'))

U  = bytes.fromhex('aa'*16)
Ub = bytes.fromhex('bb'*16)
SA = bytes.fromhex('01'*16)
SB = bytes.fromhex('02'*16)
TA = bytes.fromhex('cc'*16)
N  = bytes.fromhex('0a'*16)
NOW = 1_700_000_000_000
H32 = b'x'*32

# `users.password_algo` 曾经有 DEFAULT 'argon2id'，但实现跑的是 PBKDF2——
# 那是一份会骗人的 schema，默认值已删除（漏写该列 = 立即 INSERT 失败）。
# 故本脚本的每条 users INSERT 都必须**显式**给出算法串。
# 取值格式由 quill-control 的 `PasswordHasher::algo_tag()` 产出。
ALGO = 'pbkdf2-hmac-sha256$i=600000'

results = []
def check(name, fn, expect):
    try:
        fn(); got = 'ALLOWED'
    except sqlite3.Error:
        got = 'BLOCKED'
    ok = expect in got
    results.append((ok, name, got))
    print(f"[{'PASS' if ok else 'FAIL'}] {name}\n         -> {got}")

def seed():
    for uid, un, role in [(U,'alice','owner'), (Ub,'bob','member')]:
        con.execute("""INSERT INTO users(id,username,username_norm,display_name,
            password_hash,password_salt,password_algo,role,pwd_changed_at,created_at,updated_at)
            VALUES(?,?,?,?,?,?,?,?,?,?,?)""",
            (uid, un, un, un.title(), b'h', b's'*16, ALGO, role, NOW, NOW, NOW))
    for uid, sid, un in [(U, SA, 'alice'), (Ub, SB, 'bob')]:
        con.execute("""INSERT INTO sessions(user_id,id,kind,room_id,workspace_path,
            created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?)""",
            (uid, sid, 'solo', f'leader_session:{un}:{sid.hex()}', f'sessions/{sid.hex()}', NOW, NOW, NOW))
    con.execute("""INSERT INTO teams(user_id,id,name,room_id,leader_session_id,
        leader_expert_id,state_changed_at,created_at,updated_at)
        VALUES(?,?,?,?,?,?,?,?,?)""",
        (U, TA, 'A队', f'leader_session:alice:{SA.hex()}', SA, 'lead', NOW, NOW, NOW))
seed()

def hdr(t): print("\n" + "="*76 + f"\n{t}\n" + "="*76)

hdr("A. 跨用户复合外键 —— 「忘了加 where user_id」的最后一道物理防线")
check("A 往 B 的 session 插入 message",
      lambda: con.execute("INSERT INTO messages(user_id,id,session_id,seq,role,content,created_at) VALUES(?,?,?,?,?,?,?)",
                          (U, bytes.fromhex('11'*16), SB, 1, 'user', 'x', NOW)), 'BLOCKED')
check("A 建 team 时把 B 的 session 当 leader",
      lambda: con.execute("INSERT INTO teams(user_id,id,name,room_id,leader_session_id,leader_expert_id,state_changed_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
                          (U, bytes.fromhex('77'*16), 'x', 'r', SB, 'e', NOW, NOW, NOW)), 'BLOCKED')
check("A 的 team_members 指向 B 的 session",
      lambda: con.execute("INSERT INTO team_members(user_id,team_id,expert_id,role,member_session_id,state,state_changed_at,joined_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
                          (U, TA, 'e1', 'member', SB, 'IDLE', NOW, NOW, NOW, NOW)), 'BLOCKED')
check("A 的 task_dispatch 指向 B 的 member session",
      lambda: con.execute("INSERT INTO task_dispatches(user_id,id,room_id,team_id,round,leader_session_id,member_session_id,member_expert_id,task_digest,state,dispatched_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                          (U, bytes.fromhex('31'*16), 'r', TA, 0, SA, SB, 'e1', H32, 'RUNNING', NOW, NOW, NOW)), 'BLOCKED')
# ⚠️ 已知缺口：wiki_index 无父行可挂复合外键（文档真相源是文件）。
# 写错 user_id 会静默落入错误命名空间 —— 由 §7.3 的 doc_id↔文件双向审计兜底。
check("[已知缺口] A 往 B 的 wiki_index 命名空间写词条 -> 记录实际行为",
      lambda: con.execute("INSERT INTO wiki_index(user_id,term,doc_id,term_kind,tf,field_len,rel_path,page_title,content_hash,bytes,indexed_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                          (Ub, 'x', bytes.fromhex('41'*16), 0, 1, 10, 'a.md', 'T', H32, 100, NOW)), 'ALLOWED')

hdr("B. slug / 路径 —— 目录穿越的物理阻断")
for bad in ['../etc', 'a/b', 'a\\b', 'Foo', 'a..b', '', 'a b', '.']:
    check(f"专家 id={bad!r}",
          lambda b=bad: con.execute("""INSERT INTO experts(id,owner_user_id,display_name,
              version,description,visibility,tool_policy_json,license,asset_hash,persona_hash,created_at,updated_at)
              VALUES(?,?,?,?,?,?,?,?,?,?,?,?)""",
              (b, U, 'n', '1', 'd', 'user_authored', '{}', 'MIT', H32, H32, NOW, NOW)), 'BLOCKED')
check("合法 slug 'ops-engineer'",
      lambda: con.execute("""INSERT INTO experts(id,owner_user_id,display_name,version,
          description,visibility,tool_policy_json,license,asset_hash,persona_hash,created_at,updated_at)
          VALUES(?,?,?,?,?,?,?,?,?,?,?,?)""",
          ('ops-engineer', U, 'n', '1', 'd', 'user_authored', '{}', 'MIT', H32, H32, NOW, NOW)), 'ALLOWED')
check("is_builtin=0 却用全零 owner UUID",
      lambda: con.execute("""INSERT INTO experts(id,owner_user_id,display_name,version,
          description,visibility,tool_policy_json,license,asset_hash,persona_hash,created_at,updated_at)
          VALUES(?,?,?,?,?,?,?,?,?,?,?,?)""",
          ('x1', bytes(16), 'n', '1', 'd', 'builtin_system', '{}', 'MIT', H32, H32, NOW, NOW)), 'BLOCKED')
check("专家 tool_policy_json 为 NULL（TP-1 必须显式声明）",
      lambda: con.execute("""INSERT INTO experts(id,owner_user_id,display_name,version,
          description,visibility,tool_policy_json,license,asset_hash,persona_hash,created_at,updated_at)
          VALUES(?,?,?,?,?,?,NULL,?,?,?,?,?)""",
          ('x2', U, 'n', '1', 'd', 'user_authored', 'MIT', H32, H32, NOW, NOW)), 'BLOCKED')
check("session workspace_path 绝对路径",
      lambda: con.execute("""INSERT INTO sessions(user_id,id,kind,room_id,workspace_path,
          created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?)""",
          (U, bytes.fromhex('91'*16), 'solo', 'r', '/etc/passwd', NOW, NOW, NOW)), 'BLOCKED')
check("session workspace_path 含 ..",
      lambda: con.execute("""INSERT INTO sessions(user_id,id,kind,room_id,workspace_path,
          created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?)""",
          (U, bytes.fromhex('92'*16), 'solo', 'r', '../../B/sessions/x', NOW, NOW, NOW)), 'BLOCKED')
check("wiki_index rel_path 含 ..",
      lambda: con.execute("INSERT INTO wiki_index(user_id,term,doc_id,term_kind,tf,field_len,rel_path,page_title,content_hash,bytes,indexed_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                          (U, 't', bytes.fromhex('42'*16), 0, 1, 5, '../B/secret.md', 'T', H32, 10, NOW)), 'BLOCKED')

hdr("C. 状态机不变量 —— 由 CHECK / UNIQUE 索引在数据库层强制")
check("DONE 但 settled_at 为空",
      lambda: con.execute("INSERT INTO task_dispatches(user_id,id,room_id,team_id,round,leader_session_id,member_session_id,member_expert_id,task_digest,state,dispatched_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                          (U, bytes.fromhex('32'*16), 'r', TA, 0, SA, SA, 'e1', H32, 'DONE', NOW, NOW, NOW)), 'BLOCKED')
check("RUNNING 却带 settled_at",
      lambda: con.execute("INSERT INTO task_dispatches(user_id,id,room_id,team_id,round,leader_session_id,member_session_id,member_expert_id,task_digest,state,dispatched_at,settled_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                          (U, bytes.fromhex('33'*16), 'r', TA, 0, SA, SA, 'e1', H32, 'RUNNING', NOW, NOW, NOW, NOW)), 'BLOCKED')
check("非法状态 'PANICKED'",
      lambda: con.execute("INSERT INTO task_dispatches(user_id,id,room_id,team_id,round,leader_session_id,member_session_id,member_expert_id,task_digest,state,dispatched_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                          (U, bytes.fromhex('34'*16), 'r', TA, 0, SA, SA, 'e1', H32, 'PANICKED', NOW, NOW, NOW)), 'BLOCKED')
con.execute("INSERT INTO task_dispatches(user_id,id,room_id,team_id,round,leader_session_id,member_session_id,member_expert_id,task_digest,state,dispatched_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (U, bytes.fromhex('35'*16), 'r', TA, 0, SA, SA, 'e1', H32, 'RUNNING', NOW, NOW, NOW))
check("同 (room,round,member) 二次派工",
      lambda: con.execute("INSERT INTO task_dispatches(user_id,id,room_id,team_id,round,leader_session_id,member_session_id,member_expert_id,task_digest,state,dispatched_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                          (U, bytes.fromhex('36'*16), 'r', TA, 0, SA, SA, 'e1', H32, 'RUNNING', NOW, NOW, NOW)), 'BLOCKED')
con.execute("INSERT INTO team_members(user_id,team_id,expert_id,role,state,state_changed_at,joined_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
            (U, TA, 'lead', 'leader', 'IDLE', NOW, NOW, NOW, NOW))
check("同一 team 出现第二个 leader",
      lambda: con.execute("INSERT INTO team_members(user_id,team_id,expert_id,role,state,state_changed_at,joined_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
                          (U, TA, 'e2', 'leader', 'IDLE', NOW, NOW, NOW, NOW)), 'BLOCKED')
check("team_members 有 session 却处于 IDLE",
      lambda: con.execute("INSERT INTO team_members(user_id,team_id,expert_id,role,member_session_id,state,state_changed_at,joined_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
                          (U, TA, 'e3', 'member', SA, 'IDLE', NOW, NOW, NOW, NOW)), 'BLOCKED')
check("team max_dispatch 越界（>8，INV-7）",
      lambda: con.execute("UPDATE teams SET max_dispatch=99 WHERE user_id=? AND id=?", (U, TA)), 'BLOCKED')
check("team max_replan 越界（>5）",
      lambda: con.execute("UPDATE teams SET max_replan=99 WHERE user_id=? AND id=?", (U, TA)), 'BLOCKED')

hdr("D. 消息序号与幂等")
con.execute("INSERT INTO messages(user_id,id,session_id,seq,role,content,created_at) VALUES(?,?,?,?,?,?,?)",
            (U, bytes.fromhex('61'*16), SA, 1, 'user', 'hi', NOW))
check("同 session 重复 seq=1",
      lambda: con.execute("INSERT INTO messages(user_id,id,session_id,seq,role,content,created_at) VALUES(?,?,?,?,?,?,?)",
                          (U, bytes.fromhex('62'*16), SA, 1, 'user', 'dup', NOW)), 'BLOCKED')
check("B 用户可有自己的 seq=1（不同 session 互不干扰）",
      lambda: con.execute("INSERT INTO messages(user_id,id,session_id,seq,role,content,created_at) VALUES(?,?,?,?,?,?,?)",
                          (Ub, bytes.fromhex('63'*16), SB, 1, 'user', 'hi', NOW)), 'ALLOWED')

hdr("E. 软删除语义")
con.execute("UPDATE users SET deleted_at=? WHERE id=?", (NOW, U))
check("A 软删后他人复用用户名 'alice'",
      lambda: con.execute("""INSERT INTO users(id,username,username_norm,display_name,
          password_hash,password_salt,password_algo,role,pwd_changed_at,created_at,updated_at)
          VALUES(?,?,?,?,?,?,?,?,?,?,?)""",
          (bytes.fromhex('dd'*16), 'alice2', 'alice', 'X', b'h', b's'*16, ALGO, 'member', NOW, NOW, NOW)), 'ALLOWED')
check("A 恢复时用户名已被占用 -> 恢复失败（这正是我们要的行为）",
      lambda: con.execute("UPDATE users SET deleted_at=NULL WHERE id=?", (U,)), 'BLOCKED')
con.execute("DELETE FROM users WHERE id=?", (bytes.fromhex('dd'*16),))
con.execute("UPDATE users SET deleted_at=NULL WHERE id=?", (U,))
check("占用者删除后 A 恢复成功",
      lambda: con.execute("""INSERT INTO users(id,username,username_norm,display_name,
          password_hash,password_salt,password_algo,role,pwd_changed_at,created_at,updated_at)
          VALUES(?,?,?,?,?,?,?,?,?,?,?)""",
          (bytes.fromhex('de'*16), 'alice3', 'alice', 'Y', b'h', b's'*16, ALGO, 'member', NOW, NOW, NOW)), 'BLOCKED')

hdr("F. 删除用户时级联清理")
con.execute("INSERT INTO wiki_index(user_id,term,doc_id,term_kind,tf,field_len,rel_path,page_title,content_hash,bytes,indexed_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            (U, 'rust', bytes.fromhex('71'*16), 0, 3, 100, 'a.md', 'T', H32, 900, NOW))
b4 = con.execute("SELECT COUNT(*) FROM wiki_index WHERE user_id=?", (U,)).fetchone()[0]
con.execute("DELETE FROM users WHERE id=?", (Ub,))
af = con.execute("SELECT COUNT(*) FROM wiki_index WHERE user_id=?", (U,)).fetchone()[0]
ok = b4 == 1 and af == 1
results.append((ok, "删除 B 不影响 A 的 wiki_index", f"{b4}->{af}"))
print(f"[{'PASS' if ok else 'FAIL'}] 删除 B 不影响 A 的 wiki_index\n         -> {b4} -> {af}")
con.execute("DELETE FROM users WHERE id=?", (U,))
left = con.execute("SELECT COUNT(*) FROM wiki_index WHERE user_id=?", (U,)).fetchone()[0]
ok = left == 0
results.append((ok, "删除 A 级联清空其 wiki_index", f"left={left}"))
print(f"[{'PASS' if ok else 'FAIL'}] 删除 A 级联清空其 wiki_index\n         -> left={left}")

hdr("G. 关键查询的执行计划（索引是否真的被用上）")
con.execute("""INSERT INTO users(id,username,username_norm,display_name,password_hash,
    password_salt,password_algo,role,pwd_changed_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)""",
    (U,'alice','alice','A',b'h',b's'*16,ALGO,'owner',NOW,NOW,NOW))
con.execute("""INSERT INTO sessions(user_id,id,kind,room_id,workspace_path,created_at,updated_at,last_active_at)
    VALUES(?,?,?,?,?,?,?,?)""", (U, SA, 'solo', f'leader_session:alice:{SA.hex()}', f'sessions/{SA.hex()}', NOW, NOW, NOW))
for i in range(2, 300):
    con.execute("INSERT INTO messages(user_id,id,session_id,seq,role,content,created_at) VALUES(?,?,?,?,?,?,?)",
                (U, bytes([i%256, (i//256)%256] + [0]*14), SA, i, 'user', 'x', NOW))

QUERIES = [
 ("用户会话列表（用户切换 <50ms 主路径）",
  "SELECT id,title,last_active_at FROM sessions WHERE user_id=? AND deleted_at IS NULL ORDER BY last_active_at DESC LIMIT 20", (U,)),
 ("会话内消息倒序分页（深翻页）",
  "SELECT seq,content FROM messages WHERE user_id=? AND session_id=? ORDER BY seq DESC LIMIT 50", (U, SA)),
 ("BM25 检索：按词取倒排",
  "SELECT doc_id,tf,field_len FROM wiki_index WHERE user_id=? AND term=?", (U,'rust')),
 ("登录查用户",
  "SELECT id,password_hash FROM users WHERE username_norm=? AND deleted_at IS NULL", ('alice',)),
 ("refresh token 校验（每个请求都跑）",
  "SELECT user_id,expires_at FROM sessions_auth WHERE token_hash=?", (b'y'*32,)),
 ("派工收敛判定（AllSettled）",
  "SELECT COUNT(*) FROM task_dispatches WHERE user_id=? AND room_id=? AND state IN ('PENDING','RUNNING','ASKING')", (U,'r')),
]
for name, q, p in QUERIES:
    plan = con.execute("EXPLAIN QUERY PLAN " + q, p).fetchall()
    detail = " | ".join(r[-1] for r in plan)
    scan = any('SCAN' in r[-1] and 'USING' not in r[-1] for r in plan)
    results.append((not scan, name, detail))
    print(f"[{'PASS' if not scan else 'FAIL'}] {name}\n         -> {detail}")

hdr("H. 写放大评估：批量写 2000 行")
con.execute("CREATE TABLE probe(id INTEGER PRIMARY KEY, v TEXT)")
t0=time.perf_counter()
con.execute("COMMIT")
con.execute("BEGIN")
con.executemany("INSERT INTO probe(v) VALUES(?)", [(f'x{i}',) for i in range(2000)])
con.execute("COMMIT")
dt=time.perf_counter()-t0
print(f"2000 行单事务插入耗时: {dt*1000:.1f} ms  (推算：单次 ingest 触及 15 页 × 每页 200 词 = 3000 行，约 {dt*1500:.0f} ms)")
results.append((dt < 3.0, "2000 行批量插入 < 3s", f"{dt*1000:.1f}ms"))

print("\n" + "="*76)
n_fail = sum(1 for ok,_,_ in results if not ok)
print(f"合计 {len(results)} 项断言，失败 {n_fail} 项")
print("="*76)
sys.exit(1 if n_fail else 0)
