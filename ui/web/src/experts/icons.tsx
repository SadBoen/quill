import type { ReactNode } from 'react'

/**
 * Octop 用 lucide-react，quill 不引新依赖，这里手写同款描边图标
 * （同一套 24 网格、stroke=currentColor、无填充），只借形态不引包。
 */
function StrokeIcon({ size, children }: { size: number; children: ReactNode }): ReactNode {
  return (
    <svg
      aria-hidden="true"
      focusable="false"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {children}
    </svg>
  )
}

export function IconSearch({ size = 14 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.6-3.6" />
    </StrokeIcon>
  )
}

export function IconClose({ size = 13 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </StrokeIcon>
  )
}

export function IconRefresh({ size = 14 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
      <path d="M21 3v5h-5" />
      <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
      <path d="M8 16H3v5" />
    </StrokeIcon>
  )
}

export function IconPlus({ size = 14 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </StrokeIcon>
  )
}

export function IconLayoutGrid({ size = 14 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
    </StrokeIcon>
  )
}

export function IconList({ size = 14 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <path d="M8 6h13" />
      <path d="M8 12h13" />
      <path d="M8 18h13" />
      <path d="M3 6h.01" />
      <path d="M3 12h.01" />
      <path d="M3 18h.01" />
    </StrokeIcon>
  )
}

export function IconPencil({ size = 13 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <path d="M21.17 6.83a2.83 2.83 0 0 0-4-4L3 17v4h4Z" />
      <path d="m15 5 4 4" />
    </StrokeIcon>
  )
}

export function IconTrash({ size = 13 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <path d="M3 6h18" />
      <path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
      <path d="M10 11v6" />
      <path d="M14 11v6" />
    </StrokeIcon>
  )
}

export function IconCopy({ size = 11 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </StrokeIcon>
  )
}

export function IconCheck({ size = 11 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <path d="M20 6 9 17l-5-5" />
    </StrokeIcon>
  )
}

/** Octop 团队徽标用 lucide Users（TeamCard.tsx:231），这里是同款描边。 */
export function IconUsers({ size = 13 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" />
      <circle cx="9" cy="7" r="4" />
      <path d="M22 21v-2a4 4 0 0 0-3-3.87" />
      <path d="M16 3.13a4 4 0 0 1 0 7.75" />
    </StrokeIcon>
  )
}

/** Octop 技能市场卡片在没有 `icon_url` 时用 lucide Zap 占位
 *  （`SkillHubTab.tsx` 的 `hubCardIconFallback`）。这里是同款描边。 */
export function IconZap({ size = 16 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <path d="M4 14h6l-2 8 10-12h-6l2-8z" />
    </StrokeIcon>
  )
}

/** Octop 技能市场卡片左下角的下载数用 lucide Download 搭配
 *  （`SkillHubTab.tsx` 的 `hubCardStat`）。这里是同款描边。 */
export function IconDownload({ size = 14 }: { size?: number }): ReactNode {
  return (
    <StrokeIcon size={size}>
      <path d="M12 15V3" />
      <path d="M7 10l5 5 5-5" />
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    </StrokeIcon>
  )
}
