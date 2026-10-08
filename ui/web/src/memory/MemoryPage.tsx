import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type FormEvent, type ReactNode, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ApiError } from '../api/client'
import { Card, ErrorNotice, PageHeader } from '../components/Page'
import { deletePage, listPages, readIndex, readLog, readPage, writePage } from './api'

const DEFAULT_PATH = 'index.md'

/** 页面不存在是空资料库的正常状态，不是故障。 */
function isNotFound(error: unknown): boolean {
  return error instanceof ApiError && error.status === 404
}

/**
 * 新建页面时的骨架。**不是装饰**：服务端在写盘前会校验内容能解析成合法页面
 * （第一行 `---`、frontmatter 必须闭合），给一个空文本框的话，用户第一次保存
 * 必然撞 400，而错在服务端而不是他。
 */
function newPageTemplate(today: string): string {
  return `---\ntitle: 新页面\ntype: concept\ncreated: ${today}\nupdated: ${today}\n---\n\n`
}

export function MemoryPage(): ReactNode {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [currentPath, setCurrentPath] = useState(DEFAULT_PATH)
  const [newPath, setNewPath] = useState('')
  // 正在编辑的草稿。**连它属于哪一页一起存**：切页时不需要再写一个 effect 去清它，
  // 只要 `draft.path !== activePath` 就当没在编辑（本文件上面那条 lint 的同一纪律：
  // 派生状态别放进 state 再由副作用纠正）。
  const [draft, setDraft] = useState<{ path: string; text: string } | null>(null)

  const list = useQuery({ queryKey: ['wiki-pages'], queryFn: listPages, staleTime: 15_000 })
  const index = useQuery({ queryKey: ['wiki-index'], queryFn: readIndex, staleTime: 30_000 })
  const log = useQuery({ queryKey: ['wiki-log'], queryFn: readLog, staleTime: 30_000 })

  const paths = useMemo(() => list.data?.pages ?? [], [list.data])

  // 当前该显示哪一页，是**算出来的**，不是存在 state 里再由 effect 纠正。
  //
  // 原来这里是 `useEffect(() => { setPaths(found); if (...) setCurrentPath(found[0]) }, ...)`，
  // 也就是把「算得出」的值先存一遍、再在副作用里改一遍：多一轮渲染，
  // 而且规则那条 lint（set-state-in-effect）说的正是这个 —— 派生状态放进
  // state，就会和它的来源各跑各的。这里改成纯推导，行为不变：
  // 列表非空且当前页不在其中时，仍然落到第一页；列表还没加载（空数组）
  // 时不动它，所以不会一上来就把用户填的路径顶掉。
  const activePath = paths.length > 0 && !paths.includes(currentPath) ? paths[0] : currentPath

  const content = useQuery({
    queryKey: ['wiki-page', activePath],
    queryFn: () => readPage(activePath),
    enabled: activePath.trim().length > 0,
  })

  const editing = draft !== null && draft.path === activePath

  // 保存：带上**这一页当前的 version**（新建时 content 还没数据 → null = 「必须还不存在」）。
  // 版本对不上服务端回 409，错误原样端给用户（见下面的 ErrorNotice），不静默重试 ——
  // 静默重试等于替用户覆盖掉别人刚写的那一版。
  const save = useMutation({
    mutationFn: (text: string) => writePage(activePath, text, content.data?.version ?? null),
    onSuccess: async () => {
      setDraft(null)
      await queryClient.invalidateQueries()
    },
  })
  const remove = useMutation({
    mutationFn: () => deletePage(activePath, content.data?.version ?? ''),
    onSuccess: async () => {
      setDraft(null)
      await queryClient.invalidateQueries()
    },
  })

  const submitNewPath = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault()
    if (newPath.trim()) setCurrentPath(newPath.trim())
  }

  return (
    <div className="page-scroll">
      <PageHeader
        title={t('memory.title', { defaultValue: '资料库' })}
        description={t('memory.description', { defaultValue: '浏览 quill 的 wiki 资料库页面。' })}
      />
      <div className="settings-stack">
        <ErrorNotice error={list.error ?? index.error ?? log.error} />
        {isNotFound(content.error) ? null : <ErrorNotice error={content.error} />}
        {/* 保存/删除的失败**必须**单独端出来：409（版本过期）的原话里有当前版本号，
            那是用户重试的唯一依据；混进上面那两条里会被 `??` 顺序挡掉。 */}
        <ErrorNotice error={save.error ?? remove.error} />
        <Card
          title={t('memory.notes', { defaultValue: '页面' })}
          description={t('memory.notesDescription', { defaultValue: '来自 GET /api/wiki/pages。' })}
        >
          <nav className="page-actions" aria-label={t('memory.notes', { defaultValue: '页面' })}>
            {paths.map((path) => (
              <button
                key={path}
                type="button"
                className="secondary-button"
                aria-current={activePath === path ? 'page' : undefined}
                onClick={() => setCurrentPath(path)}
              >
                {path}
              </button>
            ))}
          </nav>
          <form className="form-grid" onSubmit={submitNewPath}>
            <label className="full-row">
              {t('memory.newPath', { defaultValue: '页面路径' })}
              <input
                name="path"
                autoComplete="off"
                value={newPath}
                onChange={(event) => setNewPath(event.target.value)}
                placeholder="concepts/入门.md"
              />
            </label>
            <div className="form-actions full-row">
              <button type="submit" className="secondary-button" disabled={!newPath.trim()}>
                {t('memory.open', { defaultValue: '打开' })}
              </button>
            </div>
          </form>
        </Card>
        {content.isPending ? <p className="page-status">{t('common.loading', { defaultValue: '加载中…' })}</p> : null}
        {/* 编辑态也要画这张卡：新建一页时 `content.data` 还不存在，但草稿已经在手里了。 */}
        {content.data || editing ? (
          <Card title={activePath} description={content.data?.title}>
            <div className="form-grid">
              <label className="full-row">
                {t('memory.content', { defaultValue: '正文' })}
                <textarea
                  aria-label={t('memory.content', { defaultValue: '正文' })}
                  rows={20}
                  readOnly={!editing}
                  value={editing ? draft.text : (content.data?.content ?? '')}
                  onChange={(event) => setDraft({ path: activePath, text: event.target.value })}
                />
              </label>
              {content.data?.warnings?.length ? (
                <p className="field-help full-row" role="status">
                  {t('memory.warnings', { defaultValue: '解析告警' })}：{content.data.warnings.join('；')}
                </p>
              ) : null}
              <div className="form-actions full-row">
                {editing ? (
                  <>
                    <button
                      type="button"
                      className="primary-button"
                      disabled={save.isPending}
                      onClick={() => save.mutate(draft.text)}
                    >
                      {save.isPending
                        ? t('memory.saving', { defaultValue: '保存中…' })
                        : t('memory.save', { defaultValue: '保存' })}
                    </button>
                    <button type="button" className="secondary-button" onClick={() => setDraft(null)}>
                      {t('memory.cancel', { defaultValue: '取消' })}
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      className="primary-button"
                      onClick={() =>
                        setDraft({
                          path: activePath,
                          text:
                            content.data?.content ??
                            newPageTemplate(new Date().toISOString().slice(0, 10)),
                        })
                      }
                    >
                      {content.data
                        ? t('memory.edit', { defaultValue: '编辑' })
                        : t('memory.create', { defaultValue: '新建这一页' })}
                    </button>
                    {content.data ? (
                      <>
                        <button
                          type="button"
                          className="secondary-button"
                          onClick={() => void content.refetch()}
                        >
                          {t('memory.reload', { defaultValue: '重新载入' })}
                        </button>
                        {/* 删除不可逆，所以它带的是**这一页当前的版本号**：
                            别人在你打开之后改过，服务端会回 409 而不是删掉他那版。 */}
                        <button
                          type="button"
                          className="danger-button"
                          disabled={remove.isPending}
                          onClick={() => remove.mutate()}
                        >
                          {remove.isPending
                            ? t('memory.removing', { defaultValue: '删除中…' })
                            : t('memory.remove', { defaultValue: '删除这一页' })}
                        </button>
                      </>
                    ) : null}
                  </>
                )}
              </div>
            </div>
          </Card>
        ) : null}
        {isNotFound(content.error) && !editing ? (
          <Card
            title={activePath}
            description={t('memory.notFound', { defaultValue: '资料库里还没有这个页面。' })}
          >
            <p className="empty-card-copy">
              {t('memory.notFoundHint', {
                defaultValue: '在上面「页面路径」填一个路径再打开；不存在就点「新建这一页」。',
              })}
            </p>
            <div className="form-actions full-row">
              <button
                type="button"
                className="primary-button"
                onClick={() =>
                  setDraft({
                    path: activePath,
                    text: newPageTemplate(new Date().toISOString().slice(0, 10)),
                  })
                }
              >
                {t('memory.create', { defaultValue: '新建这一页' })}
              </button>
            </div>
          </Card>
        ) : null}
        <Card
          title={t('memory.indexTitle', { defaultValue: '索引' })}
          description={t('memory.indexDescription', { defaultValue: '来自 GET /api/wiki/index。' })}
        >
          {index.isPending ? <p className="page-status">{t('common.loading', { defaultValue: '加载中…' })}</p> : null}
          {index.data ? (
            index.data.present && index.data.index
              ? <textarea aria-label={t('memory.indexTitle', { defaultValue: '索引' })} rows={8} readOnly value={index.data.index} />
              : <p className="empty-card-copy">{t('memory.noIndex', { defaultValue: '资料库还没有索引页。' })}</p>
          ) : null}
        </Card>
        <Card
          title={t('memory.changeLog', { defaultValue: '变更日志' })}
          description={t('memory.changeLogDescription', { defaultValue: '来自 GET /api/wiki/log。' })}
        >
          {log.isPending ? <p className="page-status">{t('common.loading', { defaultValue: '加载中…' })}</p> : null}
          {log.data ? (
            log.data.present && log.data.log
              ? <textarea aria-label={t('memory.changeLog', { defaultValue: '变更日志' })} rows={8} readOnly value={log.data.log} />
              : <p className="empty-card-copy">{t('memory.noChangeLog', { defaultValue: '资料库还没有变更日志。' })}</p>
          ) : null}
        </Card>
      </div>
    </div>
  )
}
