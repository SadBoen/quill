import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'

import { en, zhCN } from './resources'

export const LANGUAGE_OPTIONS = [
  { code: 'en', label: 'English', resource: en },
  { code: 'zh-CN', label: '简体中文', resource: zhCN },
] as const

export type AppLanguage = (typeof LANGUAGE_OPTIONS)[number]['code']
export const LANGUAGE_STORAGE_KEY = 'quill-language'

export function normalizeLanguage(language: string | null | undefined): AppLanguage {
  return LANGUAGE_OPTIONS.find(({ code }) => code === language)?.code ?? 'zh-CN'
}

function initialLanguage(): AppLanguage {
  if (typeof window === 'undefined') return 'zh-CN'
  try {
    return normalizeLanguage(window.localStorage.getItem(LANGUAGE_STORAGE_KEY))
  } catch {
    return 'zh-CN'
  }
}

const startingLanguage = initialLanguage()

void i18n.use(initReactI18next).init({
  resources: Object.fromEntries(LANGUAGE_OPTIONS.map(({ code, resource }) => [code, resource])),
  lng: startingLanguage,
  fallbackLng: 'zh-CN',
  supportedLngs: LANGUAGE_OPTIONS.map(({ code }) => code),
  interpolation: { escapeValue: false },
  returnNull: false,
})

i18n.on('languageChanged', (language) => {
  const selected = normalizeLanguage(language)
  if (typeof document !== 'undefined') document.documentElement.lang = selected
  if (typeof window !== 'undefined') {
    try {
      window.localStorage.setItem(LANGUAGE_STORAGE_KEY, selected)
    } catch {
      // A blocked storage API should not block rendering.
    }
  }
})

if (typeof document !== 'undefined') document.documentElement.lang = startingLanguage

export default i18n
