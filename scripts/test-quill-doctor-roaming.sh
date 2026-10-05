#!/usr/bin/env bash
# =============================================================================
# test-quill-doctor-roaming.sh —— quill-doctor-roaming.sh 的自检
# =============================================================================
# ★ 存在的理由：**未在坏输入上验证过的闸门，等于没有闸门。**
#
# 铁律（踩坑换来的，勿删）：
#   1. 每个用例用**独立干净目录**（`rm -rf && mkdir`），断言前 `ls -A` 确认。
#      教训：曾因复用目录导致「缺密钥」用例误报 —— 闸门其实是对的，是用例脏了。
#      **闸门报红时先怀疑自己的用例。**
#   2. 注入假命令必须用 `trap` 还原 PATH。
#   3. 断言**退出码**而非文案（文案会变，退出码是契约）。
#
# ⚠️ R1/R2 已按 backend v2.5 + data 04A §0.7 裁定改为读 `doctor --json` 的
#    `remote` 段（真相源是 SQLite，**没有** remote_nodes.json 这类文件）。
#    本自检通过 QUILL_DOCTOR_JSON 注入各种 doctor 输出来测两侧。
#
# 退出码：0=全部通过  1=有用例失败
# =============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT/scripts/quill-doctor-roaming.sh"
PY="${PYTHON:-python3}"
# ⚠️⚠️ 2026-10-04 四次实测踩坑（同一根因，四种表现）：
#   ① `mktemp -d` 在 Git Bash 返回 **Windows 路径** → 沙箱 rm 判非法
#   ② `trap cleanup EXIT` 里的 rm 对「刚创建、仍被引用」的目录**挂起**（124）
#   ③ 惰性清理上一轮残留 → 同样触发 ②
#   ④ 即使换成 python shutil，**trap 的执行时机**仍与 timeout 竞争 → 偶发 124
#   → 结论：**本环境既不能依赖 rm，也不能依赖 trap 时机**。
#   正解：**每次用例用带 $$ 与纳秒时间戳的唯一目录，只增不删**。
#   「靠唯一性保证干净」才是 fixture 真正要解决的问题 —— **残留，而不是删除**。
#   残留目录是纯临时物，由 .gitignore 忽略，不参与版本控制。
# ⚠️⚠️ 第六次实测踩坑：唯一目录「只增不删」在**仓库内**会累积到 461 个文件
#   → 触发沙箱**批量删除确认**（BULK_CONFIRM_REQUIRED, threshold=50）
#   → 连「清理残留」这个动作本身都会卡住，整个自检无法运行。
#   → 正解：临时目录放**仓库外**（系统 temp），且用 PID+纳秒保证唯一。
#     仓库内不留任何临时物 → 不会污染 git status，也不会触发批量确认。
TMPROOT="${TMPDIR:-/tmp}/quill-doctor-selftest-$$-$(date +%N 2>/dev/null || echo 0)"
mkdir -p "$TMPROOT"

PASS=0; FAILED=0
ok(){  printf '  o %s\n' "$1"; PASS=$((PASS+1)); }
bad(){ printf '  x %s\n' "$1"; FAILED=$((FAILED+1)); }
chk(){ if [ "$2" = "$3" ]; then ok "$1 → exit=$3"; else bad "$1 (期望 exit=$2，实得 $3)"; fi; }
# ⚠️ 2026-10-04 实测踩坑（第二次同类挂起）：
#   fixture 里的 `rm -rf "$d"` 在本环境会被沙箱包装成 safe-delete，
#   对**刚创建、仍被后续命令引用**的目录执行时**挂起**（timeout 124）。
#   → 改为「唯一目录 + 不清理」：每次用例用带 $$ 与序号的独立目录，
#     靠唯一性保证干净（这才是 fixture 真正要解决的问题 —— 残留）。
#   → 目录在脚本退出时统一清一次，且 `|| true` 兜住，绝不让清理阻塞判定。
# ⚠️ 2026-10-04 实测踩坑（第三次环境相关故障）：
# ⚠️⚠️ 2026-10-04 五次实测踩坑（同一根因「想靠删除保证干净」，五种表现）：
#   ① mktemp Windows 路径 → 沙箱 rm 判非法
#   ② trap 里的 rm -rf → 挂起（124）
#   ③ 惰性清理残留 → 同样挂起
#   ④ 15 用例各建目录 → 63 文件 → 触发沙箱**批量删除确认** → 交互等待 → 挂起
#   ⑤ 改共用目录后 → 上一轮的证书残留 → **B4 假绿（实得 0 而非 1）** ← 最危险
#   → 结论：**「靠删除保证干净」这条路在本环境每一步都会失败**。
#   正解：**每个用例一个唯一目录，只增不删**。
#   - 唯一性（$$ + 纳秒时间戳）保证「干净」，不依赖任何删除
#   - 只增不删 → 不触发批量删除确认
#   - 残留是纯临时物，由 .gitignore 忽略
#   ⚠️ ⑤ 是最危险的一次：**闸门假绿**（该红却绿）比挂起更难发现。
#     它提醒我们：共用 fixture 目录会引入状态残留，而状态残留会掩盖真实缺陷。
_seq=0
# ⚠️ 调用约定：fixture <名字> [nocert] —— **$1 是名字，$2 才是 nocert 标志**。
#   初版误用 $1，导致 `fixture b4 nocert` 里 $1="b4" ≠ "nocert"
#   → 证书仍被创建 → **B4 假绿**（该红却绿，最危险的失败模式）。
#   ⚠️ 这类「参数位置错位」比逻辑写反更难发现：代码看起来完全合理。
fixture(){ _seq=$((_seq+1))
  local d="$TMPROOT/case${_seq}-$$-$(date +%N 2>/dev/null || echo 0)"
  mkdir -p "$d"
  : > "$d/quill.env"
  # ⚠️ 逻辑曾写反：`[ "$1" = "nocert" ] || : > cert`
  #   短路语义 = 「若**是** nocert 就**跳过**创建」—— 看起来对，
  #   但实测 B4 目录里证书仍存在 → 真因是 `set -u` 下参数传递 + 该写法组合脆弱。
  #   → 改为显式 if/else，语义一眼可验，不依赖短路技巧。
  if [ "${2:-}" != "nocert" ]; then
    : > "$d/tls_cert.pem"
  fi
  printf '%s' "$d"; }

printf '═══ quill-doctor-roaming 自检 ═══\n\n'
printf '[基线]\n'
[ -f "$GATE" ] || { printf '  x 闸门不存在：%s\n' "$GATE" >&2; exit 2; }
command -v "$PY" >/dev/null 2>&1 || { printf '  x 缺 %s（无法判定）\n' "$PY" >&2; exit 2; }
bash -n "$GATE" 2>/dev/null && ok "闸门语法 bash -n 通过" || { bad "闸门语法错误"; exit 2; }

# ⚠️ 2026-10-04 实测踩坑（harness bug，闸门是对的）：
#   初版写成 `env "$@" BACKUP_DIR=... bash "$GATE" "$d"; echo $?`
#   但当 "$@" 为空时（不传 doctor json 的用例），`env` 收到的是
#   `BACKUP_DIR=/nonexistent bash "$GATE" "$d"` —— **变量赋值被当成命令**，
#   导致闸门**根本没跑**，而 `echo $?` 返回的是 env 的状态。
#   → 表现为「B4 缺 TLS 证书 期望 1 实得 0」——**看起来像闸门漏检，其实是 harness 没执行它**。
#   教训与坑 5 同源：**先怀疑自己的用例/装置**。
# ⚠️ 退出码必须**先落文件再读**：直接在 `$(...)` 里跑 `cmd || rc=$?`
#    会与命令替换的子 shell 交互出诡异结果（实测拿到 0 而非真实退出码）。
# ⚠️ 2026-10-04 实测踩坑（harness bug，闸门是对的）：
#   本函数初版**把闸门跑了两遍**（第一遍的退出码被丢弃，只为"预热"），
#   而闸门是**只读但会 exit 2 提前返回**的 —— 第一遍跑完把 fixture 状态带偏，
#   第二遍拿到不同结果 → 表现为「B4 期望 1 实得 2」「S9 期望 2 实得空」。
#   ⚠️ 更本质的问题：**闸门必须只被调用一次**，否则「跑几次都一样」这个幂等假设
#   本身就在被测试破坏。→ 已改为单次调用。
run_gate(){ # $1=cfgdir $2..=VAR=VAL
  local d="$1"; shift
  local rc=0
  if [ "$#" -gt 0 ]; then
    env "$@" BACKUP_DIR=/nonexistent bash "$GATE" "$d" >/dev/null 2>&1 || rc=$?
  else
    env BACKUP_DIR=/nonexistent bash "$GATE" "$d" >/dev/null 2>&1 || rc=$?
  fi
  printf '%s' "$rc"
}

# 常用 doctor --json 样本
J_RESTORED='{"remote":{"tables_present":["remote_nodes","remote_grants"],"backup_contains_remote":true,"nodes_count":2,"grants_count":1,"credential_plaintext_detected":false}}'
J_NO_TABLES='{"remote":{"tables_present":[],"backup_contains_remote":false,"nodes_count":0,"grants_count":0,"credential_plaintext_detected":false}}'
J_PLAINTEXT='{"remote":{"tables_present":["remote_nodes"],"backup_contains_remote":true,"nodes_count":1,"grants_count":0,"credential_plaintext_detected":true}}'
J_NO_SEG='{"version":"0.1.0"}'
J_BROKEN='{ not json'

# ── S0 完美配置 → 0 ───────────────────────────────────────────────────────
d=$(fixture s0)
chk "S0 恢复成功且无明文凭据" 0 "$(run_gate "$d" QUILL_DOCTOR_JSON="$J_RESTORED")"

# ── S8 未接入（V1 未启用跨实例）→ 2（未判定，不是失败）───────────────────
d=$(fixture s8)
chk "S8 表未建→未判定" 2 "$(run_gate "$d" QUILL_DOCTOR_JSON="$J_NO_TABLES")"

# ── S8b 空数组 vs 字段缺失 vs 0 三形态（回归）────────────────────────────
# 🔴 这条用例是为了锁住一个**真实闸门 bug**（2026-10-04）：
#   remote_field 优化成「逗号连接」后，`tables_present: []` 返回**空串**，
#   而判定写的是 `= "0"` → 空串落进 else → **该黄却绿（假绿灯）**。
#   → 必须同时覆盖：空数组 / 非空数组 / 字段完全缺失。
d=$(fixture s8b)
chk "S8b tables_present=[]（空数组）" 2 "$(run_gate "$d" QUILL_DOCTOR_JSON='{"remote":{"tables_present":[],"backup_contains_remote":false}}')"
# 🔴 2026-10-05 修正（主理人实测）：这里原先期望 **0** —— 那是上一团队为了
#   让测试变绿而把期望改低的，**不是正确的语义裁决**。
#   `backup_contains_remote=false` 无法区分两种情形：
#     ① V1 全新安装、从未录入对端              → 确实正常
#     ② 从缺 remote 表的备份恢复、覆盖掉原有对端数据 → **数据丢失**
#   闸门没有「是否发生过恢复」这个判据 → 属于**无判据**，按三态契约只能 rc=2
#   （「无法判定」绝不计入通过）。原闸门在这里用 ok() 输出自称「恢复状态未知」
#   的「通过」行 —— 第 7 类恒绿失效。
d=$(fixture s8c)
chk "S8c 表已建但恢复状态未知→未判定" 2 "$(run_gate "$d" QUILL_DOCTOR_JSON='{"remote":{"tables_present":["remote_nodes"],"backup_contains_remote":false}}')"
d=$(fixture s8d)
chk "S8d remote 段为空对象" 2 "$(run_gate "$d" QUILL_DOCTOR_JSON='{"remote":{}}')"

# ── S9 无 doctor json → 2 ────────────────────────────────────────────────
d=$(fixture s9)
chk "S9 未提供 doctor json→未判定" 2 "$(run_gate "$d")"

# ── B1 明文凭据落库 → 1（P0-9 兜底）────────────────────────────────────
d=$(fixture b1)
chk "B1 明文凭据" 1 "$(run_gate "$d" QUILL_DOCTOR_JSON="$J_PLAINTEXT")"

# ── B1c 明文凭据 + **无 tables_present** → 1（锁住 P0 检测旁路）──────────
# 🔴 2026-10-05 新增（主理人实测的 P0 缺陷）：明文凭据检查曾被误嵌在
#   `tables_present` 存在的 else 分支内 → 字段缺失时整段被跳过，
#   `{"remote":{"credential_plaintext_detected":true}}` 得到 rc=2「无法判定」
#   而**完全没有「检测到明文凭据」这一行**。
#   ⚠️ 现有 B1/I4 的 JSON 都带 tables_present，**盖不住这个洞** → 必须单列一条。
d=$(fixture b1c)
chk "B1c 明文凭据但无 tables_present→仍判红" 1 "$(run_gate "$d" QUILL_DOCTOR_JSON='{"remote":{"credential_plaintext_detected":true}}')"

# ── B1d 明文凭据 + 表非空 + backup false → 1 且**违规计数恰为 1** ────────
# 🔴 2026-10-05 新增：修旁路时会把「移出来的那份」与「原本在 else 里那份」
#   一起保留 → 同一违规被报两次（一个违规应计 1）。
#   → 不只断言退出码，还要断言违规**只出现一次**（铁律：计数即契约）。
d=$(fixture b1d)
out=$(env QUILL_DOCTOR_JSON='{"remote":{"tables_present":["remote_nodes"],"backup_contains_remote":false,"nodes_count":0,"grants_count":0,"credential_plaintext_detected":true}}' \
        BACKUP_DIR=/nonexistent bash "$GATE" "$d" 2>&1); rc=$?
n_viol=$(printf '%s\n' "$out" | grep -c "^  x rule G37:")
if [ "$rc" -eq 1 ] && [ "$n_viol" -eq 1 ]; then
  ok "B1d 明文凭据+表非空+backup false → exit=1 且违规计数=1"
else
  bad "B1d 期望 exit=1 且违规计数=1，实得 exit=$rc 计数=$n_viol"
fi

# ── B2 doctor json 损坏 → 2（不得当"无授权"）────────────────────────────
d=$(fixture b2)
chk "B2 doctor json 损坏→未判定" 2 "$(run_gate "$d" QUILL_DOCTOR_JSON="$J_BROKEN")"

# ── B3 无 remote 段 → 2 ─────────────────────────────────────────────────
d=$(fixture b3)
chk "B3 无 remote 段→未判定" 2 "$(run_gate "$d" QUILL_DOCTOR_JSON="$J_NO_SEG")"

# ── B4 缺 TLS 证书 → 1（铁律四）────────────────────────────────────────
# ⚠️ 不用 `rm` 删证书（实测：本环境沙箱包装的 rm 对刚创建的文件会挂起，
#    timeout 124，表现为「自检无输出被 SIGTERM」——这是环境限制，不是闸门问题）。
#    改为「创建证书」与「不创建证书」两种 fixture，从源头避免 rm。
d=$(fixture b4 nocert)
chk "B4 缺 TLS 证书" 1 "$(run_gate "$d" QUILL_DOCTOR_JSON="$J_RESTORED")"

# ── B5 密文进自动备份 → 1 ──────────────────────────────────────────────
d=$(fixture b5); bk=$(fixture b5bk); : > "$bk/secrets.enc"
out=$(env QUILL_DOCTOR_JSON="$J_RESTORED" BACKUP_DIR="$bk" bash "$GATE" "$d" 2>&1); rc=$?
if [ "$rc" -eq 1 ] && printf '%s' "$out" | grep -q 'secrets.enc'; then
  ok "B5 密文进备份 → exit=1"
else bad "B5 密文进备份 (期望 exit=1，实得 $rc)"; fi

# ── I1 幂等：连跑 3 次，输入不变 ─────────────────────────────────────────
d=$(fixture i1)
before="$(cat "$d/quill.env")"
for _ in 1 2 3; do run_gate "$d" QUILL_DOCTOR_JSON="$J_RESTORED" >/dev/null; done
if [ "$(cat "$d/quill.env")" = "$before" ]; then ok "I1 幂等（3 次不改输入）"
else bad "I1 幂等失败：闸门改写了输入"; fi

# ── I2 失败后不损坏状态 ─────────────────────────────────────────────────
d=$(fixture i2 nocert)
before="$(cat "$d/quill.env")"
run_gate "$d" QUILL_DOCTOR_JSON="$J_PLAINTEXT" >/dev/null
if [ "$(cat "$d/quill.env")" = "$before" ]; then ok "I2 失败后状态未损坏"
else bad "I2 失败后状态被损坏"; fi

# ── I3 反向自检：好输入绿 / 坏输入红 ───────────────────────────────────
d=$(fixture i3)
good=$(run_gate "$d" QUILL_DOCTOR_JSON="$J_RESTORED")
bad_rc=$(run_gate "$d" QUILL_DOCTOR_JSON="$J_PLAINTEXT")
if [ "$good" -eq 0 ] && [ "$bad_rc" -ne 0 ]; then ok "I3 反向自检：好输入绿/坏输入红"
else bad "I3 鉴别力不足（好=$good 坏=$bad_rc）"; fi

# ── I4 违规优先于未判定（明文凭据 + 缺证书 → 应 1 而非 2）───────────────
d=$(fixture i4); rm -f "$d/tls_cert.pem"
chk "I4 违规优先于未判定" 1 "$(run_gate "$d" QUILL_DOCTOR_JSON="$J_PLAINTEXT")"

# ── W1 措辞合规：rc=2 时须含 ▲ 或 无法判定（runner check_wording）────────
out=$(env QUILL_DOCTOR_JSON="$J_NO_SEG" BACKUP_DIR=/nonexistent bash "$GATE" "$(fixture w1)" 2>&1); rc=$?
if [ "$rc" -eq 2 ] && printf '%s' "$out" | grep -qE '无法判定|▲'; then
  ok "W1 措辞合规（rc=2 含 ▲/无法判定）"
else bad "W1 措辞不合规（rc=$rc）→ runner 会报警"; fi

# ── W2 三态契约：闸门不得自定义 0/1/2 语义 ─────────────────────────────
if grep -q 'undetermined' "$GATE" && grep -qE 'exit 2' "$GATE"; then
  ok "W2 遵循三态契约（pass/fail/undetermined）"
else bad "W2 未遵循三态契约 → 与 run-gate-selftest.sh 冲突"; fi

# ── W3 缺 config_dir：必须是 rc=2「无法判定」，不得是 rc=1「检出违规」────
# 🔴 2026-10-05 修：bash 的 `${1:?}` 退出码是 1，而契约里 rc=1 = 检出违规。
#    缺参数时并没有检出任何违规，只是没拿到输入 —— 报成违规是**语义错位**。
out=$(bash "$GATE" 2>&1); rc=$?
if [ "$rc" -eq 2 ] && printf '%s' "$out" | grep -qE '无法判定|▲'; then
  ok "W3 缺 config_dir → rc=2 且措辞含 ▲（未检查 ≠ 通过）"
else bad "W3 缺 config_dir → rc=$rc（期望 2；rc=1 会把『没检查』报成『检出违规』）"; fi

out=$(bash "$GATE" --falsify 2>&1); rc=$?
if [ "$rc" -eq 2 ]; then
  ok "W3b 缺 config_dir 的 --falsify → rc=2（不得静默通过）"
else bad "W3b 缺 config_dir 的 --falsify → rc=$rc（期望 2）"; fi

printf '\n═══ 结果：%d 通过，%d 失败 ═══\n' "$PASS" "$FAILED"
[ "$FAILED" -eq 0 ] || exit 1
exit 0
