#!/usr/bin/env bash
# =============================================================================
# check-tunnel-protection-boundary.sh —— G36 HTTP 隧道三项防护策略边界
# =============================================================================
# 契约（FALSIFY_CONTRACT.md §2.1，勿删，runner 依赖）：
#   用法：bash scripts/check-tunnel-protection-boundary.sh [--falsify]
#   退出码：0=通过 / 1=有问题 / 2=无法判定
#   规则：G36-1 SSRF 解析后判网段 / G36-2 元数据永远禁止
#         G36-3 路径白名单方法级+默认拒绝 / G36-4 local-only 面失效即判红
#         G36-5 敏感头剥离含 authorization / G36-6 注入连接所有者凭据
#
# 归属（07_实例间委派设计.md §责任边界）：三项防护的**实现**在共享 crate
#   `quill-bridge`（两端共用，物理位置由主理人指定）；**本闸门守边界**：
#   ① 策略规范自身的三项防护不得被削弱
#   ② Rust 实现落地后，必须与本规范逐条一致（当前 crate 未建 → 未判定）
#
# 🔴 三态纪律（铁律十四/十六）：
#   `quill-bridge` 尚未建立时，本闸门对「Rust 侧一致性」判**未判定(2)**，
#   绝不因「文件不存在」而假装通过。
# =============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SPEC="$ROOT/scripts/tunnel_policy_spec.py"
SELF_TEST="$ROOT/scripts/test_tunnel_policy.py"
BRIDGE_CRATE="$ROOT/crates/quill-bridge"

FALSIFY=0
# 🔴 2026-10-05 修复（主理人实测发现的根因缺陷，勿改回单参数写法）：
#   原实现 `[[ "${1:-}" == "--falsify" ]] && FALSIFY=1` **只认第一个参数**，
#   而 run-gate-selftest.sh:159 按契约调用的是
#       bash <gate> "$TARGET_DIR" --falsify
#   → $1 是临时目录而非 --falsify → FALSIFY 恒为 0 → **故障注入从未发生**。
#   实测两种调用行为不一致（铁律十一：先假设自己错）：
#       仅 --falsify        → 98 通过/10 失败，rc=1（真注入）
#       $DIR --falsify      → 108 通过/0 失败，rc=2（没注入，rc=2 来自 G36-7 未判定）
#   → runner 只能把它记成「环境不支持（本环境无法判定）」，
#     那是个**假标签**：真问题是反向自证从未运行，鉴别力从未被验证。
#   → 必须遍历**全部**参数（与 quill-doctor-roaming.sh:23 同一写法）。
for a in "$@"; do [[ "$a" == "--falsify" ]] && FALSIFY=1; done

BAD=0
rule(){ # $1=rule_id $2=ok|fail $3=说明
  if [[ "$2" == "ok" ]]; then
    printf '  o %-7s %s\n' "$1" "$3"
  else
    printf '  x %-7s %s\n' "$1" "$3" >&2
    BAD=$((BAD+1))
  fi
}
undet(){ printf '  ▲ %-7s %s（无法判定，不等于通过）\n' "$1" "$2"; }

PY="${PYTHON:-python3}"
command -v "$PY" >/dev/null 2>&1 || { printf '❌ 缺工具：%s（无法判定，非通过）\n' "$PY" >&2; exit 2; }
[[ -f "$SPEC" ]] || { printf '❌ 缺策略规范：%s\n' "$SPEC" >&2; exit 2; }

echo "═══ G36 隧道三项防护策略边界 ═══"

# ── 用例探针：--falsify 时对规范注入「默认拒绝」被破坏的故障 ──────────────
PROBE_DIR="$(mktemp -d)"
trap 'rm -rf "$PROBE_DIR"' EXIT
if [[ "$FALSIFY" == "1" ]]; then
  # 注入 G1：把「默认拒绝」改成「默认放行」——最危险的假闸门
  "$PY" - "$SPEC" "$PROBE_DIR/spec_faulty.py" <<'PYEOF'
import io, sys
src = io.open(sys.argv[1], encoding="utf-8").read()
old = "    # ★ 默认拒绝：走到这里说明没有任何规则匹配\n    return False"
new = "    # 故障注入：默认放行\n    return True"
if old not in src:
    sys.stderr.write("锚点未找到，策略源码已变\n"); sys.exit(3)
io.open(sys.argv[2], "w", encoding="utf-8", newline="").write(src.replace(old, new, 1))
PYEOF
  [[ $? -eq 0 ]] || { printf '❌ 故障注入失败（策略源码已变）\n' >&2; exit 2; }
  SPEC="$PROBE_DIR/spec_faulty.py"
  # 🔴 关键自检：注入必须**真的生效**。若注入后自检仍全绿，
  #    说明锚点失效或变量没传通 → 此时必须判红，不能装作注入成功。
  export TUNNEL_POLICY_SPEC="$SPEC"
  if "$PY" "$SELF_TEST" >/dev/null 2>&1; then
    printf '  x 故障注入未生效（注入后自检仍全绿）—— 闸门无鉴别力，不构成有效自检\n' >&2
    exit 1
  fi
  echo "（已注入 G1 故障：白名单默认放行，且已确认自检判红）"
  # 🔴 契约要求：反向自检输出必须含 `SELF-TEST` 标记 + `rule <ID>`，
  #    runner 靠这两者区分「真实检测判红」与「无条件报红的假自证」
  #    （run-gate-selftest.sh:170/172）。缺任一 → runner 判「假自证」或
  #    「失败信息不精确」，本闸门在反向模式下就无法被认证。
  #    标注的规则与注入点严格对应：注入改的正是**路径白名单的默认拒绝**。
  printf '  SELF-TEST: 注入故障已被真实检测路径判红（rule G36-3 路径白名单默认拒绝被削弱）\n'
fi

# ── G36-1..G36-6：跑策略自检，把结果映射到规则 ────────────────────────────
# ⚠️ 2026-10-04 实测踩坑：自检读的是**环境变量** TUNNEL_POLICY_SPEC，
#    而本闸门改的是 shell 局部变量 SPEC —— 两者未连通，导致 --falsify
#    注入的故障**根本没传给自检**，闸门在故障下仍报「策略自检 99 通过」。
#    这是一个「看起来在工作、实际什么都没做」的典型。
#    → 必须显式 export，shell 变量不会自动进子进程环境。
export TUNNEL_POLICY_SPEC="$SPEC"
SELFOUT="$("$PY" "$SELF_TEST" 2>&1)"; SELFRC=$?
TOTAL_OK="$(printf '%s' "$SELFOUT" | grep -oE '[0-9]+ 通过' | grep -oE '[0-9]+' | head -1)"
TOTAL_BAD="$(printf '%s' "$SELFOUT" | grep -oE '[0-9]+ 失败' | grep -oE '[0-9]+' | head -1)"
TOTAL_OK="${TOTAL_OK:-0}"; TOTAL_BAD="${TOTAL_BAD:-0}"

if [[ "$SELFRC" -eq 0 ]]; then
  rule "G36-1" ok "SSRF 按 DNS 解析结果判网段（防 rebinding）"
  rule "G36-2" ok "元数据/loopback 永远禁止（不受知情授权影响）"
  rule "G36-3" ok "路径白名单方法级 + 默认拒绝"
  rule "G36-4" ok "local-only 面（auth/admin/control/metrics）不可穿透"
  rule "G36-5" ok "敏感头剥离含 authorization/cookie（大小写不敏感）"
  rule "G36-6" ok "隧道层注入连接所有者凭据，覆盖对端自带 auth"
else
  # 有失败用例 → 逐条判定哪些规则真被破坏
  # ⚠️ 2026-10-04 修正：初版用「输出里出现某关键词 → 该规则判红」的粗粒度映射，
  #    结果注入 G1（只破路径白名单）却把 G36-1/2/5/6 也全判红 —— **误报**。
  #    误报会让闸门「红了但原因不对」，排障时比不报更费时间。
  #    → 改为按**失败用例所属防护域**精确归因。
  mapfile -t FAILED_LINES < <(printf '%s' "$SELFOUT" | grep -E '^\s+x ' || true)
  ssrf_bad=false; path_bad=false; hdr_bad=false; inject_bad=false
  for ln in "${FAILED_LINES[@]}"; do
    case "$ln" in
      *"元数据"*|*"rebinding"*|*"IPv4-mapped"*|*"loopback"*|*"私网"*|*"DNS"*|*"scheme"*|*"URL"*|*"纵深防御"*)
        ssrf_bad=true ;;
      esac
    case "$ln" in
      *"路径"*|*"委派"*|*"方法"*|*"local-only"*|*"默认拒绝"*|*"未知路径"*|*"专家"*|*"穿越"*|*"注入"*|*"斜杠"*|*"大小写"*|*"换行"*|*"CR "*|*"非 /"*|*"wiki"*|*"用户管理"*)
        path_bad=true ;;
    esac
    case "$ln" in
      *"剥离"*|*"保留"*|*"STRIP"*|*"KEEP"*|*"入参"*)
        hdr_bad=true ;;
    esac
    case "$ln" in
      *"所有者"*|*"注入成功"*)
        inject_bad=true ;;
    esac
  done
  [[ $ssrf_bad   == true ]] && rule "G36-1" fail "SSRF 判定被削弱" || rule "G36-1" ok "SSRF 按 DNS 解析结果判网段（防 rebinding）"
  [[ $ssrf_bad   == true ]] && rule "G36-2" fail "元数据/loopback 封禁被削弱" || rule "G36-2" ok "元数据/loopback 永远禁止"
  [[ $path_bad   == true ]] && rule "G36-3" fail "路径白名单（方法级/默认拒绝）被削弱" || rule "G36-3" ok "路径白名单方法级 + 默认拒绝"
  [[ $path_bad   == true ]] && rule "G36-4" fail "local-only 面保护被削弱" || rule "G36-4" ok "local-only 面不可穿透"
  [[ $hdr_bad    == true ]] && rule "G36-5" fail "敏感头剥离被削弱" || rule "G36-5" ok "敏感头剥离含 authorization/cookie"
  [[ $inject_bad == true ]] && rule "G36-6" fail "连接所有者凭据注入被削弱" || rule "G36-6" ok "隧道层注入连接所有者凭据"
  echo >&2
  echo "失败用例（${TOTAL_BAD} 条）：" >&2
  printf '  %s\n' "${FAILED_LINES[@]}" >&2
fi

echo
echo "策略自检：${TOTAL_OK} 通过 / ${TOTAL_BAD} 失败"

# ── Rust 侧一致性：`quill-bridge` 未建 → 未判定（绝不假装通过）────────────
echo
if [[ ! -d "$BRIDGE_CRATE" ]]; then
  undet "G36-7" "crates/quill-bridge 尚未建立 → Rust 实现与本规范的一致性**未判定**（V1 落地后须补 G36-7 校验）"
  echo
  echo "结论：策略侧通过；Rust 侧未判定。G36-7 需在 quill-bridge 落地后启用。"
  # 有未判定项 → 按三态契约，退出码 2（绝不计入通过）
  [[ "$BAD" -eq 0 ]] && exit 2
  exit 1
fi

# crate 已存在 → 校验 Rust 侧关键防护是否落地
# ⚠️ 三态必须分清（2026-10-04 实测修正）：
#   crate **只有骨架占位**（`pub mod placeholder`）→ 这是「尚未实现」= **未判定(2)**，
#     不是「实现错了」= 违规(1)。把「还没写」报成「写错了」会让闸门噪声化，
#     久而久之被无视 —— 那才是真正的失守。
#   只有当 crate 里**已经有真实防护代码**却缺某项，才判违规。
RUST_SRC="$BRIDGE_CRATE/src"
# ⚠️ 三条判据（backend 2026-10-04 指出，初版判据不够硬）：
#   backend 的骨架里有 `pub mod placeholder` + 一个 `#[test] fn crate_compiles_and_test_runs`
#   （`assert_eq!(2+2, 4)`）—— **朴素的「有没有 fn」判据会把它判成「有实现」**。
#   他称之为「典型的『看起来在工作』的假信号：能跑、会绿、但什么都没测」。
#   → 故采用三条独立判据，任一不满足即判「骨架/未判定」：
#     ① 存在**非 #[cfg(test)]** 的 `pub fn is_tunnel_path_allowed`（排除测试占位 fn）
#     ② 存在**非 #[cfg(test)]** 的 SSRF 判据（排除只有编译测试的骨架）
#     ③ 源码**不含** `todo!()` / `unimplemented!()`（铁律六：排除「写了签名但空实现」）
has_real_impl() {
  local f
  # 判据 ③：任何 todo!/unimplemented! → 一律判未实现
  if grep -rqE 'todo!\(\)|unimplemented!\(\)' "$RUST_SRC" 2>/dev/null; then
    return 1
  fi
  # 判据 ①：白名单入口必须是真实 pub fn，不能只是 #[cfg(test)] 里的
  if ! grep -rqE 'pub (async )?fn is_tunnel_path_allowed' "$RUST_SRC" 2>/dev/null; then
    return 1
  fi
  # 判据 ②：SSRF 判据入口
  if ! grep -rqE 'pub (async )?fn (check_peer_url|classify_ip)' "$RUST_SRC" 2>/dev/null; then
    return 1
  fi
  return 0
}

if ! has_real_impl; then
  undet "G36-7" "quill-bridge 仍是骨架（placeholder），三项防护**尚未实现** → Rust 侧一致性未判定（V1 落地后本项转为强制）"
  echo
  echo "结论：策略侧通过（${TOTAL_OK} 项）；Rust 侧未判定。G36-7 随 quill-bridge 实现推进自动生效。"
  [[ "$BAD" -eq 0 ]] && exit 2
  exit 1
fi

# 有真实实现 → 逐项校验
MISSING=()
grep -rqE '169\.254\.169\.254|169\.254\.0\.0/16|BLOCKED_NETWORK' "$RUST_SRC" 2>/dev/null \
  || MISSING+=("元数据网段封禁")
grep -rqE 'getaddrinfo|to_socket_addrs|lookup_host' "$RUST_SRC" 2>/dev/null \
  || MISSING+=("DNS 解析后校验（防 rebinding）")
grep -rqE 'authorization' "$RUST_SRC" 2>/dev/null \
  || MISSING+=("authorization 剥离/注入")
grep -rqE 'is_tunnel_path_allowed|TUNNEL_PATH_RULE|fn is_tunnel' "$RUST_SRC" 2>/dev/null \
  || MISSING+=("路径白名单入口")
grep -rqE 'false' "$RUST_SRC" 2>/dev/null \
  || MISSING+=("默认拒绝")

if [[ "${#MISSING[@]}" -gt 0 ]]; then
  for m in "${MISSING[@]}"; do rule "G36-7" fail "quill-bridge 已有实现但缺少：$m"; done
  echo; echo "结论：Rust 侧与规范不一致（${#MISSING[@]} 项缺失）。"
  exit 1
fi
rule "G36-7" ok "quill-bridge 关键防护已落地"

echo
if [[ "$BAD" -eq 0 ]]; then
  echo "结论：全部通过（${TOTAL_OK} 项）。"
  exit 0
fi
echo "结论：检出 $BAD 处违规。"
exit 1
