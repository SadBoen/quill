import { describe, expect, it } from 'vitest'
import { contextMismatch, type PoolGroup } from './ModelsPage'
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