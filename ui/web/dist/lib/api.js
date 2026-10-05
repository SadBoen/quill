// API 客户端 —— 同源、零第三方依赖。
//
// 三条硬约束在这里落地：
//   ① 每个失败都必须带「中文说明 + 下一步」，缺一个就是前端在制造静默失败；
//   ② fetch 本身抛异常（后端没启动）要翻译成同一种结构，不能白屏；
//   ③ 令牌只放 localStorage 的 quill.token，密码永远不落盘。

const TOKEN_KEY = 'quill.token';
const USER_KEY = 'quill.user.session';

/** 统一失败对象：detail 是中文说明，nextStep 是下一步该做什么。 */
export class ApiFailure {
  constructor({ code, status, detail, nextStep }) {
    this.code = code;
    this.status = status;
    this.detail = detail;
    this.nextStep = nextStep;
  }
}

// ── 令牌 ─────────────────────────────────────────────────────────────

export function getToken() {
  try {
    return window.localStorage.getItem(TOKEN_KEY) || '';
  } catch (_) {
    return '';
  }
}

export function setToken(token) {
  try {
    window.localStorage.setItem(TOKEN_KEY, token);
  } catch (_) {
    /* 隐私模式下写入会失败：此时本次会话仍可用内存中的令牌，不崩溃。 */
  }
}

export function clearToken() {
  try {
    window.localStorage.removeItem(TOKEN_KEY);
    window.sessionStorage.removeItem(USER_KEY);
  } catch (_) {
    /* 同上：清不掉不算致命，下次登录会覆盖。 */
  }
}

/** 登录后显示用的用户名等展示信息 —— 放 sessionStorage，刷新标签页仍在。 */
export function saveSessionUser(user) {
  try {
    window.sessionStorage.setItem(USER_KEY, JSON.stringify(user || {}));
  } catch (_) {
    /* 展示信息写不进去不影响鉴权。 */
  }
}

export function loadSessionUser() {
  try {
    const raw = window.sessionStorage.getItem(USER_KEY);
    return raw ? JSON.parse(raw) : null;
  } catch (_) {
    return null;
  }
}

// ── 请求 ─────────────────────────────────────────────────────────────

const TIMEOUT_MS = 20000;

/**
 * 发一个同源 JSON 请求。
 * 成功返回解析后的 JSON；失败一律 throw ApiFailure（结构固定，便于统一渲染）。
 */
export async function request(method, path, options = {}) {
  const headers = {};
  const token = getToken();
  if (options.auth !== false && token) headers.Authorization = 'Bearer ' + token;

  let body;
  if (options.body !== undefined) {
    headers['Content-Type'] = 'application/json';
    body = JSON.stringify(options.body);
  }

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), options.timeoutMs || TIMEOUT_MS);

  let response;
  try {
    response = await fetch(path, {
      method: method,
      headers: headers,
      body: body,
      signal: controller.signal,
      credentials: 'same-origin',
    });
  } catch (cause) {
    clearTimeout(timer);
    throw unreachableFailure(method, path, cause);
  }
  clearTimeout(timer);

  const raw = await response.text();
  let data = null;
  if (raw) {
    try {
      data = JSON.parse(raw);
    } catch (_) {
      data = null;
    }
  }

  if (!response.ok) throw failureFromResponse(response.status, data, method, path);
  return data;
}

/** 后端不可达：网络层失败，不是 4xx/5xx。 */
function unreachableFailure(method, path, cause) {
  const aborted = cause && cause.name === 'AbortError';
  return new ApiFailure({
    code: 'backend_unreachable',
    status: 0,
    detail: aborted
      ? '请求超时（超过 20 秒没收到响应）：' + method + ' ' + path + '。'
      : '连不上后端服务：' + method + ' ' + path + ' 没有到达服务端。',
    nextStep: aborted
      ? '先确认后端还在运行（systemctl status quill 或看它的日志），再点页面上的「重新检查」。若后端在跑但很慢，执行 quill doctor 打印诊断。'
      : '后端未启动或没监听本机端口。先启动服务：在项目目录执行 cargo run -p quill-server，或用 systemd 执行 systemctl start quill；启动后点右上角「重新检查」。',
  });
}

/**
 * 把响应体翻译成 ApiFailure。
 * 后端真实形状是 { error: { code, detail, next_step } }；
 * 同时兼容扁平的 { code, message|detail, next_step }，避免上游换形状时前端整页报错。
 */
function failureFromResponse(status, data, method, path) {
  const envelope = (data && typeof data === 'object' && data.error) ? data.error : (data || {});
  const detail = pickText(envelope.detail, envelope.message, envelope.error);
  const nextStep = pickText(envelope.next_step, envelope.nextStep);
  const code = pickText(envelope.code) || 'unknown';

  if (!detail) {
    return new ApiFailure({
      code: code,
      status: status,
      detail: '后端返回了 ' + status + '，但响应里没有中文说明（' + method + ' ' + path + '）。',
      nextStep: '这说明后端错误格式与前端约定不一致。执行 quill doctor 打印诊断，并把这次失败的请求路径与状态码报给维护者，不要当成「接口正常但没数据」。',
    });
  }

  return new ApiFailure({
    code: code,
    status: status,
    detail: detail,
    nextStep: nextStep || '后端没有给出下一步指引。执行 quill doctor 打印完整诊断，按诊断里的修复动作处理后重试同一操作。',
  });
}

function pickText(...values) {
  for (const value of values) {
    if (typeof value === 'string' && value.trim()) return value;
  }
  return '';
}

// ── 具体接口 ─────────────────────────────────────────────────────────

export const api = {
  login: (username, password) =>
    request('POST', '/api/auth/login', { auth: false, body: { username: username, password: password } }),

  logout: () => request('POST', '/api/auth/logout', { body: {} }),

  me: () => request('GET', '/api/auth/me'),

  health: () => request('GET', '/healthz', { auth: false }),

  listExperts: () => request('GET', '/api/experts'),

  createExpert: (expert) => request('POST', '/api/experts', { body: expert }),

  patchExpert: (id, patch) =>
    request('PATCH', '/api/experts/' + encodeURIComponent(id), { body: patch }),

  deleteExpert: (id) =>
    request('DELETE', '/api/experts/' + encodeURIComponent(id), { body: {} }),

  listWikiPages: () => request('GET', '/api/wiki/pages'),

  getWikiPage: (pagePath) =>
    request('GET', '/api/wiki/pages/' + encodeURIComponent(pagePath)),

  queryWiki: (q, topK) => request('POST', '/api/wiki/query', { body: { q: q, top_k: topK } }),

  exportBackup: () => request('POST', '/api/backup/export', { body: {} }),

  verifyBackup: (dir) => request('POST', '/api/backup/verify', { body: { dir: dir } }),

  inflight: () => request('GET', '/api/dispatch/inflight'),
};
