import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { ModelsPage, contextMismatch, type PoolGroup } from './ModelsPage'
import type { PoolEntry } from './api'

// 三个数都取自 2026-10-06 的真机实测（ISSUE-023）：
// 配置 max_context_tokens = 32768，llama.cpp 的 meta.n_ctx = 8192。
const CONFIGURED = 32768
const PROBED = 8192

function entry(contextWindow: number | null): PoolEntry {
  return {
    provider_id: 'p1',
    provider_name: '本机 llama.cpp',
    model: {
      id: 'Qwen3.5-4B-Q4_K_M.gguf',
      display_name: 'Qwen3.5-4B-Q4_K_M',
      modality: 'text',
      context_window: contextWindow,
      owned_by: 'llamacpp',
    },
    starred: true,
  }
}

function group(configuredContext: number | null, entries: PoolEntry[]): PoolGroup {
  return { providerId: 'p1', name: '本机 llama.cpp', protocol: 'openai', entries, configuredContext }
}

describe('contextMismatch', () => {
  it('报出「配的比能吞的大」——本机实测差 4 倍', () => {
    expect(contextMismatch(group(CONFIGURED, [entry(PROBED)]))).toBe(PROBED)
  })

  it('探测值缺失时不报：那是「不知道」，不是「一致」', () => {
    // 把 null 当成「没问题」是另一种谎：上游没报窗口 ≠ 配置没问题。
    expect(contextMismatch(group(CONFIGURED, [entry(null)]))).toBeNull()
  })

  it('配置值没填或拿不到时不报', () => {
    expect(contextMismatch(group(null, [entry(PROBED)]))).toBeNull()
    expect(contextMismatch(group(0, [entry(PROBED)]))).toBeNull()
  })

  it('配置值等于探测值时不报', () => {
    expect(contextMismatch(group(PROBED, [entry(PROBED)]))).toBeNull()
  })

  it('配置值比探测值**小**时不报：白占窗口不会让请求被拒，报出来是噪音', () => {
    expect(contextMismatch(group(4096, [entry(PROBED)]))).toBeNull()
  })

  it('一组里有任何一个模型对不上就报，并给出那个更小的探测值', () => {
    const g = group(CONFIGURED, [entry(CONFIGURED), entry(PROBED), entry(null)])
    expect(contextMismatch(g)).toBe(PROBED)
  })

  it('探测值为 0 视为没报，不当成「零窗口」', () => {
    expect(contextMismatch(group(CONFIGURED, [entry(0)]))).toBeNull()
  })
})

describe('压缩阈值输入框', () => {
  beforeEach(async () => {
    // 不切语言的话，按钮名是英文 "Edit"，下面的按名查找会扑空 —— 那是断言写错，
    // 不是页面坏了。
    await i18n.changeLanguage('zh-CN')
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('明写「已生效」与口径：这个数真的会被读，数字是估算值', async () => {
    // 审计结论（2026-10-06，全仓库 grep）：compaction_threshold_tokens 只出现在
    // 迁移、增删改查与测试里，**没有任何运行时读者**。所以它是一个能编辑却不生效
    // 的开关 —— 界面上不给标注，用户就会在这里改一个不动的数字，然后发现对话该超还是超。
    const provider = {
      id: 'p1',
      name: '本机',
      kind: 'custom',
      protocol: 'openai',
      base_url: 'http://127.0.0.1:18080/v1',
      has_api_key: false,
      model: 'local',
      max_context_tokens: 32768,
      compaction_threshold_tokens: 8000,
      max_output_tokens: 2048,
      enabled: true,
      is_default: true,
      created_at: 0,
      updated_at: 0,
    }
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      const body = url.includes('/providers')
        ? { providers: [provider] }
        : { default_provider_id: 'p1', pool: [], unavailable: [], disabled: [] }
      return new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    }))

    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <MemoryRouter>
        <QueryClientProvider client={client}>
          <ModelsPage />
        </QueryClientProvider>
      </MemoryRouter>,
    )

    // 自定义端点那张卡默认是收起的：先展开，否则断言的是「找不到」，不是「没标注」。
    fireEvent.click(await screen.findByRole('button', { name: '编辑' }))
    await waitFor(() => expect(screen.getByDisplayValue('8000')).toBeInTheDocument())
    // 按 testid 定位，不用文案：文案一改测试就红，而这里要守的是
    // 「这个输入框旁边必须有一句说明」，不是那一句具体怎么措辞。
    const note = screen.getByTestId('models-compaction-note')
    // testid 挂在输入框上会让人以为那是输入框的标识；守的是标注本身。
    expect(note.tagName).toBe('SMALL')
    // Q018：压缩本体已接线，标注必须改成「已生效」并说清口径（估算值）。
    // 文案与实现同批改，不让界面继续宣称「没有任何逻辑会读它」。
    expect(note).toHaveTextContent(/已生效/)
    expect(note).toHaveTextContent(/估算/)

    // 变异验证：把下面这行去掉，测试必须变红。
    //（已验证会红：删掉 <small> 那整段后，getByTestId 找不到元素。）
  })
})