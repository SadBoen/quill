import { useQuery } from '@tanstack/react-query'

import { loadSessionContext, type SessionContext } from './contextApi'

/**
 * 读一个会话的上下文窗口占用。**聊天页与用量统计页共用这一个出口**。
 *
 * 抽出来是因为两端开始长得不一样：用量页要整块图，聊天页只要一个 32px 的小环，
 * 两边各自去拉一次会出现「环是 2%、图是 3%」这种自相矛盾。
 */
export function useSessionContext(sessionId: string): {
  data: SessionContext | null
  error: unknown
  isError: boolean
} {
  const query = useQuery({
    queryKey: ['session-context', sessionId],
    queryFn: () => loadSessionContext(sessionId),
    enabled: Boolean(sessionId),
    staleTime: 10_000,
    retry: false,
  })
  return { data: query.data ?? null, error: query.error, isError: query.isError }
}