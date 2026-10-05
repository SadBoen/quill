import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import type { Expert } from '../experts/api'
import type { Session } from '../api/types'
import { Popconfirm } from '../experts/ExpertsUi'
import { expertInitial, expertTone } from './expertAvatar'

export interface ChatSidebarProps {
  /** 只传 default_enabled 为 true 的专家。 */
  experts: Expert[]
  expertsPending: boolean
  sessions: Session[]
  sessionsPending: boolean
  /** 当前选中的角色 id；空串 = 默认（没有角色的会话）。 */
  activeExpertId: string
  activeSessionId: string | null
  /** 窄屏抽屉是否展开。 */
  open: boolean
  onSelectExpert: (expertId: string) => void
  onOpenSession: (sessionId: string) => void
  onDeleteSession: (sessionId: string) => void
  /** 正在删除的会话 id；用于把该行的确认按钮置为等待。 */
  deletingId: string | null
  onNewChat: () => void
  onClose: () => void
}

function SessionGroup({
  title,
  sessions,
  activeSessionId,
  onOpenSession,
  onDeleteSession,
  deletingId,
  emptyText,
}: {
  title: string
  sessions: Session[]
  activeSessionId: string | null
  onOpenSession: (sessionId: string) => void
  onDeleteSession: (sessionId: string) => void
  deletingId: string | null
  emptyText: string
}): ReactNode {
  const { t, i18n } = useTranslation()
  return (
    <section className="chat-sidebar-group">
      <p className="chat-sidebar-group-title">{title}</p>
      {sessions.length ? (
        <ul className="chat-sidebar-sessions">
          {sessions.map((session) => (
            <li key={session.id}>
              <div className={`chat-session-item${session.id === activeSessionId ? ' is-active' : ''}`}>
                <button
                  type="button"
                  className="chat-session-open"
                  onClick={() => onOpenSession(session.id)}
                  title={session.title || t('nav.newChat', { defaultValue: '新对话' })}
                >
                  <span className="chat-session-name">{session.title || t('nav.newChat', { defaultValue: '新对话' })}</span>
                  <small className="chat-session-sub">
                    {new Intl.DateTimeFormat(i18n.resolvedLanguage || 'zh-CN', {
                      month: 'short',
                      day: 'numeric',
                    }).format(new Date(session.last_active_at))}
                    {' · '}
                    {session.message_count} {t('nav.messages', { defaultValue: '条' })}
                  </small>
                </button>
                <Popconfirm
                  className="chat-session-action"
                  label={t('chat.sidebar.deleteSession', { defaultValue: '删除会话' })}
                  icon={<span aria-hidden="true">✕</span>}
                  title={t('chat.deleteConfirmTitle', { defaultValue: '删除这个会话？' })}
                  description={t('chat.deleteConfirmHint', {
                    defaultValue: '这是软删除：会话会从列表里消失，消息记录仍保留在本机数据库中。',
                  })}
                  okText={t('common.delete', { defaultValue: '删除' })}
                  cancelText={t('common.cancel', { defaultValue: '取消' })}
                  dangerOk
                  pending={deletingId === session.id}
                  onConfirm={() => onDeleteSession(session.id)}
                />
              </div>
            </li>
          ))}
        </ul>
      ) : (
        <p className="chat-sidebar-empty">{emptyText}</p>
      )}
    </section>
  )
}

/**
 * 第二侧栏（对齐 Octop 的 chatSidebar.partial.less）：
 * 上半是角色（专家）列表，下半是当前角色的会话记录。
 * 角色维度来自 listExperts() 且只保留 default_enabled 为 true 的。
 */
export function ChatSidebar({
  experts,
  expertsPending,
  sessions,
  sessionsPending,
  activeExpertId,
  activeSessionId,
  open,
  onSelectExpert,
  onOpenSession,
  onDeleteSession,
  deletingId,
  onNewChat,
  onClose,
}: ChatSidebarProps): ReactNode {
  const { t } = useTranslation()
  const defaultSessions = sessions.filter((session) => !session.expert_id)
  const expertSessions = activeExpertId ? sessions.filter((session) => session.expert_id === activeExpertId) : []
  const activeExpert = experts.find((expert) => expert.id === activeExpertId)

  return (
    <>
      <button
        type="button"
        className={`chat-sidebar-scrim${open ? ' is-open' : ''}`}
        tabIndex={open ? 0 : -1}
        aria-hidden={!open}
        onClick={onClose}
      />
      <aside
        className={`chat-second-sidebar${open ? ' is-open' : ''}`}
        aria-label={t('chat.sidebar.label', { defaultValue: '角色与会话' })}
      >
        <header className="chat-sidebar-head">
          <strong className="chat-sidebar-head-title">
            {t('chat.sidebar.title', { defaultValue: '角色与会话' })}
          </strong>
          <button
            type="button"
            className="chat-sidebar-close"
            aria-label={t('chat.sidebar.close', { defaultValue: '收起侧栏' })}
            title={t('chat.sidebar.close', { defaultValue: '收起侧栏' })}
            onClick={onClose}
          >
            ✕
          </button>
        </header>

        <div className="chat-sidebar-scroll">
          <section className="chat-sidebar-group">
            <p className="chat-sidebar-group-title">{t('chat.sidebar.experts', { defaultValue: '角色' })}</p>
            {expertsPending ? (
              <p className="chat-sidebar-empty">{t('common.loading', { defaultValue: '加载中…' })}</p>
            ) : experts.length ? (
              <ul className="chat-expert-list">
                {experts.map((expert) => {
                  const count = sessions.filter((session) => session.expert_id === expert.id).length
                  return (
                    <li key={expert.id}>
                      <button
                        type="button"
                        className={`chat-expert-item${expert.id === activeExpertId ? ' is-active' : ''}`}
                        onClick={() => onSelectExpert(expert.id)}
                        aria-current={expert.id === activeExpertId}
                      >
                        <span className="chat-expert-avatar" data-tone={expertTone(expert.id)} aria-hidden="true">
                          {expertInitial(expert.id)}
                        </span>
                        <span className="chat-expert-name">{expert.display_name || expert.id}</span>
                        <small className="chat-expert-count">
                          {t('chat.sidebar.sessionCount', { count, defaultValue: '{{count}}' })}
                        </small>
                      </button>
                    </li>
                  )
                })}
              </ul>
            ) : (
              <p className="chat-sidebar-empty">
                {t('chat.sidebar.noExperts', { defaultValue: '还没有「默认启用」的角色。' })}
              </p>
            )}
            <button type="button" className="chat-sidebar-add" onClick={onNewChat}>
              <span aria-hidden="true">＋</span> {t('chat.sidebar.newSession', { defaultValue: '新建会话' })}
            </button>
          </section>

          {sessionsPending ? (
            <p className="chat-sidebar-empty">{t('common.loading', { defaultValue: '加载中…' })}</p>
          ) : (
            <>
              {activeExpert ? (
                <SessionGroup
                  title={activeExpert.display_name || activeExpert.id}
                  sessions={expertSessions}
                  activeSessionId={activeSessionId}
                  onOpenSession={onOpenSession}
                  onDeleteSession={onDeleteSession}
                  deletingId={deletingId}
                  emptyText={t('chat.sidebar.noSessions', { defaultValue: '这个角色还没有会话。' })}
                />
              ) : null}
              <SessionGroup
                title={t('chat.sidebar.defaultGroup', { defaultValue: '默认（未选角色）' })}
                sessions={defaultSessions}
                activeSessionId={activeSessionId}
                onOpenSession={onOpenSession}
                onDeleteSession={onDeleteSession}
                deletingId={deletingId}
                emptyText={
                  activeExpert
                    ? t('chat.sidebar.noDefaultSessions', { defaultValue: '没有未指定角色的会话。' })
                    : sessions.length
                      ? t('chat.sidebar.pickExpert', { defaultValue: '上面选一个角色，看它的会话记录。' })
                      : t('chat.sidebar.emptyAll', { defaultValue: '还没有会话，先新建一个。' })
                }
              />
            </>
          )}
        </div>
      </aside>
    </>
  )
}
