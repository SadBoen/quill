# quill-web

Quill 的单页前端，构建产物输出到 `dist/`，由 `quill-server` 通过 `QUILL_WEB_DIR`
（默认 `ui/web/dist`）挂载在 `/`。

## 来源

代码 vendor 自 [OpenOctopus](https://github.com/Zpoteiti/OpenOctopus) 的 `frontend/`
（MIT，Copyright 2026 Yucheng Zou，原许可证见 `LICENSE.OpenOctopus`）。

保留的部分是那套成熟前端轮子：React 19 + Vite 8 + TypeScript 5.9 + TanStack Query +
react-markdown + i18next 的整体骨架，以及设计系统（`index.css`、`chat/ChatPage.css`）、
主题切换、i18n、通用组件（`components/Page.tsx`）和聊天页的交互骨架。

为对接 quill 后端而重写的部分：

- `src/api/types.ts` — 原来引用 OpenOctopus 自己的 OpenAPI 生成类型（`openapi.d.ts`，5203 行），
  现按 quill `crates/quill-server/src/api_chat.rs` 的真实响应手写。
- `src/api/client.ts` — 增加 Bearer 令牌注入（`quill-token` 存 localStorage），
  quill 用 `QUILL_TOKENS=dev-token:@alice:admin` 而不是 Cookie 会话。
- `src/chat/chatApi.ts` — quill 是一次性返回 `reply` 的非流式接口，不是 NDJSON 流；
  附件上传、设备选择、effort 档位、渠道投递在 quill 没有对应端点，已移除。
- `src/chat/model.ts`、`src/chat/Transcript.tsx`、`src/chat/ChatPage.tsx` —
  消息模型换成 quill 的 `{id, seq, role, content, reasoning, turn_ms, usage}` 形状。
- `src/auth/auth.tsx` — 没有邮箱密码登录，改为输入访问令牌。
- `src/layout/AppShell.tsx`、`src/app/App.tsx`、`src/i18n/resources.ts` — 去掉
  workspace/devices/channels/automations/admin/account/memory 七个模块的导航与文案。

被整模块删除的目录：`account/`、`admin/`、`automations/`、`channels/`、`devices/`、
`memory/`、`workspace/`、`e2e/`，以及 `api/openapi.d.ts`、`api/ndjson.ts`、
`chat/AttachmentPicker.tsx`、`chat/attachments.ts`、`chat/useRecoveredHistory.ts`。
针对这些已删模块的测试一并移除，`src/api/client.test.ts` 保留并补了令牌注入用例。

## 开发

```bash
npm ci
npm run dev      # 开发服务器，/api 与 /healthz 代理到 QUILL_BACKEND（默认 127.0.0.1:18777）
npm run build    # 产物到 dist/
npm test         # vitest
```

访问令牌是服务端 `QUILL_TOKENS` 里配置的值，本地默认 `dev-token`。
