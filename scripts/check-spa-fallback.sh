#!/usr/bin/env bash
# ==============================================================================
# check-spa-fallback.sh —— SPA 深链 fallback 闸门（QA 门槛 G30）
#
# 问题：用户访问 /wiki/page/xxx 正常，**一刷新就白屏/404**。
# 原因：静态资源服务器不认识前端路由，未 fallback 到 index.html。
#
# ⚠️ 为什么这条闸门必须有自检（AGENTS.md「未自检的闸门不算闸门」）：
#   本地开发时 Vite dev server **总是**正确 fallback（它就是 dev server），
#   首页也永远正常 → **这个缺陷在本地 100% 测不出来，只在用户刷新深链时暴露**。
#   若不配"故意关掉 fallback 确认它会红"的自检，本脚本自己就可能是永不触发的摆设。
#
# 用法：
#   bash scripts/check-spa-fallback.sh [base_url] [path ...]
#   默认 base_url=http://127.0.0.1:3000，默认路径取常见深链
# 退出码：0=全部 200 / 1=有深链非 200 / 2=服务不可达或用法错误
# ==============================================================================
set -uo pipefail

if [[ -t 1 ]]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; RESET=''
fi

# 🔴 参数解析：--falsify 会被当成 BASE，必须先剔除。
#    否则 `--falsify` 单独调用时 BASE="--falsify" → 解析失败 → 误判。
FALSIFY=0
BASE=""
PATHS=()
for a in "$@"; do
  if [[ "$a" == "--falsify" ]]; then
    FALSIFY=1
  elif [[ -z "$BASE" ]]; then
    BASE="$a"
  else
    PATHS+=("$a")
  fi
done
[[ -z "$BASE" ]] && BASE="http://127.0.0.1:3000"
if [[ ${#PATHS[@]} -eq 0 ]]; then
  # 与 DEPLOY-CHECKLIST.md §9.1 保持一致
  PATHS=(/wiki /experts /sessions /settings)
fi


command -v curl >/dev/null 2>&1 || {
  printf '%s\n' "${RED}✗ 缺 curl${RESET}" >&2
  printf '%s\n' "  ⚠️ 禁止用 '|| echo 跳过' 兜底（AGENTS.md 铁律二）——缺工具必须失败" >&2
  exit 2
}

# --- 逐个深链检查 ----------------------------------------------------------
failed=0
# ⚠️ 借鉴 qa-engineer canary 的核心原则（08_测试与验收方案 §2.1.2）：
#     > 断言不写"检查为空"，写"植入标记再搜标记"
#     > assert!(x.is_empty()) 在**整体出错**时也成立（空是因为崩了，不是通过）
#
#    只判 200 + text/html 是不够的：服务可能返回
#      · 空 body            → 200 text/html 但白屏
#      · Nginx 错误页       → 200 text/html 但是别人的页面
#      · 登录重定向页       → 200 text/html 但进不去
#    因此**必须断言响应体确实含 SPA 入口**（root 容器或 script 标签）。
readonly BODY_MARKERS='id="root"|id='"'"'root'"'"'|<script'

# --- 真实检测（0=全部正常 / 1=有深链异常）--------------------------------
# 抽成函数，正常模式与 --falsify 共用同一代码路径
check_deep_links() {
local failed=0
for p in "${PATHS[@]}"; do
  # 一次取回 header 与 body
  tmp=$(mktemp)
  code=$(curl -s -o "$tmp" -D - --max-time 10 -w '%{http_code}' "$BASE$p" 2>/dev/null)
  ctype=$(printf '%s' "$code" | grep -i '^content-type:' | tail -1 | tr -d '\r')
  code=$(printf '%s' "$code" | tail -1)
  body=$(cat "$tmp" 2>/dev/null || true)
  rm -f "$tmp"

  if [[ "$code" != "200" ]]; then
    printf '%s\n' "${RED}✗ G30 rule G30-1: $p → $code${RESET}  ${YELLOW}(深链未 fallback 到 index.html)${RESET}" >&2
    failed=$((failed + 1))
  elif ! printf '%s' "$ctype" | grep -qi 'text/html'; then
    printf '%s\n' "${RED}✗ G30 rule G30-1: $p → 200 但 Content-Type 非 HTML${RESET} ${YELLOW}[$ctype]${RESET}" >&2
    failed=$((failed + 1))
  elif ! printf '%s' "$body" | grep -qE "$BODY_MARKERS"; then
    # 200 + text/html 但内容不是 SPA 入口 → 假绿，必须判红
    size=$(printf '%s' "$body" | wc -c | tr -d ' ')
    printf '%s\n' "${RED}✗ G30 rule G30-1: $p → 200 text/html 但响应体无 SPA 入口${RESET}" >&2
    printf '%s\n' "    ${YELLOW}body ${size} 字节，不含 id=\"root\" 或 <script> —— 疑似空页/错误页/重定向页${RESET}" >&2
    failed=$((failed + 1))
  else
    printf '%s\n' "${GREEN}✓ $p → 200 text/html + SPA 入口${RESET}"
  fi
done
return $failed
}

# --- 能力③：--falsify（必须在 check_deep_links 定义之后）--------------------
# ⚠️ 位置要求：本块调用 check_deep_links()，必须放在其定义之后。
#    这里起一个**故意不做 SPA fallback** 的真实 HTTP 服务，
#    然后跑**同一套深链检查**，确认它真的报红。
if (( FALSIFY )); then
  command -v python3 >/dev/null 2>&1 || {
    printf '%s\n' "${YELLOW}▲ G30 无法判定：缺 python3，无法起自检服务${RESET}" >&2
    printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
    exit 2
  }
  d=$(mktemp -d)
  trap 'rm -rf "$d"; kill ${SRV:-0} 2>/dev/null' EXIT
  cat > "$d/srv.py" <<'PY'
from http.server import BaseHTTPRequestHandler, HTTPServer
IDX = b'<!doctype html><html><head><script src="/a.js"></script></head><body><div id="root"></div></body></html>'
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        p = self.path.split('?')[0]
        if p == '/':                      # 只有首页正常，深链全 404（故意不 fallback）
            self.send_response(200); self.send_header('Content-Type','text/html')
            self.send_header('Content-Length', str(len(IDX))); self.end_headers(); self.wfile.write(IDX)
        else:
            self.send_error(404)
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', 18991), H).serve_forever()
PY
  python3 "$d/srv.py" & SRV=$!
  sleep 1.2
  out=$(BASE="http://127.0.0.1:18991" PATHS=(/wiki) check_deep_links 2>&1); rc=$?
  kill $SRV 2>/dev/null
  if ((rc == 0)); then
    printf '%s\n' "${RED}✗ G30 SELF-TEST FAILED: 故意不 fallback 的服务未被检出 —— 闸门无法自证${RESET}" >&2
    printf '%s\n' "  ${YELLOW}这不是『没有问题』，而是『检测逻辑没生效』。${RESET}" >&2
    exit 2
  fi
  printf '%s\n' "${RED}✗ G30 rule SELF-TEST: 故意不 fallback 的服务已被真实检出（falsify 模式，rc=1 为预期）${RESET}" >&2
  exit 1
fi

command -v curl >/dev/null 2>&1 || {
  printf '%s\n' "${YELLOW}▲ G30 无法判定：缺 curl${RESET}" >&2
  printf '%s\n' "  禁止用 '|| echo 跳过' 兜底（AGENTS.md 铁律二）——缺工具必须无法判定而非通过" >&2
  exit 2
}

# --- 前置：服务是否可达（不可达 = 无法判定，不是"通过"）------------------
if ! curl -fsS -o /dev/null --max-time 5 "$BASE/" 2>/dev/null; then
  printf '%s\n' "${YELLOW}▲ G30 无法判定：服务不可达（$BASE）${RESET}" >&2
  printf '%s\n' "  先启动服务，或传入正确的 base_url" >&2
  printf '%s\n' "  ⚠️ 未判定 ≠ 通过。本项不计入通过。" >&2
  exit 2
fi

if check_deep_links >/dev/null 2>&1; then
  rc=0
else
  rc=1
  check_deep_links
fi

if ((rc == 1)); then
  cat >&2 <<'EOF'

  典型症状：用户打开深链正常，刷新后白屏/404。
  根因：静态资源层未对未知路径 fallback 到 index.html。
  修复：quill-server 的 ServeDir / fallback 层需将非 /api、/ws 的未知路径
        全部回落到 index.html（前端路由使用真实路径，非 hash 路由）。
EOF
  exit 1
fi

check_deep_links
printf '\n%s\n' "${GREEN}═══ ${#PATHS[@]} 个深链均返回 200 text/html，SPA fallback 正常 ═══${RESET}"
exit 0
