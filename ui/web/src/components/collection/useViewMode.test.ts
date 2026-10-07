import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import { useViewMode, viewStorageKey } from './useViewMode'

/**
 * 视图状态的判据。
 *
 * 抽这一层的理由是「五个页面各写各的，同一个控件五种样子」。所以这里盯的
 * 不是「能不能切」，而是**切完之后记不记得、记到哪一页名下** —— 后者才是
 * 抄一份时最容易出错、错了最难发现的地方。
 */

beforeEach(() => {
  localStorage.clear()
})

describe('useViewMode', () => {
  it('默认是卡片', () => {
    const { result } = renderHook(() => useViewMode('skills'))
    expect(result.current.viewMode).toBe('card')
    expect(result.current.showCardView).toBe(true)
  })

  it('切到列表后 viewMode 与 showCardView 一起跟着变', () => {
    const { result } = renderHook(() => useViewMode('skills'))
    act(() => result.current.setViewMode('list'))
    expect(result.current.viewMode).toBe('list')
    expect(result.current.showCardView).toBe(false)
  })

  it('记住上一次的选择：重新挂载后还是列表', () => {
    const first = renderHook(() => useViewMode('skills'))
    act(() => first.result.current.setViewMode('list'))
    first.unmount()

    const second = renderHook(() => useViewMode('skills'))
    expect(second.result.current.viewMode).toBe('list')
  })

  /**
   * 这条是这一层最要紧的判据。
   *
   * 五页共用一套 hook 时最容易出的错就是存储键写死成一个，于是用户在专家页
   * 切到列表，打开技能包发现也变列表了 —— 而两个页面的行内容根本不同，
   * 用户只会当成 bug 报上来。键必须带页面名。
   */
  it('各页各记各的：专家页的偏好不会带到技能包', () => {
    const experts = renderHook(() => useViewMode('experts'))
    act(() => experts.result.current.setViewMode('list'))

    const skills = renderHook(() => useViewMode('skills'))
    expect(skills.result.current.viewMode).toBe('card')
    expect(localStorage.getItem(viewStorageKey('experts'))).toBe('list')
    expect(localStorage.getItem(viewStorageKey('skills'))).toBeNull()
  })

  it('键带页面名，且不再沿用 octop 的旧键', () => {
    // 旧键是硬编码在专家页里的 `octop:experts-view`。留着它会让老用户的选择
    // 丢失，而那属于「迁移」，不该混在这次重构里悄悄发生。
    expect(viewStorageKey('experts')).toBe('quill:experts:view')
  })

  it('默认模式可以按页指定', () => {
    // 列表页多的页面（例如 MCP）用列表当默认更顺手，不用每次进来先点一下。
    const { result } = renderHook(() => useViewMode('devices', 'list'))
    expect(result.current.viewMode).toBe('list')
  })

  it('存下的值不认就回默认，不当视图用', () => {
    // localStorage 是别人也能写的。存进去一个 'grid' 就当成合法值，
    // 界面上会掉进一个谁都没定义的分支。
    localStorage.setItem(viewStorageKey('skills'), 'grid')
    const { result } = renderHook(() => useViewMode('skills'))
    expect(result.current.viewMode).toBe('card')
  })

  it('隐私模式下存不下也不影响显示', () => {
    const original = Object.getOwnPropertyDescriptor(window, 'localStorage')
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      get() {
        throw new DOMException('denied', 'SecurityError')
      },
    })
    try {
      const { result } = renderHook(() => useViewMode('skills'))
      // 存不下一个显示偏好不该让整页画不出来。
      expect(result.current.viewMode).toBe('card')
      act(() => result.current.setViewMode('list'))
      expect(result.current.viewMode).toBe('list')
    } finally {
      if (original) Object.defineProperty(window, 'localStorage', original)
    }
  })
})