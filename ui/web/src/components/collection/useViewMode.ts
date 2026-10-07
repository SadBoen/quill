import { useCallback, useState } from 'react'

/**
 * 一个条目集合的两种看法：**卡片**与**列表**。
 *
 * 抽出来是因为四个页面（专家、市场、MCP 服务、技能包、模型）都需要它，
 * 而在抽出来之前它们是各写各的 —— 专家页和市场页各有一份几乎逐行重复的
 * `useState` + localStorage + 两个按钮，其余三个连切换器都没有。
 * 抄一份的代价不是代码行数，是**同一个控件在五个地方长五种样子**。
 *
 * 为什么固定这两个值、不做成泛型：真正需要第三种看法（比如看板拖拽）的页面
 * 还没出现。届时该把这个函数扩成带 discriminant 的泛型，而不是提前留
 * 一个谁都没用的 `TMode` 参数。
 */
export type ViewMode = 'card' | 'list'

/**
 * 视图记忆的存储键。
 *
 * **必须给每一页一个自己的键**。共用一个键的话，用户在专家页切到列表，
 * 打开技能包发现也变列表了 —— 而这两个页面的行内容根本不同，
 * 用户会以为是 bug。`storageKey` 是必填参数正是为了堵这个。
 */
export function viewStorageKey(page: string): string {
  return `quill:${page}:view`
}

/**
 * 读视图偏好。
 *
 * localStorage 在隐私模式下会抛异常，而**存不下一个显示偏好不该让整页画不出来**，
 * 所以 catch 之后回到默认值继续。
 */
function loadViewMode(storageKey: string, fallback: ViewMode): ViewMode {
  try {
    const stored = localStorage.getItem(storageKey)
    return stored === 'card' || stored === 'list' ? stored : fallback
  } catch {
    return fallback
  }
}

export interface ViewModeControl {
  viewMode: ViewMode
  setViewMode: (mode: ViewMode) => void
  /** 列表的名字有歧义：专家页叫「表格」，市场页叫「列表」。这个布尔值两边都能用。 */
  showCardView: boolean
}

/**
 * 视图状态 + 记忆。
 *
 * @param page 页面标识（`'experts'` / `'skills'` …），只用来拼存储键。
 * @param defaultMode 没有记忆时用哪个。默认 `'card'`：
 * 卡片视图不需要滚动就能扫到标题，列表适合逐字段比较 —— 猜错了代价也很小，
 * 一键就能切。
 * @param storageKeyOverride 显式指定存储键。**只有一个场景该用**：同一页面的
 * 多个 tab 应当共享一个偏好。专家页的「我的专家」与「市场」就是这种关系 ——
 * 用户在一个 tab 选了列表，切到另一个不该又变回卡片，那看起来像切换丢了。
 * 除此之外一律走 `page`。
 */
export function useViewMode(
  page: string,
  defaultMode: ViewMode = 'card',
  storageKeyOverride?: string,
): ViewModeControl {
  const storageKey = storageKeyOverride ?? viewStorageKey(page)
  const [viewMode, setViewModeState] = useState<ViewMode>(() =>
    loadViewMode(storageKey, defaultMode),
  )

  const setViewMode = useCallback(
    (mode: ViewMode) => {
      setViewModeState(mode)
      try {
        localStorage.setItem(storageKey, mode)
      } catch {
        /* 这一次的切换已经生效了，不必因为存不下就报错打扰用户 */
      }
    },
    [storageKey],
  )

  return { viewMode, setViewMode, showCardView: viewMode === 'card' }
}