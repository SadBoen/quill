
set -uo pipefail

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; DIM=$'\033[2m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; DIM=''; RESET=''
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$REPO_ROOT" || {
  printf '%s\n' "${YELLOW}▲ 无法判定：无法进入仓库根（未检查，不计入通过）${RESET}" >&2
  exit 2
}

FIXTURE="crates/quill-testkit/tests/compile_fail/newtype_misuse.rs"
OUT="/tmp/newtype-guard.$$.out"

FALSIFY=0
for a in "$@"; do
  case "$a" in
    --falsify) FALSIFY=1 ;;
    -h|--help) sed -n '3,8p' "$0"; exit 0 ;;
    *) printf '%s\n' "${YELLOW}▲ 无法判定：未知参数 $a${RESET}" >&2; exit 2 ;;
  esac
done

NTG_PROBE_LEGAL="${NTG_PROBE_LEGAL:-0}"

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

if [[ "$NTG_PROBE_LEGAL" == "1" ]]; then
  PROBE_DIR="$(mktemp -d)"

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

printf '%s\n' "  rlib: $(basename "$RLIB")（$(date -r "$RLIB" '+%Y-%m-%d %H:%M')）"
printf '%s\n' "  rustc: $(rustc --version)"

rustc --edition 2021 --crate-type bin \
  --extern "quill_adapters=$RLIB" -L dependency=target/debug/deps \
  --out-dir "$LIB_DIR" "$FIXTURE" >"$OUT" 2>&1
rc=$?

if (( rc == 0 )); then
  if [[ "$NTG_PROBE_LEGAL" == "1" ]]; then

    printf '%s\n' "${YELLOW}△ 自证探针：注入的合法 fixture 编译通过 —— 本检查据此判出「未拦住」${RESET}" >&2
  else
    printf '%s\n' "${RED}✗ rule NTG-1: 已知误用法编译通过了 —— 身份类型退化成了别名${RESET}" >&2
    printf '%s\n' "  把 SessionId / MemberId / ExpertId 传给 takes_user(UserId) 竟无报错，" >&2
    printf '%s\n' "  说明它们都是 String 的别名，编译期隔离已失效。" >&2
  fi
  exit 1
fi

misses=0
for token in 'mismatched types' 'expected `UserId`' 'found `SessionId`' 'found `MemberId`' 'found `ExpertId`'; do
  if ! grep -qF "$token" "$OUT"; then
    printf '%s\n' "${YELLOW}▲ 报错文本缺少 ${token}：可能是因别的原因失败${RESET}" >&2
    misses=$((misses + 1))
  fi
done

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
