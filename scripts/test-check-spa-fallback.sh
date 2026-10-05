#!/bin/bash
# ==============================================================================
# check-spa-fallback.sh 的闸门自检（G30）
#
# 依据 AGENTS.md「未自检的闸门不算闸门」+ qa-engineer 的明确要求：
#   > 建议做成脚本 + **一条自检：故意关掉 fallback，确认它会红**
#     —— 否则它就是个永不触发的摆设
#
# 本测试用 python3 起两个真实 HTTP 服务：
#   A. 正确 fallback 的服务   → 闸门必须绿
#   B. 故意不 fallback 的服务 → 闸门必须红   ← 关键用例
# 并额外验证：服务不可达时退出码为 2（不是 0，避免"查不到=通过"）
# ==============================================================================
set -uo pipefail
# ⚠️ 可移植性（2026-10-05 qa-engineer 修正）：此前 GATE 硬编码为**本机绝对路径**
#    /mnt/d/96_CoderWorld/quill/... → CI 干净 checkout 上不可达，
#    而"不可达"极易被误读成"通过"。改为由脚本自身位置推算仓库根。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="${GATE_SELFTEST_GATE:-$ROOT/scripts/check-spa-fallback.sh}"
pass=0; fail=0

# 🔴 前置 1：闸门脚本必须存在且语法正确
[ -f "$GATE" ] || { printf '  \033[33m▲ 无法判定：闸门脚本不存在：%s\033[0m\n' "$GATE" >&2
                    printf '    ⚠️ 未判定 ≠ 通过。\n' >&2; exit 2; }
bash -n "$GATE" 2>/dev/null || { printf '  \033[33m▲ 无法判定：闸门语法错误：%s\033[0m\n' "$GATE" >&2
                               printf '    ⚠️ 未判定 ≠ 通过。\n' >&2; exit 2; }

# 🔴 前置 2：缺 python3 必须是**无法判定（rc=2）**，绝不能 exit 0。
#    旧写法 `echo "SKIP: 无 python3"; exit 0` 是教科书级的第 2 类假成功 ——
#    缺工具的机器上"永远通过"。措辞须含 ▲ 以符合 runner 的 check_wording。
if ! command -v python3 >/dev/null 2>&1; then
  printf '  \033[33m▲ 无法判定：缺 python3，无法起自检用的真实 HTTP 服务\033[0m\n' >&2
  printf '    ⚠️ 未判定 ≠ 通过。禁止用 || true / "SKIP" 兜底（AGENTS.md 铁律二）。\n' >&2
  exit 2
fi

# 🔴 前置 3：本自检**自己**起服务（18801~18805），不依赖任何外部服务。
#    但若这些端口已被**别的进程**真正监听，上面的 server.py 会因 bind 失败
#    而**静默退出**，随后请求打到别人的服务上 → 用例结论与被测对象无关
#    （第 5 类门槛打错目标）。故先确认端口空闲，被占用即判「无法判定」。
#
# ⚠️ 2026-10-05 实测踩坑：探针**必须**带 SO_REUSEADDR。
#    本自检上一轮用 curl 访问过这些端口，退出后 socket 处于 TIME_WAIT。
#    HTTPServer 的 `allow_reuse_address` 默认为 1（= SO_REUSEADDR），
#    而不带该选项的探针 bind 会被 TIME_WAIT 拒绝 →
#    「上一轮残留」被误判成「端口被别人占用」→ 自检**连续第二次运行必红**（误报）。
#    对齐语义后：TIME_WAIT 放行（与真实服务一致），真正被监听仍报占用。
ports_free() {
  python3 - "$@" <<'PY'
import socket, sys
for p in sys.argv[1:]:
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)  # 与 HTTPServer 一致
    try:
        s.bind(("127.0.0.1", int(p)))
    except OSError:
        sys.exit(1)
    finally:
        s.close()
sys.exit(0)
PY
}
if ! ports_free 18801 18802 18803 18804 18805; then
  printf '  \033[33m▲ 无法判定：18801~18805 端口被占用，自检服务无法独占端口\033[0m\n' >&2
  printf '    ⚠️ 未判定 ≠ 通过。若强行继续，用例可能测到的是别人的服务。\n' >&2
  exit 2
fi

# 每次运行用独立目录（只增不删），避免上一轮残留污染本轮结论
BASE_TMP="${TMPDIR:-/tmp}/spatest-$$-$(date +%N 2>/dev/null || echo 0)"
mkdir -p "$BASE_TMP/good" "$BASE_TMP/bad"
# heredoc 是带引号的（<<'PY'），变量不会展开 → 由环境变量传给 Python
export SPATEST_INDEX="$BASE_TMP/index.html"
echo '<!doctype html><html><head><script type="module" src="/assets/index-a1b2c3.js"></script></head><body><div id="root"></div></body></html>' > "$BASE_TMP/index.html"

# A. 正确 fallback
cat > $BASE_TMP/good/server.py <<'PY'
from http.server import BaseHTTPRequestHandler, HTTPServer
import os
IDX = open(os.environ['SPATEST_INDEX'],'rb').read()
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        p = self.path.split('?')[0]
        if p == '/' or p.startswith(('/wiki','/experts','/sessions','/settings')):
            self.send_response(200); self.send_header('Content-Type','text/html')
            self.end_headers(); self.wfile.write(IDX)
        else:
            self.send_error(404)
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', 18801), H).serve_forever()
PY

# B. 故意不 fallback（模拟 SPA fallback 未配好）
cat > $BASE_TMP/bad/server.py <<'PY'
from http.server import BaseHTTPRequestHandler, HTTPServer
import os
IDX = open(os.environ['SPATEST_INDEX'],'rb').read()
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        p = self.path.split('?')[0]
        if p == '/':                      # 只有首页正常，深链全 404
            self.send_response(200); self.send_header('Content-Type','text/html')
            self.end_headers(); self.wfile.write(IDX)
        else:
            self.send_error(404)
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', 18802), H).serve_forever()
PY

python3 $BASE_TMP/good/server.py & GOOD=$!
python3 $BASE_TMP/bad/server.py  & BAD=$!
trap 'kill $GOOD $BAD 2>/dev/null' EXIT
sleep 1.5

echo "########## G30 SPA fallback 闸门自检 ##########"

# 1) 正确 fallback → 必须放行
bash "$GATE" http://127.0.0.1:18801 >/dev/null 2>&1; rc=$?
if [ "$rc" = "0" ]; then printf '  \033[32m✓\033[0m %-44s 退出码=0（放行）\n' "A. 正确 fallback"; pass=$((pass+1))
else printf '  \033[31m✗\033[0m %-44s 退出码=%s（期望 0）\n' "A. 正确 fallback" "$rc"; fail=$((fail+1)); fi

# 2) ★关键用例：故意关掉 fallback → 必须变红
bash "$GATE" http://127.0.0.1:18802 >/dev/null 2>&1; rc=$?
if [ "$rc" = "1" ]; then printf '  \033[32m✓\033[0m %-44s 退出码=1（变红）\n' "B. 故意关掉 fallback ★关键"; pass=$((pass+1))
else printf '  \033[31m✗\033[0m %-44s 退出码=%s（期望 1）\n' "B. 故意关掉 fallback ★关键" "$rc"; fail=$((fail+1)); fi

# 3) 服务不可达 → 退出码 2（不能是 0，否则"查不到=通过"又是假闸门）
bash "$GATE" http://127.0.0.1:18999 >/dev/null 2>&1; rc=$?
if [ "$rc" = "2" ]; then printf '  \033[32m✓\033[0m %-44s 退出码=2\n' "C. 服务不可达"; pass=$((pass+1))
else printf '  \033[31m✗\033[0m %-44s 退出码=%s（期望 2）\n' "C. 服务不可达" "$rc"; fail=$((fail+1)); fi

# 4) 只判 200 不够：200 但非 HTML 也应判红
cat > $BASE_TMP/bad/json200.py <<'PY'
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.send_header('Content-Type','application/json')
        self.end_headers(); self.wfile.write(b'{"err":"not html"}')
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', 18803), H).serve_forever()
PY
python3 $BASE_TMP/bad/json200.py & J=$!
sleep 1.2
bash "$GATE" http://127.0.0.1:18803 >/dev/null 2>&1; rc=$?
kill $J 2>/dev/null
if [ "$rc" = "1" ]; then printf '  \033[32m✓\033[0m %-44s 退出码=1（变红）\n' "D. 200 但非 text/html"; pass=$((pass+1))
else printf '  \033[31m✗\033[0m %-44s 退出码=%s（期望 1）\n' "D. 200 但非 text/html" "$rc"; fail=$((fail+1)); fi

# 5. ★200 + text/html 但 body 为空 → 必须红（只判状态码会假绿）
cat > $BASE_TMP/bad/empty200.py <<'PY'
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.send_header('Content-Type','text/html')
        self.send_header('Content-Length','0'); self.end_headers()
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', 18804), H).serve_forever()
PY
python3 $BASE_TMP/bad/empty200.py & E=$!
sleep 1.2
bash "$GATE" http://127.0.0.1:18804 >/dev/null 2>&1; rc=$?
kill $E 2>/dev/null
if [ "$rc" = "1" ]; then printf '  \033[32m✓\033[0m %-44s 退出码=1（变红）\n' "E. 200+HTML 但 body 空 ★"; pass=$((pass+1))
else printf '  \033[31m✗\033[0m %-44s 退出码=%s（期望 1）\n' "E. 200+HTML 但 body 空 ★" "$rc"; fail=$((fail+1)); fi

# 6. ★200 + text/html 但是错误页（无 SPA 入口）→ 必须红
cat > $BASE_TMP/bad/errpage.py <<'PY'
from http.server import BaseHTTPRequestHandler, HTTPServer
BODY=b'<html><head><title>502 Bad Gateway</title></head><body><h1>502</h1></body></html>'
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.send_header('Content-Type','text/html')
        self.send_header('Content-Length',str(len(BODY)))
        self.end_headers(); self.wfile.write(BODY)
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', 18805), H).serve_forever()
PY
python3 $BASE_TMP/bad/errpage.py & P=$!
sleep 1.2
bash "$GATE" http://127.0.0.1:18805 >/dev/null 2>&1; rc=$?
kill $P 2>/dev/null
if [ "$rc" = "1" ]; then printf '  \033[32m✓\033[0m %-44s 退出码=1（变红）\n' "F. 200+HTML 但是错误页 ★"; pass=$((pass+1))
else printf '  \033[31m✗\033[0m %-44s 退出码=%s（期望 1）\n' "F. 200+HTML 但是错误页 ★" "$rc"; fail=$((fail+1)); fi

echo
echo "########## 对照：naive 检查的失效 ##########"
echo "  naive：只 curl 首页 → 永远通过（首页在两个服务里都正常）"
echo "  naive2：只判 200 + Content-Type → 空 body / 502 错误页会假绿"
echo "  正确：测深链 + 验 Content-Type + 验响应体含 SPA 入口 + 区分不可达(2)"

echo
echo "########## 结果：通过 $pass / 失败 $fail ##########"
# 🔴 收尾必须带 exit（2026-10-05 qa-engineer 修正，第 2 类假成功）：
#    此前末尾没有 exit → 无论成败都退出 0 → run-gate-selftest.sh 按返回值分类
#    → G30 在结构上**永远判不了红**。
printf 'SELFTEST_%s\n' "$([ "$fail" -eq 0 ] && echo PASS || echo FAIL)"
[ "$fail" -eq 0 ] || exit 1
exit 0
