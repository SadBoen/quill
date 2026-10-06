import { useQuery } from '@tanstack/react-query'
import { type FormEvent, type ReactNode, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ApiError } from '../api/client'
import { Card, ErrorNotice, PageHeader } from '../components/Page'
import { WIKI_WRITE_ROUTE, listPages, readIndex, readLog, readPage } from './api'

const DEFAULT_PATH = 'index.md'

/** 页面不存在是空资料库的正常状态，不是故障。 */
function isNotFound(error: unknown): boolean {
  return error instanceof ApiError && error.status === 404
}

export function MemoryPage(): ReactNode {
  const { t } = useTranslation()
  const [paths, setPaths] = useState<string[]>([])
  const [currentPath, setCurrentPath] = useState(DEFAULT_PATH)
  const [newPath, setNewPath] = useState('')

  const list = useQuery({ queryKey: ['wiki-pages'], queryFn: listPages, staleTime: 15_000 })
  const index = useQuery({ queryKey: ['wiki-index'], queryFn: readIndex, staleTime: 30_000 })
  const log = useQuery({ queryKey: ['wiki-log'], queryFn: readLog, staleTime: 30_000 })
  const content = useQuery({
    queryKey: ['wiki-page', currentPath],
    queryFn: () => readPage(currentPath),
    enabled: currentPath.trim().length > 0,
  })

  useEffect(() => {
    const found = list.data?.pages ?? []
    setPaths(found)
    if (found.length && !found.includes(currentPath)) setCurrentPath(found[0])
  }, [list.data, currentPath])

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
                aria-current={currentPath === path ? 'page' : undefined}
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
        {content.data ? (
          <Card title={content.data.path} description={content.data.title}>
            <div className="form-grid">
              <label className="full-row">
                {t('memory.content', { defaultValue: '正文' })}
                <textarea
                  aria-label={t('memory.content', { defaultValue: '正文' })}
                  rows={20}
                  readOnly
                  value={content.data.content}
                />
              </label>
              {content.data.warnings?.length ? (
                <p className="field-help full-row" role="status">
                  {t('memory.warnings', { defaultValue: '解析告警' })}：{content.data.warnings.join('；')}
                </p>
              ) : null}
              <div className="form-actions full-row">
                <button type="button" className="primary-button" onClick={() => void content.refetch()}>
                  {t('memory.reload', { defaultValue: '重新载入' })}
                </button>
              </div>
            </div>
          </Card>
        ) : null}
        {isNotFound(content.error) ? (
          <Card
            title={currentPath}
            description={t('memory.notFound', { defaultValue: '资料库里还没有这个页面。' })}
          >
            <p className="empty-card-copy">
              {t('memory.notFoundHint', {
                defaultValue: '在上面「页面路径」填一个存在的路径再打开，或先接入写入接口新建页面。',
              })}
            </p>
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
        <Card title={t('memory.editTitle', { defaultValue: '编辑资料库' })} tone="danger">
          <p className="field-help full-row">
            {t('memory.writeNotWired', {
              defaultValue: 'quill 后端目前只有资料库的只读路由，写入接口尚未接通，因此本页不能保存或删除页面。下一步：实现 {{route}}，并把乐观并发用的 expected_version 一起做出来。',
              route: WIKI_WRITE_ROUTE,
            })}
          </p>
        </Card>
      </div>
    </div>
  )
}
