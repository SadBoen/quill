#!/usr/bin/env bash
# ==============================================================================
# check-boundary-singletons.sh —— G26 单例退化防线 + 绕过检测
#
# 契约（FALSIFY_CONTRACT.md §2.1，勿删，runner 依赖）：
#   用法：bash scripts/check-boundary-singletons.sh <crates 目录> [--falsify|--selftest]
#   退出码：0=通过 / 1=有问题 / 2=无法判定
#   输出：违规时 stderr 含 "rule <ID>"；--falsify 报错时含 "SELF-TEST"
#
# 🔴 --falsify 的实现要点（2026-10-04，按契约 §2.1 修正版）：
#   **绝不能写成 `if FALSIFY; then exit 1; fi`** —— 那样"什么都不检查"的空闸门
#   与"永远判绿"的恒绿闸门都能通过自证（实测：两者 --falsify 同样返回 rc=1）。
#   正解：**注入探针文件 → 照常跑 G26-1 的 grep → 由 grep 结果决定红绿**。
#   探针用 `mktemp -d` 独立目录，不污染真实 crates/，也避免"调用→清理→再调用"竞态。
# ==============================================================================
set -uo pipefail

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; RESET=''
fi

# 🔴 参数解析：先剔除 mode 标志，再取第一个位置参数为 CRATES。
#    向后兼容：`check-boundary-singletons.sh crates` 与 `... crates --falsify` 都可用。
#    ⚠️ set -u 下**不能**用 `(( MODE == falsify ))` —— bash 会把字符串当算术上下文，
#       直接报 `unbound variable`。必须用 [[ ]] 字符串比较。
MODE=normal
CRATES=""
for a in "$@"; do
  case "$a" in
    --falsify)  MODE=falsify ;;
    --selftest) MODE=selftest ;;
    -*) printf '用法: %s [crates 目录] [--falsify|--selftest]\n' "$0" >&2; exit 2 ;;
    *)  [[ -z "$CRATES" ]] && CRATES="$a" ;;
  esac
done
[[ -z "$CRATES" ]] && CRATES="crates"

# 🔴 被禁符号的统一模式（G26-1/2 用于 crates/ 源码，G26-5 用于 build.rs）
readonly PAT_ANY='Config::global[[:space:]]*\(|SessionManager::instance[[:space:]]*\('

# --- 能力③：--falsify / --selftest -----------------------------------------
# 注入探针 → 照常跑 G26-1 的检测 → 由 grep 结果决定红绿。
# ⚠️ 不提前 exit 1：红必须**由 G26-1 的 grep 得出**，否则就是"无条件报红"。
MARKER="${QUILL_SELFTEST_MARKER:-ZZSELFTESTPROBE}"
if [[ "$MODE" == "falsify" || "$MODE" == "selftest" ]]; then
  # 用 mktemp -d 独立目录：既不污染真实 crates/，也避免"调用→清理→再调用"竞态
  CRATES=$(mktemp -d)
  trap 'rm -rf "$CRATES"' EXIT
  mkdir -p "$CRATES/quill-probe/src"
  cat > "$CRATES/quill-probe/src/${MARKER}.rs" <<EOF
// 自检探针（${MARKER}）—— 不是真实违规，falsify 结束后会被删除。
// 它的唯一作用：让 G26-1 的真实 grep 逻辑跑一遍并命中。
pub fn probe() -> &'static str { "Config::global()" }
EOF
  printf "%s\n" "${YELLOW}── G26 (${MODE}) ── 已注入探针 ${MARKER}.rs（mktemp 目录，不污染真实 crates/）${RESET}"
else
  if [[ ! -d "$CRATES" ]]; then
    printf '%s\n' "${YELLOW}▲ G26 无法判定：目录不存在（${CRATES}）${RESET}" >&2
    printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
    exit 2
  fi
  # 🔴 规范化成绝对路径（qa-engineer 报告的缺陷根因）
  #   若 $CRATES 是相对值而解析出的是绝对值，case 匹配会失败 → G26-3 误报。
  #   Windows / mktemp / 仓库内路径三种调用方式必须行为一致。
  CRATES="$(cd "$CRATES" 2>/dev/null && pwd)" || {
    printf '%s\n' "${YELLOW}▲ G26 无法判定：目录无法解析为绝对路径${RESET}" >&2
    printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2; exit 2; }
  printf '%s\n' "${YELLOW}── G26 ── 检查 ${CRATES}${RESET}"
fi

command -v grep >/dev/null 2>&1 || { echo "${RED}✗ 缺 grep${RESET}" >&2; exit 2; }

# --- 不误报白名单 ---------------------------------------------------------
# ⚠️ 设计教训（实测踩坑，两次）：
#   ① 不能用"整行含 // 就排除"的做法 —— 真实违规常写成
#        let cfg = Config::global();   // ← 违规
#      整行排除会把**违规代码一起滤掉** → 静默漏报（第 1 类假闸门）。
#   ② 块注释状态必须**跨行保持**。若按行重置状态，则独立成行的 `*/`
#      会让下一行被当成正常代码 → 块注释内的违规调用泄漏（第 1 类假闸门）。
#
# 正确做法：**逐文件流式剥离注释**（状态跨行保持），再判断剩余代码。
strip_comments() {
  awk '
    {
      line = $0; out = "";   # ★ inblock 不重置，跨行保持
      while (length(line) > 0) {
        if (inblock) {
          p = index(line, "*/")
          if (p == 0) { line = ""; break }
          line = substr(line, p + 2); inblock = 0
        } else {
          s = index(line, "//"); b = index(line, "/*")
          if (s > 0 && (b == 0 || s < b)) { out = out substr(line, 1, s - 1); break }
          if (b > 0) { out = out substr(line, 1, b - 1); line = substr(line, b + 2); inblock = 1; continue }
          out = out line; break
        }
      }
      print FNR ": " out
    }
  ' "$1"
}

fail=0
skipped=0

# 🔴 count_or_zero —— 计数**唯一**入口（修 2026-10-05 主理人实测的算术语法错）
#
#   缺陷（实测于**真实 crates/**）：`total=$(grep -cE ... || echo 0)`
#     `grep -c` 在**零命中时自己已经输出 "0" 并返回 rc=1**，
#     `|| echo 0` 于是又补一行 "0" → total 变成 "0\n0"（多行字符串）→
#     `for ((k=lit; k<total; k++))` 抛 `syntax error in expression`。
#
#   为什么 23 项自检全绿也盖不住：
#     fixture 树里每个 lib.rs 都含 include!，**零命中这条分支从未被执行过**
#     ——「测试跑了」≠「测了东西」（第 3 类静默失败的教科书案例）。
#     且 `expect()` 把 stderr 丢进 /dev/null，噪音天然不可见。
#
#   正确做法：只取 grep 自己输出的那一行；空/非数字一律归 0（不是"兜底通过"，
#     而是"计数失败时该分支的判定就是 0 个候选"，语义正确）。
count_or_zero() { # $1=ERE  $2=文件 → stdout 一个纯数字
  local n
  n=$(grep -cE "$1" "$2" 2>/dev/null | head -n 1)
  case "${n:-}" in
    ''|*[!0-9]*) n=0 ;;
  esac
  printf '%s' "$n"
}

# 🔴 被禁符号的统一模式（G26-1/2 用于 crates/ 源码，G26-5 用于 build.rs）

# ⚠️ 显式豁免（第 3 陷阱：闸门会扫到"描述自己的字符串"）
#
# 排除对象是**闸门/测试基础设施与 fixture 本身**，因为它们必然包含被禁符号：
#   · quill-testkit/src/fals.rs  —— FALS 自检辅助，存放"已知坏样本"字符串
#   · quill-testkit/src/scan.rs  —— 泄漏扫描器，其测试样本含被禁模式
#   · quill-xtask                 —— 边界规则实现处，规则定义含被禁符号
#   · quill-testkit/fixtures/**   —— 边界 fixture，**故意违规**以验证闸门会红
#     （如 bnd07-singleton-regression 明确写着 Config::global()）
#
# ⚠️ **为什么不能用"路径含 test 就跳过"的宽泛规则**：
#   那样会让 `quill-store/tests/` 之类的真实测试目录也被跳过，
#   而违规代码恰恰可能藏在测试夹具里被 review 放过。
#   故：**按具体文件/目录精确列出**，新增豁免必须显式登记（对齐"每个豁免都要有反向断言"）。
readonly SELF_FILES=(
  "quill-testkit/src/fals.rs"
  "quill-testkit/src/scan.rs"
  "quill-testkit/fixtures"
  "quill-xtask"
)

is_self_file() {
  local rel="$1"
  for s in "${SELF_FILES[@]}"; do
    [[ "$rel" == "$s" || "$rel" == "$s"/* ]] && return 0
  done
  return 1
}

# 🔴 词法路径归约（不访问文件系统）
#   用途：G26-3 判定 include! 目标是否逃出 $CRATES。
#   为什么不用 cd / realpath：
#     ① 目标目录**可能不存在**（BND-08 的 include!("../../elsewhere/hidden.rs")
#        在更外层，甚至仓库外）→ cd 失败 → abs 塌陷成 "/shared.rs" → 边界判断必错
#     ② realpath 在部分 Windows 环境不存在（E14 同类问题）
#   规则：`.` 忽略；`..` 弹栈（弹到根则保持根）；其他段直接追加。
#   传入含变量/非常规字符时返回空 → 调用方保守处理，不静默放过。
lexical_join() {
  local base="$1" rel="$2" seg out=() root="" i
  [[ -n "$base" ]] || { echo ""; return; }
  [[ "$rel" != *'$'* && "$rel" != *'{'* ]] || { echo ""; return; }  # 含插值 → 无法静态判定
  case "$base" in
    /*) root="/" ;;
    *)  root="" ;;                      # base 已是绝对路径（由 $CRATES 规范化保证）
  esac
  # 拆分 base
  IFS='/' read -r -a out <<< "${base#/}"
  # 拆分并归约 rel
  IFS='/' read -r -a seg <<< "$rel"
  for i in "${seg[@]}"; do
    case "$i" in
      ""|".") continue ;;
      "..")  ((${#out[@]} > 0)) && unset 'out[${#out[@]}-1]' ;;
      *)     out+=("$i") ;;
    esac
  done
  local joined
  joined=$(IFS='/'; echo "${out[*]}")
  printf '%s%s' "$root" "$joined"
}

for entry in "Config::global" "SessionManager::instance"; do
  # 🔴 规则编号：稳定标识，供 G08 runner 断言（stderr 必须含 `rule N`）
  #    G26-1 = Config::global()    G26-2 = SessionManager::instance()
  # ⚠️ 编号一旦发布不可改（runner 与 fixture 按它断言）
  case "$entry" in
    "Config::global")           RID="G26-1" ;;
    "SessionManager::instance") RID="G26-2" ;;
    *)                          RID="G26-?" ;;
  esac

  # 逐文件扫描：剥离注释后再匹配，避免"整行排除"把违规代码一起滤掉
  # ⚠️ 用 for 循环（而非 while read）—— 后者跑在子 shell 里，
  #    skipped 计数无法传回父 shell → 豁免数量永远打印不出来。
  hits=""
  for f in $(find "$CRATES" -name '*.rs' -not -path '*/vendor/*' 2>/dev/null | sort); do
    [[ -n "$f" ]] || continue
    rel="${f#"$CRATES"/}"
    if is_self_file "$rel"; then
      skipped=$((skipped + 1))
      continue                                   # 显式豁免
    fi
    found=$(strip_comments "$f" | grep -E "${entry}[[:space:]]*\(" || true)
    [[ -n "$found" ]] && hits="${hits}${found}"$'\n'
  done
  hits="${hits%$'\n'}"

  # 🔴 --falsify 的鉴别力保证（契约 §2.1 修正版）
  #   探针只含 `Config::global()`，因此**只有 G26-1 必然命中**，G26-2 必然不命中。
  #   故此断言**仅对 G26-1 生效**（探针所针对的那条规则）——
  #   若对所有规则都断言，G26-2 会因"探针里没有它的符号"而误报"检测逻辑失效"。
  if [[ "$MODE" == "falsify" && "$RID" == "G26-1" && -z "$hits" ]]; then
    printf '%s\n' "${RED}✗ G26 SELF-TEST FAILED: 注入的探针（${MARKER}.rs）未被 ${RID} 检出${RESET}" >&2
    printf '%s\n' "  ${YELLOW}探针内容是真实违规符号，真实检测逻辑必然命中。${RESET}" >&2
    printf '%s\n' "  ${YELLOW}未命中 ⇒ **检测逻辑已失效**，本闸门无法自证。${RESET}" >&2
    printf '%s\n' "  ${YELLOW}这不是『没有问题』，而是『闸门没在检查』。${RESET}" >&2
    exit 2
  fi

  if [[ -n "$hits" ]]; then
    if [[ "$MODE" == "falsify" ]]; then
      # 🔴 契约 §2.1：--falsify 的红**由 G26-1 的真实 grep 得出**（此处 hits 已非空）
      printf "%s\n" "${RED}✗ G26 rule ${RID} SELF-TEST: 自检样例（${MARKER}，不是真实违规）${RESET}" >&2
    else
      printf "%s\n" "${RED}✗ rule ${RID}: 禁止 ${entry}()${RESET}" >&2
    fi
    printf '%s\n' "$hits" | head -20 | sed 's/^/    /' >&2
    # ⚠️ `grep -c` 零命中时已输出 0 并返回 1 —— 不能写 `|| echo 0`（会变成 "0\n0"）
    n=$(printf '%s\n' "$hits" | grep -c . || true)
    ((n > 20)) && printf '    ... 另有 %d 处\n' "$((n-20))" >&2
    fail=1
  else
    printf '%s\n' "${GREEN}✓ rule ${RID}: 无 ${entry}() 调用${RESET}"
  fi
done

# 让豁免范围在输出里可见（"豁免不能是无人看管的空白"）
# ⚠️ 用 if 而非 ((...))：算术表达式的返回值在 0 时会中断脚本
if ((skipped > 0)); then
  printf '%s\n' "${YELLOW}（已豁免 ${skipped} 个闸门/测试基础设施文件；SELF_FILES 清单见脚本头部）${RESET}" >&2
fi

# ==============================================================================
# G26-3 ~ G26-5：绕过检测
#
# 起因：qa-engineer 构造 BND-08/09/10 并**实测确认**纯 grep 会显示"通过"。
# 这三条不是"补规则"，而是**新的检查能力**。
# ==============================================================================

# --- G26-3：include! 逃逸 -------------------------------------------------
# include! 的目标在 crates/ 外 → 违规代码不进扫描范围，但编译时进二进制。
# 判定：解析 include!("...") 的路径参数，**递归纳入扫描**；
#      若目标逃出 crates/ 边界 → 直接判红（不依赖是否含违规符号）。
echo
printf '%s\n' "── G26-3 include! 逃逸 ──"
inc_escape=0
while IFS= read -r f; do
  [[ -n "$f" ]] || continue
  rel="${f#"$CRATES"/}"
  is_self_file "$rel" && continue
  # 提取 include! 的路径字面量
  # 🔴 两条正则：
  #   ① 紧跟 include!( 的首个字符串字面量 —— 覆盖 include!("x.rs")
  #   ② 其余所有 include!( —— 覆盖 include!(concat!(...)) 这类嵌套宏
  # ⚠️ 绝不能"匹配不到就跳过"：那会把 include!(concat!(env!("OUT_DIR"),...))
  #    静默放过 → 漏报。② 保证它进入判定分支并被显式告警。
  while IFS= read -r target; do
    [[ -n "$target" ]] || continue
    [[ "$target" == "?"* ]] && {
      printf '%s\n' "  ${YELLOW}⚠ rule G26-3: include! 路径无法静态解析（非字面量）${RESET}" >&2
      printf '%s\n' "      ${rel}: include!($target)" >&2
      printf '%s\n' "      ${YELLOW}目标不可静态判定 —— 请人工确认未逃出 crates/${RESET}" >&2
      continue
    }
    # 🔴 纯字符串规范化（不依赖目录存在，也不依赖 realpath）
    #   原写法 `cd A && cd B && pwd` 在 cd 失败时 sub 为空 →
    #   abs 塌陷成 "/shared.rs"（**目录信息全丢**），边界判断必错。
    #   而 BND-08 的 include!("../../elsewhere/hidden.rs") 目标目录
    #   在**更外层**（甚至仓库外），cd 必然失败 → 会被误跳过。
    #
    #   正解：把 base（当前文件所在绝对目录）与 target 的分段做**词法归约**，
    #   不访问文件系统。`..` 弹栈、`.` 忽略、遇无法判定的段则保守判红。
    base_dir=$(dirname "$f")                      # 已是绝对（$f 来自 find $CRATES/...）
    abs=$(lexical_join "$base_dir" "$target")
    if [[ -z "$abs" ]]; then
      # 路径含无法静态判定（如环境变量拼接）→ 保守处理：不静默放过
      printf '%s\n' "  ${YELLOW}⚠ rule G26-3: include! 路径无法静态解析${RESET}" >&2
      printf '%s\n' "      ${rel}: include!(\"$target\")" >&2
      printf '%s\n' "      ${YELLOW}请人工确认目标未逃出 crates/${RESET}" >&2
      continue
    fi
    case "$abs" in
      "$CRATES"/*) : ;;                        # 目标在 crates/ 内 → 合法
      "$CRATES")   : ;;
      *)
        printf '%s\n' "  ${RED}✗ rule G26-3: include! 逃出 crates/ 边界${RESET}" >&2
        printf '%s\n' "      $rel" >&2
        printf '%s\n' "      → $target" >&2
        printf '%s\n' "      ${YELLOW}编译时该文件会进二进制，但不在源码扫描范围内${RESET}" >&2
        inc_escape=1
        ;;
    esac
  done < <(
    # ① 字面量形式
    grep -oE 'include!\s*\(\s*"[^"]*"' "$f" 2>/dev/null \
      | sed -E 's/include!\s*\(\s*"//; s/"$//' || true
    # ② 非字面量形式（嵌套宏/宏调用）→ 用 ? 前缀标记，交由上面显式告警
    total=$(count_or_zero 'include!\s*\(' "$f")
    lit=$(count_or_zero 'include!\s*\(\s*"[^"]*"' "$f")
    # 🔴 不变量断言：lit ≤ total 恒成立，否则说明计数逻辑本身坏了
    #   （一旦失守，for 循环会漏掉 ? 标记 → 静默放过不可静态解析的 include!）
    if (( lit > total )); then
      printf '%s\n' "  ${YELLOW}▲ G26-3 无法判定：include! 计数异常（lit=${lit} > total=${total}）于 ${rel}${RESET}" >&2
      printf '%s\n' "      ${YELLOW}检测逻辑可能已失效 —— 本项不计入通过。${RESET}" >&2
      fail=1
      lit=$total
    fi
    for ((k = lit; k < total; k++)); do echo '?concat!'; done
  )
done < <(find "$CRATES" -name '*.rs' -not -path '*/vendor/*' 2>/dev/null | sort)
((inc_escape)) && fail=1 || printf '%s\n' "  ${GREEN}✓ 无 include! 逃出 crates/ 边界${RESET}"

# --- G26-4：符号链接逃逸 ---------------------------------------------------
# crates/x 软链到 crates/ 外 → find 不跟随，但编译时照样进二进制。
# 策略：**拒绝**任何指向 crates/ 外的软链（最严，且平台无关）。
echo
printf '%s\n' "── G26-4 符号链接逃逸 ──"
link_esc=0
# ⚠️ $CRATES 已在脚本开头规范成绝对路径；readlink -f 返回绝对路径 → 形态一致。
#    若 readlink 不可用（部分 Windows 环境），tgt 回退为原始路径，
#    此时**无法判定** → 明确告警，不静默放过（对齐"未覆盖 ≠ 通过"）。
readlink_ok=1
command -v readlink >/dev/null 2>&1 || readlink_ok=0
while IFS= read -r l; do
  [[ -n "$l" ]] || continue
  if ((readlink_ok)); then
    tgt=$(readlink -f "$l" 2>/dev/null || echo "$l")
  else
    printf '%s\n' "  ${YELLOW}⚠ rule G26-4: 本环境无 readlink，符号链接逃逸无法判定${RESET}" >&2
    printf '%s\n' "      ${l#$CRATES/} —— 请人工确认该链接未指向 crates/ 外${RESET}" >&2
    link_esc=1
    continue
  fi
  case "$tgt" in
    "$CRATES"/*) : ;;
    "$CRATES")   : ;;
    *)
      printf '%s\n' "  ${RED}✗ rule G26-4: 符号链接指向 crates/ 外${RESET}" >&2
      printf '%s\n' "      ${l#$CRATES/} → $tgt" >&2
      printf '%s\n' "      ${YELLOW}编译时照样进二进制；请改为实体文件或移入 crates/${RESET}" >&2
      link_esc=1
      ;;
  esac
done < <(find "$CRATES" -type l 2>/dev/null)
if ((link_esc)); then
  fail=1
elif ((readlink_ok)); then
  # ⚠️ 必须说明"判定了多少个"，否则"一个软链都没有"与"全部合法"输出相同
  #   → 无法区分「没检查」与「检查通过」（第 6/7 类失效）。
  n_links=$(find "$CRATES" -type l 2>/dev/null | wc -l | tr -d ' ')
  if ((n_links == 0)); then
    printf '%s\n' "  ${GREEN}✓ 无指向 crates/ 外的符号链接${RESET}  ${YELLOW}(已检查，${n_links} 个软链)${RESET}"
  else
    printf '%s\n' "  ${GREEN}✓ 无指向 crates/ 外的符号链接${RESET}  ${YELLOW}(已检查 ${n_links} 个)${RESET}"
  fi
else
  # readlink 不可用 → 明确标注"无法判定"，不静默当成通过
  printf '%s\n' "  ${YELLOW}▲ G26-4 无法判定：���环境无 readlink，符号链接逃逸未检查${RESET}" >&2
  printf '%s\n' "    ${YELLOW}这不是「通过」。请在 Linux/macOS 上复验，或用 ls -l 人工确认。${RESET}" >&2
  fail=1
fi

# --- G26-5：build.rs 生成代码 ---------------------------------------------
# build.rs 里 fs::write 生成含违规的 src/*.rs → 构建时才存在，扫 src/ 看不见。
# 策略：**单独扫 build.rs**（很多项目默认排除它）+ 检查其写入目标是否落在源码目录。
echo
printf '%s\n' "── G26-5 build.rs 生成代码 ──"
brs_hits=""
while IFS= read -r b; do
  [[ -n "$b" ]] || continue
  rel="${b#"$CRATES"/}"
  is_self_file "$rel" && continue
  found=$(strip_comments "$b" | grep -E "${PAT_ANY}" || true)
  if [[ -n "$found" ]]; then
    printf '%s\n' "  ${RED}✗ rule G26-5: build.rs 中出现被禁符号${RESET}" >&2
    printf '%s\n' "$found" | head -5 | sed 's/^/      /' >&2
    brs_hits=1
  fi
  # 生成目标落在源码目录 → 即使当前无违规，也报"无法静态判定"
  if strip_comments "$b" | grep -qE 'fs::write|File::create' \
     && strip_comments "$b" | grep -qE 'src/|\.rs"'; then
    printf '%s\n' "  ${YELLOW}⚠ rule G26-5: ${rel} 会生成 .rs 源码，其内容无法静态判定${RESET}" >&2
    printf '%s\n' "      ${YELLOW}该文件构建时才存在 —— 请确保生成内容不含被禁符号${RESET}" >&2
    brs_hits=1
  fi
done < <(find "$CRATES" -name 'build.rs' -not -path '*/vendor/*' 2>/dev/null | sort)
if [[ -z "$brs_hits" ]]; then
  printf '%s\n' "  ${GREEN}✓ build.rs 无被禁符号且不生成源码${RESET}"
else
  fail=1
fi

if ((fail > 0)); then
  cat >&2 <<'EOF'

  为什么这是硬规则：
    单进程多用户方案（02_系统架构 §2.1）成立的前提是**状态经构造函数注入**。
    一旦有人写回 static 全局入口（Config::global() / SessionManager::instance()），
    所有用户会共享同一份状态 → **运行时静默串数据**，且不报错，
    只在特定用户数/并发下才显现 —— 这类 bug 极难在测试期发现。

  正确做法：
    · 配置：Config::new(config_path, service)
    · 会话：SessionManager::new(data_dir)
    · Agent：Agent::with_config(AgentConfig{ session_manager, ... })
EOF
  exit 1
fi

printf '\n%s\n' "${GREEN}═══ G26 通过：crates/ 内无 static 全局入口 ═══${RESET}"
exit 0
