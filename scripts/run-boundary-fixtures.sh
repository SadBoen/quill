#!/usr/bin/env bash
# =============================================================================
# G32 / G08：边界闸门 fixture 自检 runner
# =============================================================================
# 依据：docs/08_测试与验收方案.md §5.2.1（G08 + G32）
# 消费：crates/quill-testkit/fixtures/boundary/manifest.json
#
# ── 这个 runner 本身是什么 ────────────────────────────────────────────────
# 它验证的是**闸门本身是否有效**，不是验证代码是否符合边界。
#   · BND-01~06 违规 fixture → 期望检查器**变红**
#   · BND-00 正向 fixture   → 期望检查器**不红**
#
#   只测违规不测正向 = 闸门会变成"一律禁止" → 逼后人绕过它（加白名单/关检查）
#   只测正向不测违规 = 闸门是假闸门         → 提供虚假的安全感
#   **两者必须同时存在，才构成完整自检。**
#
# ── runner 自身的自检（关键）────────────────────────────────────────────────
# 本 runner 也是 AI 写的。若它恒返回"通过"，则 7 个 fixture 等于不存在。
# 因此 falsify 模式：把某个 fixture 标成"期望相反"，runner 必须报失败。
# 用法：bash scripts/test-boundary-fixtures.sh --falsify
# =============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_DIR="${ROOT}/crates/quill-testkit/fixtures/boundary"
MANIFEST="${FIXTURE_DIR}/manifest.json"

RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'

# 可证伪模式：期望 runner 自身失败
FALSIFY=0
[[ "${1:-}" == "--falsify" ]] && FALSIFY=1

# ⚠️ **检查器覆盖范围**（踩过的坑，勿删这行注释）
# ---------------------------------------------------------------------------
# `check-boundary-singletons.sh` **只实现 G26 两条规则**（原编号规则 7~8）：
#   G26-1  禁 Config::global()
#   G26-2  禁 SessionManager::instance()
#
# **它不检查六规则**（路径重复 / 偏离未登记 / 依赖浮动 / 无 remote /
# 测试不全绿 / todo 残留）—— 那六条目前**尚无检查器实现**。
#
# 所以 BND-01~06 中，只有「违反 G26 的 fixture」能被现有检查器判定。
# 其余 fixture 当前会报 `no-checker`（跳过并计入「未覆盖」，**不是通过**）。
#
# ⚠️ **绝不能把"未覆盖"当"通过"** —— 那会让 G08 在六规则完全没被检查的情况下
# 显示全绿。这正是第 1 类假闸门（校验在，永不生效）。
# 待 `cargo xtask boundary` 实现六规则后，把 COVERS=0 改为 1 即可。

# ── 输出格式规范（G08 对检查器的约定）─────────────────────────────────────
# ⚠️ 检查器**必须**在 stderr 输出 `rule N` 字样（N 为规则编号），
#    否则 runner 无法断言"失败信息精确指向规则编号"——
#    一个 fixture 因别的原因失败也会被判为"成功拦住"，那就是假闸门。
# 示例契约（详见本文件末尾「输出格式规范」小节）：
#   ❌ 违反边界规则 G26：禁止 Config::global()
#   rule 26
#   命中位置：
#       crates/quill-x/src/lib.rs:42
RULE_TAG_RE='rule[[:space:]]+([0-9]+|G[0-9]+|R[0-9]+-[0-9]+|BND-[0-9]+)'

# ── 依赖检查：缺工具必须**失败**而非"跳过通过" ──────────────────────────────
missing=0
for tool in python3 git; do
  command -v "$tool" >/dev/null 2>&1 || { printf '%s✗ 缺工具：%s（fixture 自检依赖它）%s\n' "$RED" "$tool" "$RESET" >&2; missing=1; }
done
((missing)) && { printf '%s❌ 依赖缺失，fixture 自检无法执行 —— 不是"通过"而是"无法判定"%s\n' "$RED" "$RESET" >&2; exit 2; }
[[ -f "$MANIFEST" ]] || { printf '%s❌ 缺 manifest：%s%s\n' "$RED" "$MANIFEST" "$RESET" >&2; exit 2; }

# ── 检查器：优先用 xtask，退化到脚本 ────────────────────────────────────────
# ⚠️ 关键：找不到检查器时**必须失败**。若此处"跳过通过"，
#    G08 会在没有任何检查器的情况下恒绿 —— 那是第 1 类假闸门。
find_checker() {
  if [[ -x "${ROOT}/target/debug/xtask" ]] && "${ROOT}/target/debug/xtask" boundary --help >/dev/null 2>&1; then
    echo "xtask"; return 0
  fi
  if [[ -x "${ROOT}/scripts/check-boundary-singletons.sh" ]]; then
    echo "singletons"; return 0
  fi
  return 1
}
CHECKER="$(find_checker)" || {
  printf '%s❌ 找不到边界检查器（xtask 或 check-boundary-singletons.sh）%s\n' "$RED" "$RESET" >&2
  printf '   %s\n' "⚠️ 这里必须失败而不是跳过 —— 否则 G08 恒绿而无人察觉。" >&2
  exit 2
}

run_checker() {  # $1=fixture 根目录；输出到 $OUT，退出码到 $RC
  local dir="$1" out rc
  if [[ "$CHECKER" == "xtask" ]]; then
    out="$("${ROOT}/target/debug/xtask" boundary "${dir}/crates" 2>&1)"; rc=$?
  else
    out="$(bash "${ROOT}/scripts/check-boundary-singletons.sh" "${dir}/crates" 2>&1)"; rc=$?
  fi
  printf '%s' "$out" > "$TMP_OUT"
  RC=$rc
}

# ── 主流程 ────────────────────────────────────────────────────────────────
PY="${PYTHON:-python3}"
TMP_DIR="$(mktemp -d)"; trap 'rm -rf "$TMP_DIR"' EXIT
TMP_OUT="${TMP_DIR}/out.txt"

total=0; pass=0; fail=0; uncovered=0; by_design=0; unsupported=0; symlink_ok=1; fail_details=""

# 读 manifest（python3 只用来提取字段，不做判断逻辑）
mapfile -t FIXTURES < <(python3 - "$MANIFEST" <<'PY'
import json, sys
d = json.load(open(sys.argv[1], encoding="utf-8"))
for f in d.get("fixtures", []):
    req = "git" if "git" in str(f.get("requires", "")).lower() else "-"
    print("\t".join([f["id"], f["dir"], "pass" if f.get("must_pass") else "fail", req]))
PY
)

(( ${#FIXTURES[@]} > 0 )) || { printf '%s❌ manifest 里没有任何 fixture%s\n' "$RED" "$RESET" >&2; exit 2; }

printf '%s═══ 边界闸门 fixture 自检（%d 个，检查器：%s）═══%s\n' "$RESET" "${#FIXTURES[@]}" "$CHECKER" "$RESET"

for line in "${FIXTURES[@]}"; do
  IFS=$'\t' read -r id fdir expect req <<< "$line"
  total=$((total + 1))
  src="${FIXTURE_DIR}/${fdir}"
  work="${TMP_DIR}/${fdir}"

  if [[ ! -d "$src" ]]; then
    fail=$((fail + 1)); fail_details+="${RED}  [${id}] fixture 目录不存在：${src}${RESET}"$'\n'; continue
  fi

  # 该 fixture 违反的规则是否落在现有检查器的覆盖范围内？
  # `covered` 来自 manifest 的 covered_by 字段；缺省视为"六规则未实现 → 未覆盖"
  covered="$("$PY" -c "import json,sys;d=json.load(open(sys.argv[1],encoding='utf-8'));print(next((f.get('covered_by','G26') for f in d['fixtures'] if f['id']==sys.argv[2]),'G26'))" "$MANIFEST" "$id")"
  if [[ "$covered" == "none" ]] && [[ "$CHECKER" != "xtask" ]]; then
    uncovered=$((uncovered + 1))
    printf '  %s○%s %-8s 检查器尚未实现 → **未覆盖**（不是通过）\n' "$YELLOW" "$RESET" "$id"
    continue
  fi
  # 🔴 设计性不覆盖：**永远不该变绿**，且必须永远可见。
  # 这类 fixture 存在的意义是"诚实登记盲区"，不是"等闸门去拦"。
  if [[ "$covered" == "none-by-design" ]]; then
    by_design=$((by_design + 1))
    printf '  %s⊘%s %-8s **设计性不覆盖**（方法论上限，勿试图补）\n' "$YELLOW" "$RESET" "$id"
    continue
  fi
  cp -r "$src" "$work"

  # 符号链接类 fixture：链接**不预先提交**（Windows 需管理员/开发者模式），
  # 由 runner 在临时副本上创建。若本平台不支持 → 计入"未覆盖"并标黄，**不静默跳过**。
  if [[ "$id" == "BND-09" ]]; then
    mkdir -p "$work/crates"
    ln -s ../real_src "$work/crates/quill-linked" 2>/dev/null || true
    cmd //c mklink /D "$(cygpath -w "$work/crates/quill-linked")" "$(cygpath -w "$work/real_src")" >/dev/null 2>&1 || true
    # 🔴 **必须验证"真的是符号链接"**，不能只看命令退出码。
    # Windows Git Bash 下 `ln -s` 常常**创建一个真实的目录副本**并返回 0 ——
    # 那时 G26-4 正确地不判红（它确实是普通目录），但**测试目的已落空**
    # （我们本想测"链接指向 crates/ 外"，实际没建出链接）。
    # 这与 G33 防的"静默跳过"同源：命令"成功"不等于目的达成。
    if [[ -L "$work/crates/quill-linked" ]] || cmd //c "dir /AL" 2>/dev/null | grep -q "quill-linked"; then
      symlink_ok=1
    else
      symlink_ok=0
    fi
  fi

  # 🔴 本平台建不出真符号链接 → **不跑检查器**，直接计入"无法判定"。
  # 理由：跑它会得到"通过"，但那是"目录副本被判定为合法"，**与测试目的无关** ——
  # 属于 G33 防的"语义错位"（报告通过而实际没测到该风险）。
  if [[ "${symlink_ok:-1}" -eq 0 ]]; then
    unsupported=$((unsupported + 1))
    printf '  %s▲%s %-8s 本平台无法创建符号链接（Windows 需开发者模式）→ **无法判定**，不计入通过\n' "$YELLOW" "$RESET" "$id"
    printf '     %s↑ CI 目标 ubuntu-latest 支持软链，那里会真正判定%s\n' "$YELLOW" "$RESET"
    continue
  fi

  # 规则 1/2/4 需要真实 git 仓库才能判定（diff / remote）
  if [[ "$req" == "git" || "$expect" != "pass" ]]; then
    ( cd "$work" && git init -q 2>/dev/null && git add -A 2>/dev/null && \
      git -c user.email=f@x -c user.name=fixture commit -qm fixture 2>/dev/null ) || true
  fi

  run_checker "$work"
  out="$(cat "$TMP_OUT")"

  # 🔴 环境错误 ≠ 判定结果
  # 检查器退出码 2 = 用法/环境错误（目录不存在、缺工具）。
  # 那**不是"通过"也不是"违规"**，而是"无法判定" —— 必须单列，
  # 否则会像 G33 之前那样把"没跑成"显示成"通过"（第 7 类：语义错位）。
  if [[ $RC -eq 2 ]]; then
    if [[ "${symlink_ok:-1}" -eq 0 ]]; then
      unsupported=$((unsupported + 1))
      printf '  %s▲%s %-8s 本平台不支持（软链）→ **无法判定**，计入未覆盖\n' "$YELLOW" "$RESET" "$id"
    else
      fail=$((fail + 1))
      fail_details+="${RED}  [${id}] 检查器退出码 2（环境/用法错误），非判定结果${RESET}"$'\n'
      fail_details+="      输出：$(printf '%s' "$out" | head -2 | tr '\n' '|' | cut -c1-120)"$'\n'
    fi
    continue
  fi
  symlink_ok=1
  actual="pass"; [[ $RC -ne 0 ]] && actual="fail"

  # 可证伪模式：把期望反过来，runner 必须报失败
  if (( FALSIFY )); then
    if [[ "$expect" == "pass" ]]; then expect="fail"; else expect="pass"; fi
  fi

  if [[ "$actual" == "$expect" ]]; then
    pass=$((pass + 1))
    printf '  %s✓%s %-8s 期望=%-4s 实测=%-4s\n' "$GREEN" "$RESET" "$id" "$expect" "$actual"
  else
    fail=$((fail + 1))
    fail_details+="${RED}  [${id}] 期望=${expect} 实测=${actual}${RESET}"$'\n'
    fail_details+="      输出：$(printf '%s' "$out" | head -3 | tr '\n' '|' | cut -c1-140)"$'\n'
  fi

  # 违规用例额外断言：失败信息必须精确指向规则编号
  # （不检查这条，fixture 因别的原因失败也会被判为"成功拦住"）
  if [[ "$actual" == "fail" && "$expect" == "fail" ]]; then
    if ! printf '%s' "$out" | grep -qE "$RULE_TAG_RE"; then
      fail=$((fail + 1))
      fail_details+="${RED}  [${id}] 失败信息**未精确指向规则编号**（缺 'rule N' 字样）${RESET}"$'\n'
      fail_details+="      → 这样的 fixture 无法区分'闸门拦住了'与'它因别的原因失败了'"$'\n'
    fi
  fi
done

printf '\n'
if (( fail == 0 )); then
  printf '%s✓ %d/%d 已覆盖 fixture 行为符合预期%s\n' "$GREEN" "$pass" "$((total - uncovered))" "$RESET"
  if (( uncovered > 0 )); then
    printf '%s⚠️  %d 个 fixture **未覆盖**（检查器尚未实现，补上即转绿）%s\n' "$YELLOW" "$uncovered" "$RESET"
  fi
  if (( by_design > 0 )); then
    printf '%s⊘  %d 个 fixture **设计性不覆盖**（方法论上限，永远不会转绿，且必须保持可见）%s\n' "$YELLOW" "$by_design" "$RESET"
    printf '%s   ⚠️ 这一类是"诚实登记的盲区"。**不要试图补检查器，也放宽匹配规则来让它变绿**%s\n' "$YELLOW" "$RESET"
    printf '%s   —— 那会引入真正的漏报。详见 manifest 的 _covered_by 三态说明。%s\n' "$YELLOW" "$RESET"
  fi
  if (( uncovered > 0 || by_design > 0 )); then
    printf '%s   ⚠️ 未覆盖 ≠ 通过。两者都不该被解读为"检查通过"。%s\n' "$YELLOW" "$RESET"
  fi
  # 反向自检：若 falsify 模式下也全绿，说明 runner 恒返回"通过" —— 它本身是假闸门
  if (( FALSIFY )); then
    printf '%s❌ 反向自检失败：falsify 模式下仍全部通过 → runner 本身是假闸门%s\n' "$RED" "$RESET" >&2
    exit 1
  fi
  exit 0
else
  printf '%s❌ %d/%d fixture 不符合预期：%s\n' "$RED" "$fail" "$total" "$RESET" >&2
  printf '%s%s%s\n' "$fail_details" "$RESET" >&2
  if (( FALSIFY )); then
    printf '%s✓ 反向自检通过：falsify 模式下确实报失败 → runner 有鉴别力%s\n' "$GREEN" "$RESET"
    exit 0
  fi
  exit 1
fi

# =============================================================================
# 输出格式规范（检查器实现方必读）
# =============================================================================
# runner 的第 6 步会断言违规用例的输出含 `rule N`。若你的检查器不输出它，
# BND-01~06 会全部被判为"失败信息不精确"。
#
# 要求的输出形态（stderr）：
#   ❌ 违反边界规则 G26：禁止 Config::global()
#      rule 26
#      命中位置：
#          crates/quill-x/src/lib.rs:42
#          crates/quill-y/src/main.rs:17
#
# 为什么必须有 rule 编号：
#   一个 fixture 只违反一条规则。若检查器因**别的原因**失败（缺工具、路径不存在、
#   文件编码错误），测试仍会看到"它红了"—— 但那不是"闸门拦住了违规"。
#   **规则编号是区分这两者的唯一依据。**
#
# 正向用例（must_pass=true）的输出要求：
#   ✓ 无违规（G26 等 2 条规则）
#   退出码必须**恰为 0**（不是"非 1"）—— 检查器崩溃时退出码可能是 2，
#   只判"非 1"会把崩溃误判为通过。
# =============================================================================
