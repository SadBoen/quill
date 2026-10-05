#!/usr/bin/env bash
# ==============================================================================
# check-doc-refs.sh —— G39：文档引用真实性闸门
#
# 🔴 这道闸门存在的理由（2026-10-05 QA 实测）：
#   今天挖出的**所有**「闸门失效 / 文档说谎」类故障，根因都是
#   **文档里引用了不存在的路径或命令**，而没有任何一层检查这件事。
#   实测到的假引用（不是猜测，是 test -e / 实际执行确认）：
#     · 引用 `scripts/check-todo-markers.sh` —— 文件不存在
#     · 引用 `deploy/quill.service`          —— 文件不存在
#     · 声称可跑 `cargo xtask boundary`       —— error: no such command
#     · 引用 `QUILT-PATCHES.md` / `patches/`  —— 均不存在
#   这些假引用让人以为"已经做了"，而实际什么都没做 —— 属 AGENTS.md 八类失效里的
#   第 3 类（静默失败）与第 6 类（语义错位）。
#
# 规则拆分：
#   G39-a  仓库内相对路径引用必须真实存在
#   G39-b  cargo 的**子命令**必须真实存在
#
# 明确不管的（避免误报闸门 —— 误报比假闸门更常见、更容易被学会忽略）：
#   · 文档内章节引用（`见铁律十四`）          —— 不是路径
#   · 标识符 / 类型名 / 常量（`UserId`）      —— 不是路径
#   · 代码围栏内（``` 包裹）的反引号内容      —— 那是演示用的
#   · 通配符示意路径（`crates/*`、`src/*.rs`）
#   · 占位符（`06_xxx.md`、`<user>`、`$PWD`、`..`、绝对路径、`~/.cargo`）
#   · 构建产物路径（`target/`、`ui/web/dist/`）—— 是否存在取决于是否构建过，
#     不判定，但**必须显式计数输出**（0 与未检查必须可区分）
#   · 除 cargo 子命令外的外部命令（`docker`、`wsl`、`sudo`…）—— 是否安装取决于
#     环境，不是文档缺陷；同样显式计数输出
#
# 三态契约：
#   0 = 通过 / 1 = 检出违规 / 2 = 无法判定（措辞必含 ▲，绝不计入通过）
#   违规输出必含 `rule G39`。
#
# 用法：
#   bash scripts/check-doc-refs.sh                 # 查 AGENTS.md
#   bash scripts/check-doc-refs.sh --docs          # 加查 docs/*.md
#   bash scripts/check-doc-refs.sh a.md b.md       # 显式指定文件
#   bash scripts/check-doc-refs.sh --falsify       # 自证（注入已知坏输入）
# ==============================================================================
set -uo pipefail

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; DIM=$'\033[2m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; DIM=''; RESET=''
fi

GATE='G39'
FALSIFY=0
SCAN_DOCS=0
declare -a TARGETS=()
for a in "$@"; do
  case "$a" in
    --falsify) FALSIFY=1 ;;
    --docs)    SCAN_DOCS=1 ;;
    -h|--help) sed -n '2,38p' "$0"; exit 0 ;;
    -*)        printf '%s\n' "${YELLOW}▲ ${GATE} 无法判定：未知参数 $a${RESET}" >&2
               printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
               exit 2 ;;
    *)         TARGETS+=("$a") ;;
  esac
done

# --- 依赖检查（缺工具必须 rc=2；禁止 || true 兜底）--------------------------
for _t in awk find grep mktemp; do
  if ! command -v "$_t" >/dev/null 2>&1; then
    printf '%s\n' "${YELLOW}▲ ${GATE} 无法判定：缺少必需工具 ${_t}${RESET}" >&2
    printf '%s\n' "  ⚠️ 缺工具时若判绿，本闸门就退化成『什么都不检查的恒绿闸门』。" >&2
    printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
    exit 2
  fi
done

# --- cargo 探测（本仓库常态：cargo 只在 ~/.cargo/bin，不在默认 PATH）--------
CARGO_BIN=""
discover_cargo() {
  local c
  if command -v cargo >/dev/null 2>&1; then CARGO_BIN=$(command -v cargo); return 0; fi
  for c in "${CARGO_HOME:-}/bin/cargo" "$HOME/.cargo/bin/cargo" /usr/local/cargo/bin/cargo; do
    if [[ -n "$c" && -x "$c" ]]; then CARGO_BIN="$c"; return 0; fi
  done
  return 1
}

# cargo 内置子命令：直接放行（探测它们没有鉴别力，只会变慢）
readonly CARGO_BUILTIN=" add bench build check clean clippy doc fetch fix fmt generate-lockfile help init install locate-project login logout metadata new owner package pkgid publish read-manifest remove report run rustc rustdoc search test tree uninstall update upgrade vendor verify-project version yank "

probe_cargo_sub() {
  local sub="$1"
  if command -v timeout >/dev/null 2>&1; then
    timeout 30 "$CARGO_BIN" "$sub" --help >/dev/null 2>&1
  else
    "$CARGO_BIN" "$sub" --help >/dev/null 2>&1
  fi
}

# --- 已知命令表（首词命中才按"命令"处理，否则按路径候选处理）---------------
readonly KNOWN_CMDS=" cargo bash sh python3 python node docker git wsl sudo systemctl systemd-analyze curl wget make quill goose grep find diff ls cat chmod stat ln mkdir rm cp mv printf test mktemp readlink sort sed awk apt-get npm npx rustc rustup cargo-nextest "

# --- 构建产物前缀（存在与否取决于是否构建过 → 不判定，但显式计数）----------
is_build_output() {
  case "$1" in
    target|target/*|*/target|*/target/*|node_modules|node_modules/*|*/node_modules/*) return 0 ;;
    ui/web/dist|ui/web/dist/*|dist|dist/*) return 0 ;;
    *.db|*.db-wal|*.db-shm) return 0 ;;
  esac
  return 1
}

# --- 路径形态判定（宁可放过，不可误报）------------------------------------
# 裸文件名（无 '/'）的扩展名白名单。
# 🔴 为什么必须有白名单：`schema.org` 长得像"文件名"，实为域名；
#   若按"有扩展名就是文件"判，它会永远判红 —— 而**一直红的门槛会被学会忽略**。
readonly FILE_EXTS=" md sh json toml rs py yml yaml env lock txt service sql html css js ts tsx example conf cfg ini xml gradle mod "

looks_like_path() {
  local t="$1" stem ext
  [[ -n "$t" ]] || return 1
  # 绝对路径 / home 相对 / 显式相对 / URL
  case "$t" in
    /*|~*|./*|../*|..|*'://'*) return 1 ;;
  esac
  # 通配符（crates/*、src/*.rs 这类示意路径，不判红）
  case "$t" in
    *'*'*|*'?'*) return 1 ;;
  esac
  # 含 shell / 示例语法的字符 → 不是路径（引号、重定向、变量、括号、管道…）
  local forb=$'\'"`<>|&;$(){}=!#@\\ \t' i
  for (( i = 0; i < ${#forb}; i++ )); do
    [[ "$t" == *"${forb:i:1}"* ]] && return 1
  done
  # 占位符：只排除**整词**是占位符的情形。
  # 🔴 不能用 `*todo*` 这类包含匹配 —— `scripts/check-todo-markers.sh` 是真实
  #   文件名（本仓库确实缺它），用包含匹配会把它静默放过 = 漏报。
  #   漏报比误报更危险：误报会被发现，漏报会一直骗人。
  local low="${t,,}" stem0
  stem0="${low%.*}"
  case "$stem0" in
    todo|name|path|file|dir|example|placeholder|foo|bar|baz) return 1 ;;
  esac
  case "$low" in
    *xxx*|*yyy*|*zzz*) return 1 ;;               # 三个及以上重复字母才算示意
  esac
  # 版本/区间/三态记法（`0/1/2`、`2/3/4`）不是路径
  case "$t" in
    [0-9]*/[0-9]*) [[ "$t" =~ ^[0-9]+(/[0-9]+)+$ ]] && return 1 ;;
  esac
  # CIDR 网段记法（`127.0.0.1/8`、`169.254.0.0/16`、`10/8`）—— **零漏报**的一条：
  #   网段记法在任何文件系统里都不可能是仓库内路径，因此排除它不会放过任何真幻觉。
  #   （排除「枚举并列」如 `text/markdown`、`a/b/c` 就不行 —— 那会真的放过
  #     `infra/bridge/` 这类幻觉路径，属于用消除误报换鉴别力，本闸门拒绝这么做。）
  if [[ "$t" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){0,3}/[0-9]{1,2}$ ]]; then return 1; fi
  # 裸域名（无 scheme）：gist.github.com/x、schema.org、example.com/a
  #   —— 首段像主机名（含点且 TLD 段是纯字母）且不是仓库内目录 → 不是仓库路径
  case "$t" in
    */*)
      local first="${t%%/*}" last1
      last1="${first##*.}"
      if [[ "$first" == *.* && "$last1" != "$first" && "$last1" =~ ^[A-Za-z]{2,6}$ ]]; then
        return 1
      fi
      ;;
  esac
  case "$t" in
    */*) : ;;                                    # 含 '/'：目录或文件路径
    *)
      [[ "$t" != .* ]] || return 1
      [[ "$t" == *.* ]] || return 1             # 裸名必须带扩展名
      stem="${t%.*}"; ext="${t##*.}"
      [[ -n "$stem" && -n "$ext" ]] || return 1
      case "${ext,,}" in
        $FILE_EXTS) : ;;                        # 已知文件扩展名
        *) return 1 ;;                          # 未知扩展名 → 多半是域名/标识符
      esac
      ;;
  esac
  return 0
}

# --- 豁免机制（🔴 治理铁律：每个豁免必须显式、有理由、且被计数输出）----------
#   文档里确实需要引用"不存在的对象"时（例如把一个幻觉引用当反例写进正文），
#   在该 md 中写一行：
#       <!-- g39:waive token=<反引号内的原文> reason=<为什么豁免> -->
#   豁免不会静默：命中的条目会单独列出（数量 + token + 理由），
#   自检里另有一条反向断言 —— 豁免之外���违规代码仍须判红。
declare -a _WAIVE_TOK=()
declare -a _WAIVE_WHY=()
declare -a _WAIVED=()
# 解析 <!-- g39:waive token=<原文> reason=<理由> -->
# 🔴 token 可以**含空格**（如 `cargo xtask boundary`），
#    所以不能按"非空格"截取 —— 那样只会拿到 `cargo`，豁免静默失效。
#    边界取 reason= / --> / 行尾 三者中最先出现的那个。
load_waivers() {
  local f="$1" line rest tok why
  while IFS= read -r line; do
    [[ "$line" == *'g39:waive'* ]] || continue
    rest="${line#*token=}"
    case "$rest" in
      *" reason="*) tok="${rest%% reason=*}"; why="${rest#* reason=}" ;;
      *'-->'*)      tok="${rest%%-->*}";   why="（未写 reason）" ;;
      *)            tok="$rest";           why="（未写 reason）" ;;
    esac
    # 去掉 HTML 注释收尾与首尾空白
    why="${why%%-->*}"; why="${why%"${why##*[![:space:]]}"}"
    tok="${tok%"${tok##*[![:space:]]}"}"
    [[ -n "$tok" ]] || continue
    _WAIVE_TOK+=("$tok"); _WAIVE_WHY+=("${why:-（未写理由）}")
  done < "$f"
}
is_waived() {
  local t="$1" i
  for (( i = 0; i < ${#_WAIVE_TOK[@]}; i++ )); do
    [[ "${_WAIVE_TOK[i]}" == "$t" ]] && return 0
  done
  return 1
}
waive_reason() {
  local t="$1" i
  for (( i = 0; i < ${#_WAIVE_TOK[@]}; i++ )); do
    [[ "${_WAIVE_TOK[i]}" == "$t" ]] && { printf '%s' "${_WAIVE_WHY[i]}"; return 0; }
  done
  return 1
}
# 记录一次豁免命中（token 与理由成对存，避免按下标错位取理由）
record_waiver() {
  local t="$1" why
  why="$(waive_reason "$t")"
  _g_waived=$(( _g_waived + 1 ))
  _WAIVED+=("$t —— ${why:-（未写理由）}")
}

# --- 统计与违规清单（函数间共享，故用全局）---------------------------------
_g_viol=0           # 是否检出违规（authoritative 档，0/1 标志）
_g_vio_n=0          # authoritative 档违规**条数**（布尔标志不能当计数用）
_g_undet=0          # 是否存在无法判定项
_g_path_ok=0        # 已检查且真实存在的路径数
_g_gitref_ok=0      # 已用 git 核实存在的远端引用数（独立计数：不与路径混算，
                    # 「0 条」与「没检查过」在屏幕上必须能区分）
_g_resolved=0       # 根相对不存在、但仓库内以别处可达的路径数
_g_skip_build=0     # 构建产物跳过数
_g_skip_ext=0       # 外部命令未验证数
_g_waived=0         # 显式豁免命中数
_g_ref=0            # reference 档命中数（计数并报告，不参与 rc）
_g_arch=0           # archived 档命中数（不判定）
_g_tok_all=0        # 反引号 token 总数
_g_tok_path=0       # 路径候选数
_g_tok_cmd=0        # 命令候选数
declare -a _VIOL=()
declare -a _RESOLVED=()

# --- 文档档位（2026-10-05 新增）---------------------------------------------
# 🔴 为什么分档：全量扫描 docs/ 的实测结果是 **22/23 份 rc=1、203 处原始命中**。
#   其中绝大部分出自**上一团队的设计详稿快照**（reference），它们记录的是"当时规划的
#   文件/命令"，其中相当一部分从未落地。若把 15 份详稿与 6 份权威文档一视同仁，
#   本闸门会永远红 —— 而**一直红的门槛会被学会忽略**，比没有门槛更糟
#   （这正是 AGENTS.md「误报闸门与假闸门同等有害」）。
#   因此按「结论写在哪」分档：
#     authoritative —— 结论写在这里，**必须 rc=0**，红了就是缺陷
#     reference     —— 设计详稿快照，**冻结**：计数并显眼报告，但不参与 rc
#     archived      —— 已作废留档，**不判定**：只报数量
#   代价（必须说清）：reference 档新长出的假引用不会再让 CI 红。
#   缓解：这些文档**不是实现依据**，实现只看权威链；且 reference 档的计数每次都显眼输出。
doc_tier() {
  local t
  t="$(grep -oP '<!--\s*doc-status:\s*\K[a-z]+' "$1" 2>/dev/null | head -1)"
  # 🔴 没写状态位 = 默认 authoritative（**缺声明从严**，不默认放过）
  printf '%s' "${t:-authoritative}"
}
add_viol() { # rule file line token why  (记录档位，供报告分档显示)
  local t; t="$(doc_tier "$ROOT/$2")"
  case "$t" in
    archived)  _g_arch=$(( _g_arch + 1 )) ;;
    reference) _g_ref=$(( _g_ref + 1 ))   ;;
    *)         _g_viol=1; _g_vio_n=$(( _g_vio_n + 1 )) ;;
  esac
  _VIOL+=("$1|$2|$3|$4|$5|$t")
}

add_undet() {
  _g_undet=1
  printf '%s\n' "${YELLOW}▲ ${GATE} 无法判定：$1${RESET}" >&2
  printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
}

# 仓库内全部路径清单（用于「根相对不存在但在仓库内可达」的兜底判定）
_PATHS=""
_collect_paths() {
  local root="$1"
  [[ -n "$_PATHS" ]] && return 0
  _PATHS=$(mktemp)
  find "$root" \( -name .git -o -name target -o -name node_modules \
                -o -name .scratch -o -name _scratch \) -prune -o -print \
      >"$_PATHS" 2>/dev/null
}

_reset_counters() {
  _g_viol=0; _g_vio_n=0; _g_undet=0; _g_path_ok=0; _g_gitref_ok=0; _g_resolved=0
  _g_skip_build=0; _g_skip_ext=0; _g_waived=0
  _g_ref=0; _g_arch=0
  _g_tok_all=0; _g_tok_path=0; _g_tok_cmd=0
  _VIOL=(); _RESOLVED=(); _WAIVED=(); _WAIVE_TOK=(); _WAIVE_WHY=(); _PATHS=""
}

# --- 反引号 token 提取（🔴 必须跳过代码围栏）------------------------------
extract_tokens() {
  awk '
    BEGIN { infence = 0 }
    {
      if ($0 ~ /^[ \t]*(```|~~~)/) { infence = !infence; next }
      if (infence) next
      rest = $0
      while (match(rest, /`[^`]+`/)) {
        tok = substr(rest, RSTART + 1, RLENGTH - 2)
        gsub(/\r/, "", tok)
        gsub(/^[ \t]+|[ \t]+$/, "", tok)
        if (tok != "") printf "%d\t%s\n", NR, tok
        rest = substr(rest, RSTART + RLENGTH)
      }
    }
  ' "$1"
}

# --- 单个路径引用的判定（唯一真相源：falsify 与正常模式共用）----------------
judge_path() {
  local root="$1" rel="$2" ref="$3" line="$4"
  local t stem suffix

  t="$rel"
  # 行号后缀：foo.rs:12 / foo.md:84-87
  if [[ "$t" == *:* ]]; then
    stem="${t%%:*}"; suffix="${t#"$stem"}"
    if [[ "$suffix" =~ ^:[0-9]+(-[0-9]+)?$ ]]; then
      t="$stem"
    else
      return 0
    fi
  fi
  # 尾斜杠：`crates/` 与 `crates` 是同一个东西，但 find 输出不带尾斜杠，
  #   不剥掉会让后缀匹配（见下）必然失败 → 把真存在的目录误判成不存在。
  while [[ "$t" == */ && ${#t} -gt 1 ]]; do t="${t%/}"; done

  is_build_output "$t" && { _g_skip_build=$(( _g_skip_build + 1 )); return 0; }

  # ── git 远端引用（`origin/main`）────────────────────────────────────
  # 🔴 2026-10-05 修：通配符判断**必须**在本段之前。
  #   否则 `origin/*`（"这一整族引用"）会匹配 ^origin/[^/]+$ 被当成
  #   一个具体 ref 去 verify，然后判红 —— 与既有"含通配符不是可执行引用"
  #   规则直接冲突。教训：新增的分类分支要检查它**插在哪一段**，
  #   插错位置会把上游已解决的误报重新引入（顺序也是语义的一部分）。
  case "$t" in
    *'*'*|*'?'*) _g_skip_ext=$(( _g_skip_ext + 1 )); return 0 ;;
  esac

  # 🔴 为什么**验证**而不是排除：把它加进排除表就是"用消除误报换鉴别力"，
  #   而 git 恰恰**能**真的回答它存不存在 —— 于是这一类从"误报"升级为
  #   "多一类真引用"：文档里若写出幻觉的 `origin/nosuch`，本闸门会判红。
  # 零漏报论证：`origin/` 是**封闭命名空间**（git 远端名只占一层），
  #   `origin/a/b` 这种多层的仍走路径判定，故不会放过 `infra/bridge/` 类幻觉。
  if [[ "$t" =~ ^origin/[^/]+$ ]]; then
    if ! git rev-parse --git-dir >/dev/null 2>&1; then
      # 缺工具必须「无法判定」，不得兜底成通过，也不得当成"引用不存在"
      add_undet "当前不在 git 工作树内，无法验证远端引用：$t"
      return 0
    fi
    if git rev-parse --verify --quiet "refs/remotes/$t" >/dev/null 2>&1; then
      _g_gitref_ok=$(( _g_gitref_ok + 1 )); return 0
    fi
    add_viol 'G39-a' "$ref" "$line" "$rel" \
      "git 远端引用不存在：$t（核实命令：git rev-parse --verify refs/remotes/$t）"
    return 0
  fi

  looks_like_path "$t" || return 0

  if [[ -e "$root/$t" ]]; then
    _g_path_ok=$(( _g_path_ok + 1 )); return 0
  fi

  # 根相对不存在 → 仓库内任意位置以同后缀可达（vendor 引用、docs/ 下的裸文件名）
  # → 不判红，但显式列出（0 与未检查必须可区分）
  if grep -q -- "/$t\$" "$_PATHS" 2>/dev/null; then
    _g_resolved=$(( _g_resolved + 1 )); _RESOLVED+=("$t"); return 0
  fi

  # 显式豁免（必须在最后：豁免只压"不存在"这一条判定，不影响其他检查）
  if is_waived "$rel" || is_waived "$t"; then
    if is_waived "$rel"; then record_waiver "$rel"; else record_waiver "$t"; fi
    return 0
  fi

  add_viol 'G39-a' "$ref" "$line" "$rel" "文件或目录不存在：$t"
}

# --- 单条命令引用的判定 ----------------------------------------------------
judge_command() {
  local root="$1" tok="$2" ref="$3" line="$4"
  local first rest sub a
  first="${tok%% *}"
  rest="${tok#* }"

  # 🔴 2026-10-05：含通配符的 token **不是可执行引用**，不判定。
  #   起因：主理人自己写裁决记录时用了 `cargo xtask *`（意思是"这一整族都不存在"），
  #   结果被判红 —— 而带 `*` 的命令**根本跑不起来**，它陈述的是"一族命令"而非"这一条命令"。
  #   零漏报论证：不含通配符的写法（`cargo xtask boundary`）仍然照常判红，
  #   自检里有反向断言确保这条放行不会变成漏洞。
  case "$tok" in *'*'*|*'?'*) _g_skip_ext=$(( _g_skip_ext + 1 )); return 0 ;; esac

  case "$first" in
    cargo)
      sub="${rest%% *}"
      sub="${sub#--}"
      [[ -n "$sub" ]] || return 0
      if [[ "$CARGO_BUILTIN" == *" $sub "* ]]; then return 0; fi
      if [[ -z "$CARGO_BIN" ]]; then
        add_undet "${ref}:${line}  \`${tok}\`（本机找不到 cargo，无法验证子命令 '${sub}'）"
        return 0
      fi
      if probe_cargo_sub "$sub"; then
        return 0
      fi
      if is_waived "$tok"; then
        record_waiver "$tok"; return 0
      fi
      add_viol 'G39-b' "$ref" "$line" "$tok" "cargo 子命令不存在：cargo ${sub}"
      ;;
    bash|sh|python3|python|node)
      # 解释器 + 脚本路径：脚本是否真实存在正是本闸门的核心关注点
      for a in $rest; do
        case "$a" in -*) continue ;; esac
        judge_path "$root" "$a" "$ref" "$line"
      done
      ;;
    *)
      # 其他外部命令：是否安装取决于环境，不是文档缺陷 → 不判定，计数输出
      _g_skip_ext=$(( _g_skip_ext + 1 ))
      ;;
  esac
}

# --- 主判定函数（falsify 与正常模式共用同一份，绝不复制第二份）-------------
check_all() {
  local root="$1"; shift
  local -a files=("$@")
  [[ ${#files[@]} -gt 0 ]] || files=("$root/AGENTS.md")

  local f line tok head
  _collect_paths "$root"

  for f in "${files[@]}"; do
    if [[ ! -f "$f" ]]; then
      add_undet "目标文件不存在：$f"
      continue
    fi
    load_waivers "$f"

    while IFS=$'\t' read -r line tok; do
      [[ -n "$tok" ]] || continue
      _g_tok_all=$(( _g_tok_all + 1 ))

      if [[ "$tok" == *' '* && "$KNOWN_CMDS" == *" ${tok%% *} "* ]]; then
        _g_tok_cmd=$(( _g_tok_cmd + 1 ))
        judge_command "$root" "$tok" "${f#$root/}" "$line"
      else
        _g_tok_path=$(( _g_tok_path + 1 ))
        judge_path "$root" "$tok" "${f#$root/}" "$line"
      fi
    done < <(extract_tokens "$f")
  done

  # 恒绿检测：一个候选都没提取到 = 提取器可能已失效 → 无法判定（rc=2），不是通过
  if (( _g_tok_path == 0 && _g_tok_cmd == 0 )); then
    add_undet "未提取到任何候选引用（路径 0 / 命令 0）—— 疑似提取器失效，或该文档本就不含引用；本闸门不判绿"
  fi

  if (( _g_viol )); then return 1; fi
  if (( _g_undet )); then return 2; fi
  return 0
}

# --- 报告 -----------------------------------------------------------------
report() {
  local label="$1" e rid f l tok why n=0
  printf '\n%s\n' "${DIM}── ${GATE} (${label}) ──${RESET}"
  printf '反引号 token %d 个 → 路径候选 %d / 命令候选 %d\n' "$_g_tok_all" "$_g_tok_path" "$_g_tok_cmd"
  printf '已检查且存在 %d · 仓库内可达(非根相对) %d · 跳过(构建产物) %d · 未验证(外部命令) %d · 显式豁免 %d\n' \
    "$_g_path_ok" "$_g_resolved" "$_g_skip_build" "$_g_skip_ext" "$_g_waived"
  printf 'git 远端引用已核实存在 %d 条%s\n' "$_g_gitref_ok" \
    "$( (( _g_gitref_ok == 0 )) && printf '（本批文档未出现 origin/<分支> 形式，非"没检查"）' )"

  if (( _g_resolved > 0 )); then
    printf '%s\n' "  · 根相对不存在但仓库内可达 ${_g_resolved} 处（不判红，列出以免静默）："
    printf '%s\n' "${_RESOLVED[@]}" | sort -u | sed 's/^/      /'
  fi
  if (( _g_waived > 0 )); then
    printf '%s\n' "  ${YELLOW}· 显式豁免 ${_g_waived} 处（g39:waive，每个豁免都必须有理由）${RESET}"
    printf '%s\n' "${_WAIVED[@]}" | sed 's/^/      /'
  fi
  (( _g_skip_build > 0 )) && printf '  · 构建产物路径 %d 处未判定（存在与否取决于是否构建过）\n' "$_g_skip_build"
  (( _g_skip_ext   > 0 )) && printf '  · 外部命令 %d 处未验证（是否安装取决于环境，非文档缺陷）\n' "$_g_skip_ext"

  if (( _g_viol || _g_ref || _g_arch )); then
    local tier
    for e in "${_VIOL[@]}"; do
      IFS='|' read -r rid f l tok why tier <<<"$e"
      n=$(( n + 1 ))
      case "$tier" in
        authoritative)
          printf '%s\n' "  ${RED}✗ rule ${rid}: ${f}:${l}  \`${tok}\`${RESET}" >&2
          printf '%s\n' "      ${why}" >&2 ;;
        reference)
          printf '%s\n' "  ${YELLOW}~ rule ${rid} [reference 档·冻结]: ${f}:${l}  \`${tok}\`${RESET}" >&2
          printf '%s\n' "      ${why}" >&2 ;;
        *)
          printf '%s\n' "  ${DIM}- rule ${rid} [archived 档·已作废·不判定]: ${f}:${l}  \`${tok}\`${RESET}" >&2 ;;
      esac
    done
  fi

  # 🔴 分档计数**无条件输出**：「0」与「未检查」必须在屏幕上可区分
  #   （把计数藏在 if 里 = 干净时什么都不显示 = 与「没检查」无法分辨）
  printf '\n%s\n' "  ── 分档计数（本闸门的判定口径）──" >&2
  if (( _g_viol )); then
    printf '%s\n' "     ${RED}authoritative 违规 ${_g_vio_n} ← 计入 rc，只有这档会让闸门变红${RESET}" >&2
    printf '%s\n' "  ${RED}检出 ${_g_vio_n} 处假引用（rule ${GATE}-a 路径 / ${GATE}-b 命令，均在 authoritative 档）${RESET}" >&2
  else
    printf '%s\n' "     ${GREEN}authoritative 违规 0 ← 已检查完毕，0 处假引用${RESET}" >&2
  fi
  printf '%s\n' "     ${YELLOW}reference 冻结档 ${_g_ref}${RESET}   ← 计数但不计入 rc（上一团队设计详稿快照）" >&2
  printf '%s\n' "     ${DIM}archived 已作废 ${_g_arch}${RESET}   ← 不判定（仅留档）" >&2
  if (( _g_ref || _g_arch )); then
    printf '%s\n' "     ${YELLOW}⚠ 冻结档里仍有 ${_g_ref} 处未清（reference）+ ${_g_arch} 处（archived）。这些不是缺陷判定项，但它们意味着**照着这些文档写代码会写到不存在的文件** —— 实现只认 authoritative 档。${RESET}" >&2
  fi
}

# --- 能力③：--falsify（注入已知坏输入，跑与正常模式完全相同的判定）----------
if (( FALSIFY )); then
  discover_cargo
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"; [[ -n "${_PATHS:-}" && -f "${_PATHS}" ]] && rm -f "$_PATHS"' EXIT
  mkdir -p "$tmp/scripts"

  # 探针 A（G39-a）：确定违规的仓库内路径引用（裸引用 + 解释器调脚本两种形态）
  cat > "$tmp/probe-a.md" <<'EOF'
# probe A —— 故意注入的假路径引用
自检请运行 `scripts/definitely-not-here.sh` 与 `bash scripts/nope-neither.sh`。
EOF
  # 探针 B（G39-b）：确定违规的 cargo 子命令
  cat > "$tmp/probe-b.md" <<'EOF'
# probe B —— 故意注入的假子命令
自检请运行 `cargo definitely-not-a-subcommand-zzz`。
EOF

  # 🔴 判据必须是 check_all 的**返回值** + 命中规则的 ID。
  #    输出字样只作佐证：若检测逻辑被破坏（打印了 "✗ rule G39" 却 return 0），
  #    只 grep 字样会把"坏了"误判成"已检出"（AGENTS.md 陷阱 3）。
  # 🔴 断言绑定到探针实际命中的那条规则（A→G39-a，B→G39-b），
  #    绝不对另一条规则断言 —— 否则真实闸门会被误判为坏。
  for probe in a b; do
    _dbg=$(mktemp)
    check_all "$tmp" "$tmp/probe-$probe.md" >"$_dbg" 2>&1
    _rc=$?
    report "falsify/G39-$probe" >>"$_dbg" 2>&1
    _want="rule G39-$probe"
    if (( _rc != 1 )) || ! grep -q "$_want" "$_dbg"; then
      printf '%s\n' "${RED}✗ ${GATE} SELF-TEST FAILED (G39-${probe}): 已知坏输入未被检出 —— 闸门无法自证（check_all rc=${_rc}，期望 1）${RESET}" >&2
      sed 's/^/    /' "$_dbg" >&2
      printf '%s\n' "  ${YELLOW}这不是『没有问题』，而是『检测逻辑没生效』或『提取器没提取到』。${RESET}" >&2
      rm -f "$_dbg"; _PATHS=""
      exit 2
    fi
    rm -f "$_dbg"
    _reset_counters
  done

  printf '%s\n' "${RED}✗ ${GATE} rule SELF-TEST OK: 故意注入的假路径（G39-a）与假子命令（G39-b）均被真实检出（falsify 模式，rc=1 为预期）${RESET}" >&2
  exit 1
fi

# --- 正常模式 -------------------------------------------------------------
SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SELF_DIR/.." && pwd)"
discover_cargo

declare -a FILES=()
if (( ${#TARGETS[@]} > 0 )); then
  for t in "${TARGETS[@]}"; do
    case "$t" in /*) FILES+=("$t") ;; *) FILES+=("$ROOT/$t") ;; esac
  done
  # 🔴 路径基准根的判定：显式传入的目标文件若**不在仓库内**（自检的临时仓库、
  #   外部文档），基准根必须改用该文件所在目录 —— 否则会把临时仓库里真实存在的
  #   文件判成"不存在"（假红），或反过来把仓库外的路径判成存在（假绿）。
  _outside=0
  for t in "${TARGETS[@]}"; do
    case "$t" in
      /*) _f="$t" ;;
      *)  _f="$ROOT/$t" ;;
    esac
    [[ "$_f" == "$ROOT"/* ]] || _outside=1
  done
  if (( _outside )); then
    ROOT="$(cd "$(dirname "${FILES[0]}")" && pwd)"
  fi
else
  FILES+=("$ROOT/AGENTS.md")
  if (( SCAN_DOCS )) && [[ -d "$ROOT/docs" ]]; then
    while IFS= read -r f; do FILES+=("$f"); done < <(find "$ROOT/docs" -maxdepth 1 -name '*.md' | sort)
  fi
fi

trap '[[ -n "${_PATHS:-}" && -f "${_PATHS}" ]] && rm -f "$_PATHS"' EXIT
check_all "$ROOT" "${FILES[@]}"
_rc=$?
report 'normal'
if (( _rc == 0 )); then
  printf '\n%s\n' "${GREEN}═══ ${GATE} 通过：文档中的仓库内路径与命令引用均真实存在 ═══${RESET}"
fi
exit "$_rc"
