#!/usr/bin/env bash
# =============================================================================
# 通用闸门自检 runner
# =============================================================================
# 依据：docs/FALSIFY_CONTRACT.md
# 消费：scripts/gates.manifest.json
#
# ── 设计立场：adapter 式，不是"同构假设" ─────────────────────────────────────
# 闸门之间差异极大（静态扫描 / HTTP 行为 / 产物检查），
# 所以 runner **不假设它们同构**，只提供**共享机制**：
#   · 统一加载 manifest
#   · 统一跑三态 + 措辞校验
#   · 统一跑反向自检（--falsify）
#   · 统一报告（通过 / 有问题 / 无法判定 / 已退役 四类，**分开计数**）
#
# 闸门自己的具体怎么测，由它的 self_test 脚本负责。
# =============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# ⚠️ 允许用环境变量指定 manifest —— 供 G35 第 11 项构造假闸门做端到端验证。
#    生产用法不要设这个变量。
MANIFEST="${QUILL_GATES_MANIFEST:-${ROOT}/scripts/gates.manifest.json}"

RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; CYAN=$'\033[36m'; RESET=$'\033[0m'

FALSIFY=0
[[ "${1:-}" == "--falsify" ]] && FALSIFY=1

PY="${PYTHON:-python3}"

# ── 环境依赖：缺工具必须**失败**而非"跳过通过" ──────────────────────────────
for t in python3; do
  command -v "$t" >/dev/null 2>&1 || {
    printf '%s❌ 缺工具：%s（无法判定，非通过）%s\n' "$RED" "$t" "$RESET" >&2
    exit 2
  }
done
[[ -f "$MANIFEST" ]] || { printf '%s❌ 缺 manifest：%s%s\n' "$RED" "$MANIFEST" "$RESET" >&2; exit 2; }

# ── 三态判定 ────────────────────────────────────────────────────────────────
#  0 = 通过        → 计入通过
#  1 = 有问题      → 计入"检出违规"（正常模式下是失败）
#  2 = 无法判定    → 计入"未覆盖"，**绝不计入通过**（铁律十四/十六）
#  其他 = 契约违规 → 报错（闸门实现有问题）
classify() {  # $1=rc ; 输出 pass|fail|undetermined|invalid
  case "$1" in
    0) echo pass ;;
    1) echo fail ;;
    2) echo undetermined ;;
    *) echo invalid ;;
  esac
}

# ── 措辞合规检查（主理人批准的硬要求：机器能判对，人会读错 → 要卡措辞）──────
WORDING_RE='无法判定|未判定 ≠ 通过|▲'
check_wording() {  # $1=gate_id $2=rc $3=output ; 输出 ok|bad，不合格时打印提示
  [[ "$2" -eq 2 ]] || { echo ok; return; }
  if printf '%s' "$3" | grep -qE "$WORDING_RE"; then
    echo ok
  else
    printf '⚠️ %s 退出码为 2 但措辞不合规 —— 人读日志会误判为"通过"\n' "$1" >&2
    echo bad
  fi
}

# ── 加载 manifest ───────────────────────────────────────────────────────────
# ⚠️ 用 \x1f（单元分隔符）而非 \t：bash 的 `read` 会**丢弃尾部空白字段**，
#    导致"retired_at 为空"被读成"有值"，把正常闸门误判成已退役。
#    每行末尾追加哨兵 `_`，保证分隔符不被尾部空白吞掉。
#
# 字段：id │ script │ self_test │ falsifiable │ rules │ verified_by │ retired_at │ retired_reason │ _
mapfile -t ROWS < <("$PY" - "$MANIFEST" <<'PYEOF'
import json, sys
SEP = "\x1f"
d = json.load(open(sys.argv[1], encoding="utf-8"))
for g in d.get("gates", []):
    row = [g["id"], g.get("script", ""), g.get("self_test", ""),
           str(g.get("falsifiable", True)).lower(),
           ",".join(g.get("rules", [])) or "-",
           g.get("verified_by", ""), g.get("retired_at", ""),
           g.get("retired_reason", ""), "_"]
    print(SEP.join(row))
PYEOF
)

(( ${#ROWS[@]} > 0 )) || { printf '%s❌ manifest 里没有任何闸门%s\n' "$RED" "$RESET" >&2; exit 2; }

# ── manifest 合规校验：委托给独立校验器（单一真相源）────────────────────
# ⚠️ 为什么不把规则硬编码在本 runner 里：
#   硬编码在 runner 里，加第五道闸门时容易漏掉某条校验 ——
#   那是「约定」而非「强制」。独立校验器可单独测试，规则不会分叉。
if ! "$PY" "${ROOT}/scripts/validate-gates-manifest.py" "$MANIFEST"; then
  printf '%s❌ manifest 不合规（详见上方）—— 修正后重跑%s\n' "$RED" "$RESET" >&2
  exit 2
fi

# ── 临时产物清扫钩子（devops 2026-10-05 新增）────────────────────────────
# 存在理由：某些 self_test 在本环境**只能**「只增不删」（rm -rf 会被沙箱包装成
# safe-delete 而挂起），于是每次自检都在仓库根留下 .tmp-*/ 残留 —— 实测累积到
# 242 个目录 / 461 个空文件，从不清理。
#
# 🔴 为什么放在 runner 而不是各 self_test 里：
#   ① runner 是**所有** self_test 的唯一公共调用点，一处生效全覆盖；
#   ② 各 self_test 改不动时（越出本次改动范围），runner 仍是最后一道兜底。
#
# 🔴 铁律：只删 **git 明确忽略** 的 .tmp-* 目录。
#   `git check-ignore` 是硬闸门 —— 它判不了的一律留下并报出来，
#   绝不做「看起来像临时物就删」的猜测（那正是误删源码的路径）。
sweep_tmp() {  # $1 = 阶段标签
  local d n=0
  for d in "${ROOT}"/.tmp-*; do
    [[ -d "$d" ]] || continue
    if git -C "$ROOT" check-ignore -q "${d#"${ROOT}"/}" 2>/dev/null; then
      rm -rf -- "$d" || printf '  ⚠️ 清理失败（保留待人工处理）：%s\n' "${d#"${ROOT}"/}"
      n=$((n + 1))
    else
      printf '  ⚠️ 未清理（git 未忽略，性质不明）：%s\n' "${d#"${ROOT}"/}" >&2
    fi
  done
  # ⚠️ printf 的格式串必须是**第一个参数**，占位符与参数必须一一对应。
  #    初版写成 '%s 清理… %d 个' 却传了 4 个参数（YELLOW/RESET/标签/计数），
  #    导致标签漂到下一行、%d 打印出 RESET 变量 —— 肉眼完全看不出是 bug。
  (( n > 0 )) && printf '  ⌫ %s%s 清理自检临时目录 %d 个%s\n' "$YELLOW" "$1" "$n" "$RESET"
  return 0
}
sweep_tmp "（本轮开始前）"

# ── 主流程 ─────────────────────────────────────────────────────────────────
n_pass=0; n_fail=0; n_undet=0; n_retired=0; n_wording=0; n_noself=0; n_selfproved=0
falsify_bad=""

if (( FALSIFY )); then
  printf '%s═══ 闸门反向自检（期望每道都能判红）═══%s\n' "$CYAN" "$RESET"
else
  printf '%s═══ 闸门自检（%d 道）═══%s\n' "$CYAN" "${#ROWS[@]}" "$RESET"
fi

for row in "${ROWS[@]}"; do
  IFS=$'\x1f' read -r id script st fals rules vby rat rrs _pad <<< "$row"
  [[ -n "$id" ]] || continue

  # 退役条目：不参与判定，但必须显示（契约 §9.3）
  if [[ -n "$rat" ]]; then
    n_retired=$((n_retired + 1))
    printf '  %s⊘%s %-6s 已退役（%s）%s\n' "$YELLOW" "$RESET" "$id" "$rat" "$rrs"
    continue
  fi

  [[ -z "$st" ]] && n_noself=$((n_noself + 1))

  # ── 反向模式：验证闸门脚本**本身**能判红 ──
  # ⚠️ 这里**不跑 self_test**，而是直接调闸门脚本的 `--falsify`（契约 §2.1 能力③）。
  #
  # 为什么不能靠 self_test：self_test 返回非0 可能有两种原因 ——
  #   (a) 它真的在验证什么（好）；(b) 它无条件失败（坏，且啥也没验证）。
  # 二者从退出码上不可区分。**只有闸门脚本自己的 --falsify 能区分。**
  if (( FALSIFY )); then
    if [[ "$fals" == "true" ]]; then
      if [[ -n "$script" && -f "${ROOT}/${script}" ]]; then
        TARGET_DIR="$(mktemp -d)"
        out="$(bash "${ROOT}/${script}" "${TARGET_DIR}" --falsify 2>&1)"; rc=$?
        rm -rf "${TARGET_DIR}"
        # 🔴 **决定性判定（主理人 2026-10-04 发现的根因缺陷）**：
        #   仅凭 `--falsify` 返回 1 **不足以**判定闸门有效 ——
        #   「什么都不检查、只在 --falsify 时无条件报红」的闸门同样返回 1。
        #
        # 区分办法：**看输出里有没有 SELF-TEST 且带规则 ID** ——
        #   无条件报红的闸门写不出"SELFTEST"标记，因为它根本不检查。
        # ⚠️ 更强的判据（若闸门实现了 --selftest）：selftest 与 falsify 的结果必须**不同**。
        #    两者相同 → 说明红与输入无关 → 不可自证。
        has_st_marker=0
        printf '%s' "$out" | grep -qE 'SELF-TEST' && has_st_marker=1
        has_rule_id=0
        printf '%s' "$out" | grep -qE 'rule[[:space:]]+[A-Za-z0-9]' && has_rule_id=1

        # 🔴 rc=127 优先判定：脚本不存在 / 路径错 / 解释器缺失。
        #    这**不是"无法判定"**（环境问题），而是**配置错误**，必须硬失败。
        #    依据：devops 2026-10-04 指出——脚本存在但内部 exit 127 的场景也会发生。
        #    ⚠️ 必须放在 has_st_marker 之前，否则会被误报成"假自证（rc=1）"。
        if (( rc == 127 )); then
          printf '  %s❌%s %-6s 退出码 127：脚本不存在/路径错/解释器缺失（配置错误）\n' "$RED" "$RESET" "$id"
          falsify_bad+="[$id] rc=127 "
          n_fail=$((n_fail + 1))
        elif (( rc == 2 )); then
          # 🔴 rc=2 有【两种完全不同】的含义，混在一起就是恒绿陷阱（契约能力④）：
          #   (a) 探针未被检出 → **检测逻辑已失效**，是"闸门坏了"
          #   (b) 环境不支持（如 G26-4 需 POSIX 软链）→ 真正的"无法判定"
          # 区分依据：stderr 是否含 SELF-TEST FAILED（devops 2026-10-04 贡献的三道闸门都打印它）
          if printf '%s' "$out" | grep -qE 'SELF-TEST FAILED'; then
            printf '  %s✗%s %-6s **检测逻辑已失效**：探针未被检出（rc=2 + SELF-TEST FAILED）\n' "$RED" "$RESET" "$id"
            printf '      ⚠️ 这不是"环境不支持"—— 探针是真实违规内容，未命中只有一个解释。\n' >&2
            falsify_bad+="[$id] 检测逻辑失效（探针未检出） "
            n_fail=$((n_fail + 1))
          else
            printf '  %s▲%s %-6s 环境不支持（本环境无法判定）→ 不计入通过\n' "$YELLOW" "$RESET" "$id"
            n_undet=$((n_undet + 1))
          fi
        elif (( rc == 0 )); then
          printf '  %s✗%s %-6s **无法自证**：--falsify 仍返回 0（该闸门在验证什么？）\n' "$RED" "$RESET" "$id"
          falsify_bad+="[$id] --falsify 仍 rc=0 "
          n_fail=$((n_fail + 1))
        elif (( has_st_marker == 0 )); then
          # 返回了 1 但没有 SELF-TEST 标记 → 典型的「无条件报红」假自证
          printf '  %s✗%s %-6s **假自证**：--falsify rc=1 但输出无 SELF-TEST 标记\n' "$RED" "$RESET" "$id"
          printf '      %s
' "$(printf '%s' "$out" | head -1)" >&2
          printf '      ⚠️ 无条件报红的闸门能骗过自证 —— 见契约 §2.1 的反例。\n' >&2
          falsify_bad+="[$id] 假自证（无 SELF-TEST 标记） "
          n_fail=$((n_fail + 1))
        elif (( has_rule_id == 0 )); then
          printf '  %s✗%s %-6s **失败信息不精确**：无 rule ID，runner 无法区分是哪条规则\n' "$RED" "$RESET" "$id"
          falsify_bad+="[$id] 失败信息缺 rule ID "
          n_fail=$((n_fail + 1))
        elif (( rc == 1 )); then
          printf '  %s✓%s %-6s 可自证：--falsify 确实判红（rc=1）\n' "$GREEN" "$RESET" "$id"
          n_pass=$((n_pass + 1)); n_selfproved=$((n_selfproved + 1))
        elif (( rc == 2 )); then
          printf '  %s▲%s %-6s 无法判定（--falsify 需环境支持）→ 不计入通过\n' "$YELLOW" "$RESET" "$id"
          n_undet=$((n_undet + 1))
        else
          printf '  %s❌%s %-6s 契约违规：--falsify rc=%d（只允许 0/1/2）\n' "$RED" "$RESET" "$id" "$rc"
          n_fail=$((n_fail + 1))
        fi
      else
        # 🔴 找不到脚本**不是"无法判定"，是配置错误**。
        #    若判"无法判定"，则"路径写错"的闸门会静默逃过自证。
        printf '  %s❌%s %-6s 闸门脚本不存在：%s（manifest 配置错误）\n' "$RED" "$RESET" "$id" "$script"
        falsify_bad+="[$id] 脚本不存在 "
        n_fail=$((n_fail + 1))
      fi
    else
      printf '  %s▲%s %-6s falsifiable:false（%s）→ 反向不适用\n' "$YELLOW" "$RESET" "$id" "$vby"
      n_undet=$((n_undet + 1))
    fi
    continue
  fi

  # 正常模式：跑 self_test（它内部负责"会红"与"不该红"两侧）
  if [[ -z "$st" || ! -f "${ROOT}/${st}" ]]; then
    printf '  %s▲%s %-6s 无 self_test → **无法判定**，不计入通过\n' "$YELLOW" "$RESET" "$id"
    n_undet=$((n_undet + 1))
    continue
  fi
  out="$(bash "${ROOT}/${st}" 2>&1)"; rc=$?
  state="$(classify $rc)"
  w="$(check_wording "$id" "$rc" "$out")"
  [[ "$w" == "bad" ]] && n_wording=$((n_wording + 1))

  case "$state" in
    pass)          printf '  %s✓%s %-6s 自检全绿（%s）\n' "$GREEN" "$RESET" "$id" "$rules"
                    n_pass=$((n_pass + 1)) ;;
    fail)          printf '  %s✗%s %-6s 自检发现问题\n' "$RED" "$RESET" "$id"
                    printf '%s\n' "$out" | head -6 | sed 's/^/      /' >&2
                    n_fail=$((n_fail + 1)) ;;
    undetermined)  printf '  %s▲%s %-6s **无法判定**（rc=2）→ 不计入通过\n' "$YELLOW" "$RESET" "$id"
                    printf '     ⚠️ 未判定 ≠ 通过。%s\n' "$(printf '%s' "$out" | head -1)" >&2
                    n_undet=$((n_undet + 1)) ;;
    invalid)       printf '  %s❌%s %-6s 契约违规：rc=%d（只允许 0/1/2）\n' "$RED" "$RESET" "$id" "$rc"
                    n_fail=$((n_fail + 1)) ;;
  esac
done

# 🔴 收尾再扫一次：G37 的 self_test 就在上面刚跑完，它留下的残留必须清掉，
#    否则**下一轮**的 G38（工作区卫生）会因上一轮的残渣而误报。
sweep_tmp "（本轮结束后）"

# ── 报告（三类分开计数，契约 §6.3 / 铁律十六）──────────────────────────────
printf '\n'
printf '%s─── 分类计数 ───%s\n' "$RESET" "$RESET"
printf '  通过       %d\n' "$n_pass"
printf '  有问题     %d\n' "$n_fail"
printf '  无法判定   %d   ← 不等于通过\n' "$n_undet"
(( n_retired )) && printf '  已退役     %d   ← 保留记录，不参与判定\n' "$n_retired"
(( n_wording )) && printf '  措辞不合规 %d   ← 退出码 2 但人读会误判（契约 §5.1）\n' "$n_wording"
(( n_noself )) && printf '  无 self_test %d   ← 无法证明"不误报"（契约 §4）\n' "$n_noself"

# ── 判定 ─────────────────────────────────────────────────────────────────
if (( FALSIFY )); then
  if (( n_selfproved == 0 )); then
    # 🔴 关键修正：反向模式下"计数"与"结论"必须一致。
    # 若一道都没真正自证成功（全是 0/1/2 的其他组合），**不能报"通过"** ——
    # 那是"报告通过但什么都没验证"（第 7 类语义错位）。
    printf '%s❌ 反向自检失败：**没有任何闸门被证明能判红**%s\n' "$RED" "$RESET" >&2
    printf '   （问题 %d / 无法判定 %d）\n' "$n_fail" "$n_undet" >&2
    printf '%s   ⚠️ "未能证明"不等于"通过"。闸门需要实现 --falsify（契约 §2.1 能力③）。%s\n' "$YELLOW" "$RESET" >&2
    exit 1
  fi
  if [[ -n "$falsify_bad" ]]; then
    printf '%s❌ 反向自检失败：%s%s\n' "$RED" "$falsify_bad" "$RESET" >&2
    exit 1
  fi
  printf '%s✓ 反向自检通过：%d 道闸门被证明能判红%s\n' "$GREEN" "$n_selfproved" "$RESET"
  if (( n_undet > 0 )); then
    printf '%s⚠️  %d 道在反向模式下无法判定（多为未实现 --falsify）%s\n' "$YELLOW" "$n_undet" "$RESET"
  fi
  exit 0
fi

if (( n_fail > 0 || n_wording > 0 )); then
  printf '%s❌ 闸门自检未通过（问题 %d / 措辞不合规 %d）%s\n' "$RED" "$n_fail" "$n_wording" "$RESET" >&2
  exit 1
fi
if (( n_undet > 0 )); then
  printf '%s⚠️  %d 道闸门在本环境无法判定 —— 不是通过%s\n' "$YELLOW" "$n_undet" "$RESET"
  printf '%s   ↑ 若这些在 CI 目标平台上可判定，那里会真正验证%s\n' "$YELLOW" "$RESET"
fi
printf '%s✓ %d 道闸门自证有效%s\n' "$GREEN" "$n_pass" "$RESET"
exit 0

# =============================================================================
# 边界：runner 的职责是「共享机制」，不是「同构实现」
# -----------------------------------------------------------------------------
# runner **不**假设：所有闸门都是 grep 扫描、都接受同一种参数、都用同一种输出格式。
# runner **只**提供：三态分类 / 措辞校验 / 反向自检 / 四类分开计数 / 退役留痕。
#
# 闸门特有的测试逻辑（如 HTTP 行为、产物内容）**全部在它自己的 self_test 里**。
# 新增闸门时不需要改 runner —— 只需在 manifest 声明 + 写自己的 self_test。
# =============================================================================
