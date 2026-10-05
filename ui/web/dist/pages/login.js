// 登录页 —— 未登录时唯一可达的页面。

import { api, setToken, saveSessionUser } from '../lib/api.js';
import { h, mount, errorPanel, notePanel } from '../lib/dom.js';

export function render(root, ctx) {
  const username = h('input', {
    class: 'field-input',
    type: 'text',
    name: 'username',
    placeholder: '用户名',
    autocomplete: 'username',
    required: true,
  });
  const password = h('input', {
    class: 'field-input',
    type: 'password',
    name: 'password',
    placeholder: '密码',
    autocomplete: 'current-password',
    required: true,
  });
  const button = h('button', { class: 'btn btn-primary', type: 'submit', text: '登录' });
  const output = h('div', { class: 'stack' });

  const form = h('form', {
    class: 'panel form',
    novalidate: true,
    on: {
      submit: async (event) => {
        event.preventDefault();
        const name = username.value.trim();
        const secret = password.value;
        if (!name || !secret) {
          mount(output, [
            errorPanel({
              code: 'local_input',
              status: 0,
              detail: '用户名或密码没有填。',
              nextStep: '把用户名和密码都填上再点登录。密码不会被保存，页面刷新后需要重新输入。',
            }),
          ]);
          return;
        }
        // 密码只在这一次请求里用一次，之后立刻从输入框清掉，且绝不写进 localStorage。
        button.disabled = true;
        button.textContent = '登录中…';
        mount(output, [h('p', { class: 'muted', text: '正在向后端校验凭据…' })]);
        try {
          const result = await api.login(name, secret);
          const token = result && result.token;
          if (!token) {
            mount(output, [
              errorPanel({
                code: 'bad_login_response',
                status: 0,
                detail: '后端返回了登录成功响应，但里面没有令牌，登录无法继续。',
                nextStep: '这说明后端 /api/auth/login 的响应形状与约定不一致。把这次响应报给维护者，不要反复点登录。',
              }),
            ]);
            return;
          }
          setToken(token);
          password.value = '';
          const user = (result && result.user) || { username: name };
          saveSessionUser({
            username: user.username || name,
            display_name: user.display_name || '',
            user_id: user.user_id || '',
            is_admin: Boolean(user.is_admin),
          });
          mount(output, [
            notePanel('ok', '登录成功', '正在进入总览页…'),
          ]);
          ctx.onLoggedIn();
        } catch (err) {
          // 401 时后端刻意不区分「用户名错」与「密码错」，前端文案也不得区分。
          const detail = err.status === 401
            ? err.detail + '（用户名或密码不正确。）'
            : err.detail;
          mount(output, [errorPanel({ code: err.code, status: err.status, detail: detail, nextStep: err.nextStep })]);
        } finally {
          button.disabled = false;
          button.textContent = '登录';
        }
      },
    },
  }, [
    h('h2', { class: 'page-title', text: '登录' }),
    h('p', { class: 'muted', text: '登录后本页面会保存令牌，刷新浏览器不用重新登录。密码不会被保存。' }),
    h('label', { class: 'field' }, [h('span', { class: 'field-label', text: '用户名' }), username]),
    h('label', { class: 'field' }, [h('span', { class: 'field-label', text: '密码' }), password]),
    h('div', { class: 'form-actions' }, [button]),
  ]);

  mount(root, [form, output]);
  username.focus();
}
