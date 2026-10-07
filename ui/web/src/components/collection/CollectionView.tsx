import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import type { ViewMode } from './useViewMode'
import './collection.css'

/**
 * 卡片 / 列表 的切换器。
 *
 * 抽出来是因为专家页和市场页各有一份几乎逐行相同的两个按钮，而 MCP 服务、
 * 技能包、模型三个页面一个都没有 —— 同一个控件，五种存在方式。
 *
 * **只有一种看法时不要渲染它。** 模型页现在两种都有；如果某个页面永远只有
 * 卡片，画一个点不动的切换器比不画更糟。
 */
export function ViewToggle({
  viewMode,
  onChange,
  cardLabel,
  listLabel,
  testIdPrefix,
}: {
  viewMode: ViewMode
  onChange: (mode: ViewMode) => void
  /** 「列表」在不同页面的叫法不同：专家页叫「表格」，其余叫「列表」。 */
  cardLabel?: string
  listLabel?: string
  /** 给测试用的前缀。五个页面各有一组，必须能分开定位。 */
  testIdPrefix?: string
}): ReactNode {
  const { t } = useTranslation()
  return (
    <span className="collection-view-toggle" role="group">
      <button
        type="button"
        aria-pressed={viewMode === 'card'}
        className="collection-view-toggle-item"
        data-testid={testIdPrefix ? `${testIdPrefix}-card` : undefined}
        onClick={() => onChange('card')}
      >
        {cardLabel ?? t('collection.viewCard', { defaultValue: '卡片' })}
      </button>
      <button
        type="button"
        aria-pressed={viewMode === 'list'}
        className="collection-view-toggle-item"
        data-testid={testIdPrefix ? `${testIdPrefix}-list` : undefined}
        onClick={() => onChange('list')}
      >
        {listLabel ?? t('collection.viewList', { defaultValue: '列表' })}
      </button>
    </span>
  )
}

/**
 * 卡片 / 列表 的布局壳。
 *
 * 只管**外层**——间距、栅格、窄屏塌缩。卡片里放什么、行里放什么，由各页面
 * 自己决定，因为那正是各页面本来就该不一样的部分。
 *
 * 栅格在这里定死，是因为「卡片在五个页面长得一样」是抽这一层的全部意义：
 * 间距与列宽如果各页自己写，五张卡片摆在一起会宽窄不一，而用户一眼就看出来
 * 那不是同一个系统。
 *
 * @param items 条目。已经过滤好（搜索、分页都算调用方的活）。
 * @param renderCard 卡片内容。
 * @param renderList 列表行内容。
 * @param cardKey 每个条目的稳定 key —— 用 slug/id，别用数组下标。
 */
export function CollectionView<T>({
  viewMode,
  items,
  renderCard,
  renderList,
  cardKey,
  empty = null,
}: {
  viewMode: ViewMode
  items: readonly T[]
  renderCard: (item: T) => ReactNode
  renderList: (item: T) => ReactNode
  cardKey: (item: T) => string
  empty?: ReactNode
}): ReactNode {
  // 空集合时不渲染容器：留一个空壳在 DOM 里，读屏软件会念出「列表，0 项」，
  // 而界面上写的是「还没有安装任何技能包」—— 两句话在描述同一个空状态。
  if (items.length === 0) return <>{empty}</>

  if (viewMode === 'card') {
    return (
      <ul className="collection-grid">
        {items.map((item) => (
          <li className="collection-grid-item" key={cardKey(item)}>
            {renderCard(item)}
          </li>
        ))}
      </ul>
    )
  }

  return (
    <ul className="collection-list">
      {items.map((item) => (
        <li className="collection-list-item" key={cardKey(item)}>
          {renderList(item)}
        </li>
      ))}
    </ul>
  )
}