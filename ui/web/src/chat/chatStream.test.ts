import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'

import { ApiError } from '../api/client'
import { streamChatMessage, type StreamEvent } from './chatStream'

/**
 * 一个可以由测试自己往里塞块的响应体。
 *
 * 关键在「一块一块塞」：一次 `enqueue` 全部内容的话，界面上的分帧逻辑
 * 根本不会被触发，而那正是这套代码唯一值得测的地方。
 */
function controllableStream() {
  const encoder = new TextEncoder()
  let controller!: ReadableStreamDefaultController<Uint8Array>
  const stream = new ReadableStream<Uint8Array>({
    start(c) {
      controller = c
    },
  })
  return {
    response: () =>
      new Response(stream, { status: 200, headers: { 'content-type': 'text/event-stream' } }),
    send: (text: string) => controller.enqueue(encoder.encode(text)),
    /** 把一段 UTF-8 拆成两半塞进去，模拟 TCP 把一个汉字切成两片。 */
    sendSplitInHalf(text: string) {
      const bytes = encoder.encode(text)
      const cut = Math.floor(bytes.length / 2)
      controller.enqueue(bytes.slice(0, cut))
      controller.enqueue(bytes.slice(cut))
    },
    close: () => controller.close(),
  }
}

const DONE_BODY = {
  session_id: 'S1',
  user_message: { id: 'U1', seq: 1, content: '你好', created_at: 1 },
  reply: '最终答案。',
  reasoning: '',
  message: { id: 'A1', seq: 2, content: '最终答案。', created_at: 2 },
  finish_reason: 'Stop',
  usage: { input: 30, output: 8 },
  turn_ms: 1,
  tool_calls: [],
  tool_rounds: 0,
}

function stubFetch(response: () => Response) {
  const mock = vi.fn(async () => response())
  vi.stubGlobal('fetch', mock)
  return mock
}

beforeEach(() => {
  localStorage.clear()
})
afterEach(() => {
  vi.unstubAllGlobals()
})

describe('帧解析', () => {
  it('把每一帧交出来，并把 done 的负载原样交回', async () => {
    const s = controllableStream()
    const events: StreamEvent[] = []
    stubFetch(() => s.response())

    const pending = streamChatMessage('S1', '你好', { onEvent: (e) => events.push(e) })

    s.send('event: user_message\ndata: {"id":"U1","seq":1,"content":"你好","created_at":1}\n\n')
    s.send('event: delta\ndata: {"kind":"text","text":"你"}\n\n')
    s.send('event: delta\ndata: {"kind":"reasoning","text":"想"}\n\n')
    s.send(`event: done\ndata: ${JSON.stringify(DONE_BODY)}\n\n`)
    s.close()

    const result = await pending
    expect(events.map((e) => e.kind)).toEqual(['user_message', 'delta', 'delta', 'done'])
    expect(result.reply).toBe('最终答案。')
    const deltas = events.filter((e) => e.kind === 'delta')
    expect(deltas[0].kind === 'delta' && deltas[0].data).toEqual({ text: '你', deltaKind: 'text' })
    // 思考增量不能混进正文：界面上它是折叠的另一块。
    expect(deltas[1].kind === 'delta' && deltas[1].data.deltaKind).toBe('reasoning')
  })

  it('一帧被拆到两个数据行时会拼回来', async () => {
    const s = controllableStream()
    stubFetch(() => s.response())
    const pending = streamChatMessage('S1', '你好', {})
    // axum 在值里遇到换行会多写一条 `data:` 行；按 SSE 规范要拼回去。
    s.send('event: delta\ndata: {"kind":"text",\ndata: "text":"多行"}\n\n')
    s.send(`event: done\ndata: ${JSON.stringify(DONE_BODY)}\n\n`)
    s.close()
    await expect(pending).resolves.toMatchObject({ reply: '最终答案。' })
  })

  it('最后一帧后面连换行都没有时，那一帧仍然算数', async () => {
    // 收尾处原来只 flushFrame（flush 的是已经攒好的 data），`buffer` 里剩下的
    // 半行从来没被处理过就丢了。流正好断在帧中间时，最后那句 `data:` 就这么没了
    // —— 下面「没拿到 done 就当断流」于是报出一个其实已经收到的答案。
    const s = controllableStream()
    stubFetch(() => s.response())
    const pending = streamChatMessage('S1', '你好', {})

    s.send(`event: done\ndata: ${JSON.stringify(DONE_BODY)}`) // 故意不给结尾换行
    s.close()

    await expect(pending).resolves.toMatchObject({ reply: '最终答案。' })
  })

  it('一个汉字被 TCP 切成两片时不会变成乱码', async () => {
    const s = controllableStream()
    stubFetch(() => s.response())
    const events: StreamEvent[] = []
    const pending = streamChatMessage('S1', '你好', { onEvent: (e) => events.push(e) })

    s.sendSplitInHalf('event: delta\ndata: {"kind":"text","text":"测"}\n\n')
    s.send(`event: done\ndata: ${JSON.stringify(DONE_BODY)}\n\n`)
    s.close()
    await pending

    const delta = events.find((e) => e.kind === 'delta')
    expect(delta?.kind === 'delta' && delta.data.text).toBe('测')
  })

  it('认不出的事件被丢帧，而不是让整轮失败', async () => {
    const s = controllableStream()
    stubFetch(() => s.response())
    const events: StreamEvent[] = []
    const pending = streamChatMessage('S1', '你好', { onEvent: (e) => events.push(e) })
    s.send('event: something_new\ndata: {"a":1}\n\n')
    s.send(`event: done\ndata: ${JSON.stringify(DONE_BODY)}\n\n`)
    s.close()
    await expect(pending).resolves.toMatchObject({ reply: '最终答案。' })
    expect(events).toHaveLength(1)
  })
})

describe('三种结束', () => {
  it('error 帧变成带「下一步」的 ApiError', async () => {
    const s = controllableStream()
    stubFetch(() => s.response())
    const pending = streamChatMessage('S1', '你好', {})
    s.send(
      'event: error\ndata: {"code":"provider_unavailable","detail":"连不上模型服务","next_step":"先确认端点活着"}\n\n',
    )
    s.close()
    await expect(pending).rejects.toMatchObject({
      code: 'provider_unavailable',
      nextStep: '先确认端点活着',
    })
  })

  it('连接断了而没有 done 时报错，绝不把半句话当成答案', async () => {
    const s = controllableStream()
    stubFetch(() => s.response())
    const events: StreamEvent[] = []
    const pending = streamChatMessage('S1', '你好', { onEvent: (e) => events.push(e) })
    s.send('event: delta\ndata: {"kind":"text","text":"说了半"}\n\n')
    s.close()
    const error = await pending.catch((e) => e)
    expect(error).toBeInstanceOf(ApiError)
    expect(error.code).toBe('stream_truncated')
    // 已经收到的那半句确实来过 —— 问题不在丢字，而在不能把它当答案交出去。
    expect(events.some((e) => e.kind === 'delta')).toBe(true)
  })

  it('响应不是 SSE 时报 4xx 信封，而不是当成空流', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        new Response(
          JSON.stringify({ error: { code: 'bad_request', detail: 'content 不能为空。', next_step: '填上 content' } }),
          { status: 400, headers: { 'content-type': 'application/json' } },
        ),
      ),
    )
    await expect(streamChatMessage('S1', '  ', {})).rejects.toMatchObject({
      status: 400,
      code: 'bad_request',
      nextStep: '填上 content',
    })
  })

  it('响应体不可读时说清楚是浏览器不支持，而不是静默返回空', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(null, { status: 200 })))
    await expect(streamChatMessage('S1', '你好', {})).rejects.toMatchObject({
      code: 'stream_unsupported',
    })
  })

  it('回的不是事件流时直接说，而不是抛一个 JSON.parse 语法错误', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(
        async () =>
          new Response('{"reply":"一次性返回"}', {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
      ),
    )
    const error = await streamChatMessage('S1', '你好', {}).catch((e) => e)
    expect(error).toBeInstanceOf(ApiError)
    expect(error.code).toBe('not_a_stream')
    expect(error.nextStep).toContain('messages/stream')
  })
})