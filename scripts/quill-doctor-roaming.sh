#!/usr/bin/env bash
# =============================================================================
# quill-doctor-roaming.sh —— 实例委派部署闸门（V1：标准 WS + HTTP 登录）
# =============================================================================
# ⚠️ 传输层已校准（2026-10-04）：**iroh/QUIC 作废**，改为标准 WebSocket + HTTP 登录。
#    本脚本**不含任何 iroh/relay/中继逻辑** —— 那套已随传输层作废（见 09 文档 §1.3）。
#
# ★ 退出码严格遵循 scripts/run-gate-selftest.sh 的三态契约（不可自定义）：
#     0 = pass         通过
#     1 = fail         检出违规
#     2 = undetermined 无法判定（**绝不计入通过**）
#   ⚠️ 历史教训：初版曾用 0/1/2 = 全绿/警告/错误，与本契约冲突：
#      真实违规(退出2)被 runner 判成「未覆盖」→ 闸门静默失效；
#      且警告(退出1)被 runner 判成 fail → 健康实例被误报违规。已修正。
#
# 幂等：只读，不修改任何状态；可反复执行。
# 依据：docs/09_实例间委派_运维方案.md §10
# =============================================================================
set -uo pipefail

# 🔴 2026-10-05 订正：原写 `CFG_DIR="${1:?usage: ...}"`。
#    bash 的 `${var:?}` 在变量为空时**退出码是 1** —— 而本脚本自己声明的契约里
#    rc=1 是「**检出违规**」。缺参数时它**并没有检出任何违规**，只是没拿到输入。
#    把「没给输入」报成「发现缺陷」会让 CI 在配置错误时给出误导性的红灯。
#    正确语义：无法判定 → rc=2 + ▲ 措辞（绝不计入通过）。
if [[ $# -lt 1 || -z "${1:-}" || "${1:-}" == -* ]]; then
  printf '%s\n' "▲ G36 无法判定：缺少必填参数 <config_dir>（未检查，不计入通过）" >&2
  printf '%s\n' "  用法：bash scripts/quill-doctor-roaming.sh <config_dir> [--falsify]" >&2
  printf '%s\n' "  ⚠️ 这是**没检查**，不是**检查通过**。未判定 ≠ 通过。" >&2
  exit 2
fi
CFG_DIR="$1"
FALSIFY=0
for a in "$@"; do [[ "$a" == "--falsify" ]] && FALSIFY=1; done

PY="${PYTHON:-python3}"
# ⚠️ 缺 python3 必须判「无法判定」而非静默通过（铁律：缺工具≠通过）
command -v "$PY" >/dev/null 2>&1 || {
  printf '  ▲ 缺工具 %s —— 无法判定（不等于通过）\n' "$PY"
  printf '\n结论：无违规，但有 1 项无法判定（不等于通过）。\n'
  exit 2
}
BAD=0; UNDET=0
# ⚠️ 契约 §2.1 + runner:143 的硬要求：违规输出必须含 "rule <ID>" 字样，
#    否则 runner 无法区分「真检查」与「无条件报红」的恒绿闸门。
RID="${RID:-G37}"
fail(){ printf '  x rule %s: %s\n' "$RID" "$*"; BAD=$((BAD+1)); }
warn(){ printf '  ▲ %s\n' "$*"; UNDET=$((UNDET+1)); }   # ▲ 措辞合规必需（runner check_wording）
ok(){   printf '  o %s\n' "$*"; }

# =============================================================================
# ⚠️ R1/R2 的真相源已裁定（backend v2.5 + data 04A §0.7）：**只有 SQLite，没有文件**
#   remote_nodes / remote_grants / remote_dispatches → quill.db
#   对端 B 的凭据 → data/{uid}/secrets.enc（契约 §六：凭据不建表）
#   ⚠️ `remote_nodes.json` / `remote_grants.json` 是 **iroh 路线的残留假设**，不存在。
#      （我最初按 iroh 路线写了这个闸门 → 检查的是不存在的东西 → 长期报「未判定」，
#        正是 data 说的「静默失败」。已按裁定改为查 doctor --json。）
#
# ★ 闸门**不直接开 DB** —— 那违反「doctor 不碰业务数据」的边界。
#   统一读 `quill doctor --json` 的 `remote` 段。
# =============================================================================
# ── --falsify：注入探针，验证闸门**自己**能判红 ──────────────────────────
# ⚠️ 契约 §2.1 能力③ + runner:131 的要求：反向模式**不跑 self_test**，
#    而是调闸门自己的 `--falsify`。原因：self_test 返回非0 有两种原因
#    （真的在验证 / 无条件失败），从退出码上不可区分。
# ⚠️ 2026-10-04 实测踩坑：初版没有 --falsify，导致本闸门无法登记进 manifest
#    （runner 需要它），等于**闸门存在但 CI 永不覆盖**。
if [[ "$FALSIFY" == "1" ]]; then
  # ⚠️ 绝不能写成 `if FALSIFY; then exit 1; fi` —— 那样「什么都不检查」与
  #    「永远判红」的闸门都能通过自证。正解：**注入探针 → 照常跑 R1 的检查 →
  #    由检查结果决定红绿**（对齐 G26 的做法）。
  export QUILL_DOCTOR_JSON='{"remote":{"tables_present":["remote_nodes"],"backup_contains_remote":true,"nodes_count":1,"grants_count":0,"credential_plaintext_detected":true}}'
  printf '  SELF-TEST 注入探针：credential_plaintext_detected=true（真实违规内容，R1 必然命中）\n' >&2
  FALSIFY_PROBE=1
fi

DOCTOR_JSON="${QUILL_DOCTOR_JSON:-}"

# 🔴 性能：初版每取一个字段就 spawn 一次 python3（单次闸门 5.5s × 15 用例 = 97s，
#    慢到会被 CI 当成超时）。→ 改为**一次解析，输出 key=value 行**，后续用 grep 取。
#    纯性能优化，语义不变（仍只读 doctor --json，不碰 DB）。
DOCTOR_FIELDS=""
if [ -n "$DOCTOR_JSON" ]; then
  DOCTOR_FIELDS="$("$PY" -c "
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(1)
r = d.get('remote') or {}
def enc(v):
    if isinstance(v, bool): return 'true' if v else 'false'
    if isinstance(v, list):  return ','.join(str(x) for x in v)
    return str(v)
for k, v in r.items():
    print('%s=%s' % (k, enc(v)))
" <<< "$DOCTOR_JSON" 2>/dev/null)" || DOCTOR_FIELDS=""
fi

# 取字段（从已解析的 key=value 行里 grep，不 spawn 进程）
remote_field() { # $1=字段名；找不到返回 1
  local line
  line="$(printf '%s\n' "$DOCTOR_FIELDS" | grep -m1 "^$1=" || true)"
  [ -n "$line" ] || return 1
  printf '%s' "${line#*=}"
}

# ── R1 对端节点登记 ──────────────────────────────────────────────────────
# 🔴 backend v2.5 提供的两个字段回答的是「**恢复成功了吗**」，
#    而不只是「库里现在有 N 个对端」—— 后者无法区分
#    「恢复成功」与「库里本来就有、从未验证过恢复」。
if [ -z "$DOCTOR_JSON" ]; then
  warn "未提供 doctor --json（QUILL_DOCTOR_JSON）—— 无法判定对端登记状态（V1 未接入或库不可读）"
elif ! remote_field tables_present >/dev/null; then
  warn "doctor --json 无 remote 段或库不可读 —— 无法判定（不等于通过）"
else
  # ⚠️ 2026-10-04 实测踩坑（**闸门真 bug，非 harness**）：
  #   优化 remote_field 时把 list 从「长度」改成「逗号连接」，
  #   于是 `tables_present: []` 现在返回**空串**，而下面的判定仍写 `[ "$tables" = "0" ]`
  #   → 空串 ≠ "0" → 落进 else 分支 → **S8「表未建」被判成「全部通过」exit=0**。
  #   🔴 这是**假绿灯**（该黄却绿），比假红危险得多。
  #   → 判定必须覆盖三种形态：非空 / 空串 / 字段缺失。
  tables=$(remote_field tables_present)
  contains=$(remote_field backup_contains_remote)
  if [ "$contains" = "true" ]; then
    ok "恢复后的库含 remote_nodes 数据（$tables）"
  elif [ -z "$tables" ] || [ "$tables" = "0" ]; then
    # 库在但表未建 = V1 未启用跨实例 → 未判定，不是失败
    warn "库在但 remote_* 表未建（V1 未启用跨实例）—— 无法判定，不等于通过"
  else
    n=$(remote_field nodes_count 2>/dev/null || echo "?")
    # 🔴 2026-10-05 修正（主理人实测）：此处原为 ok「对端登记可读，节点数 N（库含表但恢复状态未知）」，
    #   而文案自称「未知」却计入通过 —— **恒绿分支**（第 7 类失效）。
    #   原因：`backup_contains_remote=false` 无法区分两种情形：
    #     ① V1 全新安装、从未录入对端      → 确实正常
    #     ② 从缺 remote 表的备份恢复、覆盖掉原有对端数据 → **数据丢失**
    #   闸门没有「是否发生过恢复」这个判据，因此不能把「无判据」当「有判据的通过」。
    #   → 按三态契约：无法区分时只能 warn（rc=2，绝不计入通过）。
    warn "对端登记可读，节点数 $n，但恢复状态未知（backup_contains_remote=false）—— 无法判定，不等于通过"
  fi
fi

# ── P0-9 明文凭据兜底：**R1 段最外层**，与 tables_present 是否存在无关 ─────
# 🔴 2026-10-05 修正（主理人实测的 P0 缺陷）：这段检查此前被**误嵌在上面的
#   `else`（tables_present 存在）分支内部** → 字段缺失时整段被跳过 →
#   `{"remote":{"credential_plaintext_detected":true}}` 得到 rc=2「无法判定」
#   而**完全没有「检测到明文凭据」这一行**。这是 P0-9 凭据泄漏的**检测旁路**
#   （第 8 类：闸门响得响亮地绿着，但只看了一小片）。
#   → 现在移到最外层：无论 DOCTOR_JSON 是否有 remote 段 / 表是否已建，都必须查。
# ⚠️ 字段**缺失**时不能报 ok「无明文凭据落库/落文件」—— 那等于把「没看」
#    说成「看过且没问题」（同样是恒绿）。→ 缺字段判 warn（rc=2）。
if [ -z "$DOCTOR_JSON" ]; then
  warn "未提供 doctor --json —— 无法判定是否存在明文凭据（不等于通过）"
elif remote_field credential_plaintext_detected >/dev/null; then
  if [ "$(remote_field credential_plaintext_detected)" = "true" ]; then
    fail "检测到明文凭据（credential_plaintext_detected=true）—— 凭据必须只在 secrets.enc"
  else
    ok "无明文凭据落库/落文件"
  fi
else
  warn "doctor --json 无 credential_plaintext_detected 字段 —— 明文凭据状态无法判定（不等于通过）"
fi

# ── R2 跨实例授权（remote_grants）────────────────────────────────────────
# ⚠️ 上游教训：授权状态损坏必须**硬失败**（fail-closed），不得当成「没人授权」
if [ -z "$DOCTOR_JSON" ]; then
  warn "未提供 doctor --json —— 无法判定跨实例授权状态"
elif ! remote_field tables_present >/dev/null; then
  warn "doctor --json 无 remote 段 —— 授权状态无法判定（不等于通过）"
else
  g=$(remote_field grants_count 2>/dev/null || echo 0)
  if [ "$g" = "0" ]; then
    ok "无跨实例授权（V1 默认 deny，符合预期）"
  else
    ok "跨实例授权可读，$g 条"
  fi
fi

# ── R3 传输层配置（标准 WS + 反向代理）───────────────────────────────────
# ★ 与 iroh 无关：**不需要任何中继/打洞/额外端口**
# ⚠️ 2026-10-04 实测踩坑：改 R1/R2 时把 ENV_FILE 的定义连带删掉了，
#   `set -u` 直接 abort（unbound variable）→ 脚本在 line 113 崩掉。
#   **崩掉的闸门比报错的闸门更糟**：它连"发现了什么"都说不出来。
ENV_FILE="$CFG_DIR/quill.env"
if [ ! -f "$ENV_FILE" ]; then
  warn "无 $ENV_FILE —— 无法判定传输层配置"
elif [ ! -f "$CFG_DIR/tls_cert.pem" ] && [ ! -f "$CFG_DIR/tls_cert.crt" ]; then
  fail "未找到 TLS 证书 —— 对端 base_url 若为 https/wss 而无证书，公网暴露即违反铁律四"
else
  ok "TLS 证书存在"
fi

# ── R4 备份卫生（凭据与私钥都不进自动备份）───────────────────────────────
BD="${BACKUP_DIR:-/nonexistent}"
if [ -d "$BD" ]; then
  if [ -e "$BD/secrets.enc" ] && [ "${ALLOW_SECRET_BACKUP:-0}" != "1" ]; then
    fail "备份目录含 secrets.enc —— 密文与钥匙同处会扩大密钥暴露面（应与 master.key 同策，用户自行保管）"
  else
    ok "备份目录未包含 secrets.enc"
  fi
else
  ok "备份目录未配置或不存在（跳过备份卫生检查）"
fi

# ── --falsify 自检收尾：探针必须被检出 ────────────────────────────────────
# ⚠️ runner:157-166 的判定：rc=2 且含 "SELF-TEST FAILED" = **检测逻辑已失效**
#    （不是「环境不支持」）。探针是真实违规内容，未命中只有一个解释。
if [[ "${FALSIFY_PROBE:-0}" == "1" && "$BAD" -eq 0 ]]; then
  printf '  x %s SELF-TEST FAILED: 注入的探针（明文凭据）未被 rule %s 检出\n' "$RID" "$RID" >&2
  printf '    未命中 => **检测逻辑已失效**，本闸门无法自证。\n' >&2
  printf '    这不是「没有问题」，而是「闸门没在检查」。\n' >&2
  exit 2
fi

printf '\n'
if   [ "$BAD" -gt 0 ];   then printf '结论：检出 %d 个违规，%d 项无法判定。\n' "$BAD" "$UNDET"; exit 1
elif [ "$UNDET" -gt 0 ]; then printf '结论：无违规，但有 %d 项无法判定（不等于通过）。\n' "$UNDET"; exit 2
else                         printf '结论：全部通过。\n'; exit 0
fi
