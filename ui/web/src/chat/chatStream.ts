/**
 * 读服务端那条 SSE 路由。
 *
 * 为什么不用 `EventSource`：它只能 GET，而发消息必须 POST 带 body。
 * 所以只能用 `fetch` + `ReadableStream.getReader()` 自己拆帧 ——
 * 拆帧规则就两条：`event:` / `data:` 两行一组，空行收尾，`:` 开头的是注释。
 *
 * 这一层**只负责把字节变成事件**。收到增量之后界面怎么显示是 ChatPage 的事，
 * 两边不互相知道对方的内部形状。
 */

import { ApiError, readErrorEnvelope, TOKEN_STORAGE_KEY } from '../api/client'
import type { SendMessageResponse } from '../api/types'

export type StreamEvent =
  | { kind: 'user_message'; data: SendMessageResponse['user_message'] }
  | { kind: 'delta'; data: { text: string; deltaKind: 'text' | 'reasoning' } }
  | { kind: 'tool_call'; data: { name: string; arguments: unknown } }
  | { kind: 'tool_result'; data: { name: string; ok: boolean } }
  | { kind: 'discard'; data: { round: number; text: string; reasoning: string } }
  | { kind: 'done'; data: SendMessageResponse }
  | { kind: 'error'; data: { code: string; detail: string; next_step: string } }

export interface StreamHandlers {
  /** 每一个事件都会经过这里。抛错会中断这次读取。 */
  onEvent?: (event: StreamEvent) => void
  signal?: AbortSignal
}

function token(): string {
  try {
    return window.localStorage.getItem(TOKEN_STORAGE_KEY) ?? ''
  } catch {
    return ''
  }
}

/**
 * 发一句话，逐帧回调，最后交回那条完整的 `done`。
 *
 * 三种结束都要有说法，绝不能静悄悄地停：
 *   - 收到 `done` → 返回它；
 *   - 收到 `error` → 抛 `ApiError`（detail 与 next_step 都是服务端写的）；
 *   - 连接断了却既没有 `done` 也没有 `error` → 抛错。
 *     「一半的话」不能当成答案显示出去。
 */
export async function streamChatMessage(
  sessionId: string,
  text: string,
  handlers: StreamHandlers = {},
): Promise<SendMessageResponse> {
  const response = await fetch(`/api/sessions/${encodeURIComponent(sessionId)}/messages/stream`, {
    method: 'POST',
    credentials: 'same-origin',
    headers: {
      'Content-Type': 'application/json',
      ...(token() ? { Authorization: `Bearer ${token()}` } : {}),
    },
    body: JSON.stringify({ content: text }),
    signal: handlers.signal,
  })

  if (!response.ok) {
    // 准备阶段的错误仍然是普通 JSON 信封（内容为空、会话不存在、没配模型），
    // 与老路由完全一样 —— 所以这里复用同一套解析，而不是自己编一套。
    const body = await response.json().catch(() => undefined)
    throw readErrorEnvelope(response, body)
  }
  if (!response.body) {
    throw new ApiError(0, 'stream_unsupported', '这条响应没有可读的流，浏览器不支持 ReadableStream。')
  }
  const contentType = response.headers.get('content-type') ?? ''
  if (!contentType.includes('text/event-stream')) {
    // 服务端回的不是流。硬往下拆只会得到一个 `JSON.parse` 的语法错误，
    // 用户看到的是一句完全摸不着头脑的话。
    throw new ApiError(
      response.status,
      'not_a_stream',
      `这条接口回的不是事件流（content-type：${contentType || '（空）'}）。`,
      '确认 quill-server 是支持流式的那一版（POST /api/sessions/{id}/messages/stream），并重启它。',
    )
  }

  const reader = response.body.getReader()
  const decoder = new TextDecoder('utf-8')
  let buffer = ''
  // 当前正在攒的那一帧。SSE 的分帧规则只有两行一空行，所以只需要这两个。
  let event = ''
  let data = ''
  let result: SendMessageResponse | null = null
  let failure: ApiError | null = null

  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    // 一个 UTF-8 字符可能被 TCP 拆在两半，decoder 里的 stream 模式负责拼回来。
    buffer += decoder.decode(value, { stream: true })

    let newline = buffer.indexOf('\n')
    while (newline >= 0) {
      const line = buffer.slice(0, newline).replace(/\r$/, '')
      buffer = buffer.slice(newline + 1)
      const handled = handleLine(line)
      if (!handled) {
        // 空行：一帧结束。
        flushFrame()
      }
      newline = buffer.indexOf('\n')
    }
  }
  buffer += decoder.decode()
  // 最后一帧后面没有空行也算数（服务端总以空行收尾，这里只是兜底）。
  flushFrame()

  if (failure) throw failure
  if (!result) {
    throw new ApiError(
      0,
      'stream_truncated',
      '连接在回复写完之前就断了，界面上的这段话不是完整答案。',
      '检查 quill-server 是否还在运行、浏览器网络面板里这条请求是否被中断，然后重发一次。',
    )
  }
  return result

  function handleLine(line: string): boolean {
    if (line === '') return false
    if (line.startsWith(':')) return true // keep-alive 注释
    const colon = line.indexOf(':')
    const field = colon < 0 ? line : line.slice(0, colon)
    const raw = colon < 0 ? '' : line.slice(colon + 1)
    const value = raw.startsWith(' ') ? raw.slice(1) : raw
    if (field === 'event') event = value
    else if (field === 'data') data += value
    // 别的字段按 SSE 规范忽略。
    return true
  }

  function flushFrame(): void {
    if (event === '' && data === '') return
    const name = event === '' ? 'message' : event
    const payload = data === '' ? {} : (JSON.parse(data) as Record<string, unknown>)
    event = ''
    data = ''
    const parsed = toStreamEvent(name, payload)
    // 认不出的事件名直接丢帧而不是当成错误：服务端以后加一个新事件时，
    // 老界面的正确反应是「没看见」，不是「整轮崩掉」。
    if (parsed) handlers.onEvent?.(parsed)
  }

  function toStreamEvent(name: string, payload: Record<string, unknown>): StreamEvent | null {
    switch (name) {
      case 'user_message':
        return {
          kind: 'user_message',
          data: payload as unknown as SendMessageResponse['user_message'],
        }
      case 'delta':
        return {
          kind: 'delta',
          data: {
            text: String(payload.text ?? ''),
            deltaKind: payload.kind === 'reasoning' ? 'reasoning' : 'text',
          },
        }
      case 'tool_call':
        return { kind: 'tool_call', data: { name: String(payload.name ?? ''), arguments: payload.arguments } }
      case 'tool_result':
        return { kind: 'tool_result', data: { name: String(payload.name ?? ''), ok: payload.ok === true } }
      case 'discard':
        return {
          kind: 'discard',
          data: {
            round: Number(payload.round ?? 0),
            text: String(payload.text ?? ''),
            reasoning: String(payload.reasoning ?? ''),
          },
        }
      case 'done':
        result = payload as unknown as SendMessageResponse
        return { kind: 'done', data: result }
      case 'error':
        failure = new ApiError(
          0,
          String(payload.code ?? 'unknown'),
          String(payload.detail ?? '这一轮失败了。'),
          String(payload.next_step ?? '') || null,
        )
        return {
          kind: 'error',
          data: {
            code: String(payload.code ?? 'unknown'),
            detail: String(payload.detail ?? ''),
            next_step: String(payload.next_step ?? ''),
          },
        }
      default:
        // 认不出的事件名直接丢帧而不是当成错误：服务端加一个新事件时，
        // 老界面的正确反应是「没看见」，不是「整轮崩掉」。
        return null
    }
  }
}