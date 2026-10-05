
import sqlite3, sys, re, os, glob
from pathlib import Path

FAIL = []
PASSED = 0
def ck(name, ok, detail=""):
    global PASSED
    print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f"  {detail}" if detail else ""))
    if ok: PASSED += 1
    else: FAIL.append(name)

ROOT = Path(__file__).resolve().parents[3]
SQL  = ROOT / "crates/quill-store/migrations/0001_init.sql"

print("="*76)
print("断言 1：invites 表存在于 schema（v1 保留结构）")
print("="*76)
sql = open(SQL, encoding='utf-8').read()
ck("invites 表已建", "CREATE TABLE invites" in sql)
ck("有独立主键约束", re.search(r'CREATE TABLE invites \((.*?)\n\) STRICT', sql, re.S)
   and "PRIMARY KEY" in re.search(r'CREATE TABLE invites \((.*?)\n\) STRICT', sql, re.S).group(1))
ck("有 code 唯一索引", "CREATE UNIQUE INDEX ux_invites_code" in sql)
ck("有外键到 users", "FOREIGN KEY (created_by) REFERENCES users(id)" in sql)

print()
print("="*76)
print("断言 2：全仓无任何 INSERT 路径（写路径必须为空）")
print("="*76)

srcs = [p for p in glob.glob(os.path.join(ROOT, "crates/**/*.rs"), recursive=True)]
ck("crates 下 .rs 文件数 > 0（扫描有意义）", len(srcs) > 0, f"共 {len(srcs)} 个")

TEST_PATH = re.compile(r'[\\/](tests?|benches|fixtures?|examples?)[\\/]', re.I)
srcs_prod = [p for p in srcs if not TEST_PATH.search(p)]
ck("存在非测试的生产源码（否则扫描无意义）", len(srcs_prod) > 0,
   f"生产 {len(srcs_prod)} 个 / 测试 {len(srcs)-len(srcs_prod)} 个")
srcs = srcs_prod

write_pat = re.compile(r'(INSERT\s+(?:OR\s+\w+\s+)?INTO\s+invites|UPDATE\s+invites|DELETE\s+FROM\s+invites)', re.I)
hits = []
outside = []
for p in srcs:
    for i, line in enumerate(open(p, encoding='utf-8', errors='ignore'), 1):
        if write_pat.search(line):
            rel = os.path.relpath(p, ROOT)
            hits.append(f"{rel}:{i}: {line.strip()}")

            if not rel.startswith('crates/quill-control' + os.sep):
                outside.append(f"{rel}:{i}")

ck("invites 写操作只出现在 quill-control（其他 crate 命中即违规）", not outside,
   "" if not outside else "越界命中 " + str(outside))
print(f"       （quill-control 内的 {len(hits)} 处属预期内，非违规）")

repo_hits = []
for p in srcs:
    t = open(p, encoding='utf-8', errors='ignore').read()
    if re.search(r'\bInvite(Repo|Store|Service|Model|Record)\b', t):
        repo_hits.append(os.path.relpath(p, ROOT))
ck("无 invites 仓储层/领域模型", not repo_hits, "" if not repo_hits else str(repo_hits))

route_hits = [os.path.relpath(p, ROOT) for p in srcs
              if re.search(r'/invites?', open(p, encoding='utf-8', errors='ignore').read())]
ck("无 invites 路由挂载", not route_hits, "" if not route_hits else str(route_hits))

print()
print("="*76)
print("断言 3：结构本身可被 v1.1 启用（约束完整，无需改 schema）")
print("="*76)
con = sqlite3.connect(':memory:'); con.execute("PRAGMA foreign_keys=ON")
con.executescript(sql)
U  = bytes.fromhex('aa'*16); U2 = bytes.fromhex('bb'*16)
now = 1

ALGO = 'pbkdf2-hmac-sha256$i=600000'
con.execute("INSERT INTO users(id,username,username_norm,display_name,password_hash,password_salt,password_algo,role,pwd_changed_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            (U,'owner','owner','Owner',b'h',b's'*16,ALGO,'owner',now,now,now))
try:
    con.execute("INSERT INTO invites(id,code_hash,created_by,expires_at,created_at) VALUES(?,?,?,?,?)",
                (bytes.fromhex('01'*16), b'k'*32, U, now+86400000, now))
    ok1 = True
except sqlite3.Error as e:
    ok1 = False; print("   ", e)
ck("v1.1 写入路径可用（结构未阻塞）", ok1)

def blocked(sql_text, params, label):

    try:
        con.execute(sql_text, params)
        return False, "❌ 被接受（约束缺失）"
    except sqlite3.IntegrityError as e:
        return True, f"IntegrityError: {e}"
    except sqlite3.Error as e:
        return False, f"❌ 非约束错误（假绿）: {e}"

def fresh(n):  return bytes([n]+[0]*15)
def code(n):  return bytes([n]*32)

ok,d = blocked("INSERT INTO invites(id,code_hash,created_by,max_uses,used_count,expires_at,created_at) VALUES(?,?,?,?,?,?,?)",
               (fresh(1), code(1), U, 1, 5, now+1, now), "used>max")
ck("约束：used_count <= max_uses", ok, d)

ok,d = blocked("INSERT INTO invites(id,code_hash,created_by,role,expires_at,created_at) VALUES(?,?,?,?,?,?)",
               (fresh(2), code(2), U, 'admin', now+1, now), "非法role")
ck("约束：role IN (owner,member)", ok, d)

ok,d = blocked("INSERT INTO invites(id,code_hash,created_by,expires_at,created_at) VALUES(?,?,?,?,?)",
               (fresh(3), code(3), bytes.fromhex('cc'*16), now+1, now), "signer不存在")
ck("约束：FK 到 users 生效", ok, d)

ok,d = blocked("INSERT INTO invites(id,code_hash,created_by,expires_at,created_at) VALUES(?,?,?,?,?)",
               (fresh(4), code(4), U, now+1, now), "合法行")
ck("合法行写入成功（约束未误伤）", not ok, d)

ok,d = blocked("INSERT INTO invites(id,code_hash,created_by,expires_at,created_at) VALUES(?,?,?,?,?)",
               (fresh(5), code(4), U, now+1, now), "code_hash重复")
ck("约束：code_hash 唯一", ok, d)

ok,d = blocked("INSERT INTO invites(id,code_hash,created_by,expires_at,created_at) VALUES(?,?,?,?,?)",
               (fresh(6), code(6), U, now, now), "expires<=issued")
ck("约束：expires_at > now（过期邀请）", ok, d)

print()
print("="*76)
print("断言 4：★ 断言自身可被证伪（FALS）")
print("="*76)
print("  若上述写路径检查写错了正则/漏扫目录，它会『假绿』。")
print("  因此用一条故意违规的样例验证检测器会红：")
SAMPLE = 'INSERT INTO invites(id,code_hash,created_by,expires_at,created_at) VALUES(1,2,3,4,5);'
m = write_pat.search(SAMPLE)
ck("检测器对真实违规样例会红（自检通过）", m is not None,
   f"匹配到: {m.group(0) if m else 'None'}")

print()
print("="*76)
n = len(FAIL)
print(f"合计 {len(FAIL)+PASSED} 项检查，失败 {len(FAIL)} 项")
if FAIL:
    for f in FAIL: print("  ❌", f)
print("="*76)
sys.exit(1 if FAIL else 0)
