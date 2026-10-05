#!/usr/bin/env bash
# ==============================================================================
# test-newtype-guard.sh —— 反向自证：身份 newtype 真的能挡住误用
# ==============================================================================
# 主理人维护。
#
#   用法：bash crates/quill-testkit/scripts/test-newtype-guard.sh
#   退出码：0=合规 / 1=违规 / 2=无法判定
#
# ⚠️ **本文件放在 `crates/quill-testkit/scripts/` 而非根 `scripts/`**：
#   根 `scripts/**` 与根 `Cargo.toml` 的维护权不在 M2-1 派工范围内。
#   ⚠️ 因此本检查**未进入** `scripts/gates.manifest.json`，也未被 CI 调用 ——
#   登记与接线须主理人决定。在那之前它属于「已写但未接入 CI 的检查」，
#   不能算覆盖，只能算存在。
#
# ⚠️ `compile_fail` doctest 只要「编译失败」就通过，**不检查失败原因**。
#   若用例本身写错（打错函数名），它同样「通过」= 假闸门。
#   本脚本额外断言报错文本里出现 `mismatched types` 与具体类型名，
#   才敢说「它是因为类型不匹配而拒绝」。
#
# ⚠️ 本脚本是新闸门，**尚未进入 scripts/gates.manifest.json**（登记由主理人做）。
#    在登记之前它属于「已写但未接入 CI 的检查」——不能算覆盖，只能算存在。
# ==============================================================================
set -uo pipefail

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; DIM=$'\033[2m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; DIM=''; RESET=''
fi

# ⚠️ 本脚本在 `crates/quill-testkit/scripts/` 下，故仓库根要上溯 **两级**
# （scripts → quill-testkit → crates → 仓库根）。上溯一级会停在 crates/，
# 导致后续相对路径全部找不到，并被判成「无法判定」。
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$REPO_ROOT" || {
  printf '%s\n' "${YELLOW}▲ 无法判定：无法进入仓库根（未检查，不计入通过）${RESET}" >&2
  exit 2
}

FIXTURE="crates/quill-testkit/tests/compile_fail/newtype_misuse.rs"
OUT="/tmp/newtype-guard.$$.out"

# --- 参数：--falsify（注入确定坏输入，走**同一段**判定逻辑）-----------------
FALSIFY=0
for a in "$@"; do
  case "$a" in
    --falsify) FALSIFY=1 ;;
    -h|--help) sed -n '3,8p' "$0"; exit 0 ;;
    *) printf '%s\n' "${YELLOW}▲ 无法判定：未知参数 $a${RESET}" >&2; exit 2 ;;
  esac
done

# 🔴 自证探针开关：用**同一个脚本**跑「合法用例」当探针。
#   合法用例编译**成功** → 脚本必须判定为「NTG-1 已知误用法未被拦住」并退出 1。
#   若它反而退出 0，说明这段判定已经分不清「拦住了」与「没拦住」= 检测逻辑失效。
#   用环境变量而不是复制一份代码，是为了让自证与正常模式**跑完全相同的路径**。
NTG_PROBE_LEGAL="${NTG_PROBE_LEGAL:-0}"

# --- 环境前置：缺工具必须 rc=2「无法判定」，不许兜底通过（铁律二）---
if ! command -v cargo >/dev/null 2>&1; then
  if [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
  else
    printf '%s\n' "${YELLOW}▲ 无法判定：本机无 cargo（编译验证无法执行，本项不计入通过）${RESET}" >&2
    exit 2
  fi
fi

if [[ ! -f "$FIXTURE" ]]; then
  printf '%s\n' "${YELLOW}▲ 无法判定：fixture 不存在 ${FIXTURE}（未检查，不计入通过）${RESET}" >&2
  exit 2
fi

printf '%s\n' "── newtype 编译期隔离验证 ──"
printf '%s\n' "  fixture: ${FIXTURE}"

# 自证探针：把 fixture 换成「全部合法」的等价文件。
# 它与真实 fixture 的差别**只有类型对不对**，其余完全一致 ——
# 因此「判红」只可能来自类型隔离本身，不可能来自环境或语法。
if [[ "$NTG_PROBE_LEGAL" == "1" ]]; then
  PROBE_DIR="$(mktemp -d)"
  # 🔴 探针必须**真的是合法的**：它与真实 fixture 的唯一差别是「类型对不对」。
  #   一旦这里用了不存在的 API（如 `ExpertId::new`），编译失败原因会变成
  #   「方法找不到」而不是「类型不匹配」——自证就变成了**因错误原因判红**，
  #   比不自证更坏。构造方式必须与 newtype_misuse.rs 里**已验证可用**的写法一致。
  cat >"$PROBE_DIR/probe_legal.rs" <<'PROBE'
use quill_adapters::{ExpertId, MemberId, SessionId, UserId};
fn takes_user(_u: UserId) {}
fn takes_member(_m: MemberId) {}
fn takes_expert(_e: ExpertId) {}
fn main() {
    takes_user(UserId::from_bytes([1u8; 16]));
    takes_member(MemberId::parse("cost-analyst-1").expect("合法"));
    takes_expert(ExpertId::parse("cost-analyst").expect("合法"));
    let s: SessionId = SessionId::from_bytes([2u8; 16]);
    let _ = s.to_compact_hex();
}
PROBE
  FIXTURE="$PROBE_DIR/probe_legal.rs"
  printf '%s\n' "  ⚠ 自证探针模式：fixture 已替换为**全部合法**的等价文件"
fi

# 用 `rustc --emit=metadata` 单独编译该文件（不进 workspace，避免污染全量测试）
# 契约层 rlib 由 cargo 现场编译出来，保证验证的是当前 HEAD 的真实类型。
LIB_DIR="$(mktemp -d)"
trap 'rm -rf "$LIB_DIR" "$OUT" ${PROBE_DIR:-}' EXIT

if ! cargo build -p quill-adapters --quiet 2>"$OUT"; then
  printf '%s\n' "${RED}✗ 无法判定：quill-adapters 自身编译失败，验证无意义${RESET}" >&2
  sed -n '1,20p' "$OUT" >&2
  exit 2
fi

RLIB="$(ls -t target/debug/deps/libquill_adapters-*.rlib 2>/dev/null | head -n 1)"
if [[ -z "$RLIB" ]]; then
  printf '%s\n' "${YELLOW}▲ 无法判定：找不到 libquill_adapters rlib（已检查 1 个 target 目录）${RESET}" >&2
  exit 2
fi
# ⚠️ **必须挑最新的 rlib**：本机 target/debug/deps 里可能残留**另一个 rustc
# 版本**编出来的同名 rlib（实测踩到：1.98 编的旧 rlib + 当前 1.99 的 rustc
# → E0514 incompatible version）。取错版本会让「误用被拒绝」变成
# 「因工具链不匹配被拒绝」——两者 rc 都是 1，但**结论完全不同**。
printf '%s\n' "  rlib: $(basename "$RLIB")（$(date -r "$RLIB" '+%Y-%m-%d %H:%M')）"
printf '%s\n' "  rustc: $(rustc --version)"

# --- 编译「已知错误用法」：必须失败 ---
rustc --edition 2021 --crate-type bin \
  --extern "quill_adapters=$RLIB" -L dependency=target/debug/deps \
  --out-dir "$LIB_DIR" "$FIXTURE" >"$OUT" 2>&1
rc=$?

if (( rc == 0 )); then
  if [[ "$NTG_PROBE_LEGAL" == "1" ]]; then
    # 🔴 探针模式下「编译通过」是**预期**的，这里不得输出"类型退化"这种结论性断言 ——
    #   那会把一次正常的自证说成一次真实故障。
    printf '%s\n' "${YELLOW}△ 自证探针：注入的合法 fixture 编译通过 —— 本检查据此判出「未拦住」${RESET}" >&2
  else
    printf '%s\n' "${RED}✗ rule NTG-1: 已知误用法编译通过了 —— 身份类型退化成了别名${RESET}" >&2
    printf '%s\n' "  把 SessionId / MemberId / ExpertId 传给 takes_user(UserId) 竟无报错，" >&2
    printf '%s\n' "  说明它们都是 String 的别名，编译期隔离已失效。" >&2
  fi
  exit 1
fi

# --- 失败原因必须核对：mismatched types + 具体类型名 ---
misses=0
for token in 'mismatched types' 'expected `UserId`' 'found `SessionId`' 'found `MemberId`' 'found `ExpertId`'; do
  if ! grep -qF "$token" "$OUT"; then
    printf '%s\n' "${YELLOW}▲ 报错文本缺少 ${token}：可能是因别的原因失败${RESET}" >&2
    misses=$((misses + 1))
  fi
done

# 🔴 工具链不匹配必须**单独识别**为「无法判定」，不能混进「违规」：
# E0514 说明 rlib 与 rustc 不同版本，此时「编译失败」与「类型隔离」毫无关系。
if grep -qF 'E0514' "$OUT" || grep -qF 'incompatible version of rustc' "$OUT"; then
  printf '%s\n' "${YELLOW}▲ 无法判定：rlib 与当前 rustc 版本不匹配（E0514），${RESET}" >&2
  printf '%s\n' "  本项**不计入通过也不计入违规** —— 请 cargo clean 后重跑。" >&2
  exit 2
fi

if (( misses > 0 )); then
  printf '%s\n' "${RED}✗ rule NTG-1: 编译失败了，但原因不是类型不匹配（缺 ${misses} 项）${RESET}" >&2
  printf '%s\n' "  —— 这说明 fixture 自身有问题，本项不能作为「隔离有效」的证据。" >&2
  sed -n '1,40p' "$OUT" >&2
  exit 1
fi

# --- 正向：合法用法必须能编译（防误报闸门，铁律十二）---
cat >"$LIB_DIR/legal.rs" <<'LEGAL'
use quill_adapters::{SessionId, UserId};
fn takes_user(_u: UserId) {}
fn main() {
    let u = UserId::from_bytes([1u8; 16]);
    takes_user(u);
    let s: SessionId = SessionId::from_bytes([2u8; 16]);
    let _ = s.to_compact_hex();
}
LEGAL
if ! rustc --edition 2021 --crate-type bin \
      --extern "quill_adapters=$RLIB" -L dependency=target/debug/deps \
      --out-dir "$LIB_DIR" "$LIB_DIR/legal.rs" >"$OUT" 2>&1; then
  if grep -qF 'E0514' "$OUT" || grep -qF 'incompatible version of rustc' "$OUT"; then
    printf '%s\n' "${YELLOW}▲ 无法判定：合法用例因 rlib/rustc 版本不匹配而失败（E0514），不计入通过${RESET}" >&2
    exit 2
  fi
  printf '%s\n' "${RED}✗ rule NTG-2: 合法用法编译失败 —— 本检查成了「一律禁止」的假闸门${RESET}" >&2
  sed -n '1,40p' "$OUT" >&2
  exit 1
fi

printf '%s\n' "${GREEN}✓ rule NTG-1: 3 处已知误用（SessionId/MemberId/ExpertId → UserId）全部被编译器拒绝${RESET}"
printf '%s\n' "${GREEN}✓ rule NTG-2: 合法用法（UserId → UserId）编译通过（已检查 1 例，不误报）${RESET}"

# --- --falsify：证明这段判定**分得清**「拦住了」与「没拦住」----------------
# 🔴 判据取被测脚本的**退出码**，输出字样只作佐证（AGENTS.md 铁律十九）。
#    一个"永远退出 0"的检测器在这里会被判 rc=2 而不是被骗过去。
if (( FALSIFY )); then
  probe_out="$(mktemp)"
  NTG_PROBE_LEGAL=1 bash "$0" >"$probe_out" 2>&1
  prc=$?
  sed 's/^/    /' "$probe_out" >&2
  rm -f "$probe_out"
  if (( prc == 1 )); then
    printf '%s\n' "${RED}✗ NTG rule SELF-TEST OK: 注入的「全部合法」探针被判为 NTG-1 违规（退出码 1 为预期）—— 证明本检查确实靠类型不匹配判红，不是恒绿${RESET}" >&2
    exit 1
  fi
  printf '%s\n' "${RED}✗ NTG SELF-TEST FAILED: 注入的合法探针未被判红（退出码 ${prc}，期望 1）${RESET}" >&2
  printf '%s\n' "  这不是「没有问题」，而是**检测逻辑已失效**：它分不清拦住了与没拦住。" >&2
  exit 2
fi

exit 0
