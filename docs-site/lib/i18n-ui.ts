import { defineI18nUI } from 'fumadocs-ui/i18n';
import { i18n } from './i18n';

/**
 * Interface strings for the two languages.
 *
 * Fumadocs ships no Traditional Chinese catalogue, so these are written here —
 * the same approach the reference site this layout follows takes. Only the
 * strings the framework renders around the content are listed: page titles,
 * summaries and body text all come from the repository's own bilingual
 * documents through the content sync.
 */
export const i18nUI = defineI18nUI(i18n, {
  en: {
    displayName: 'English',
    search: 'Search',
    searchNoResult: 'No results',
    toc: 'On this page',
    tocNoHeadings: 'No headings',
    lastUpdate: 'Last updated',
    chooseTheme: 'Theme',
    chooseLanguage: 'Language',
    nextPage: 'Next',
    previousPage: 'Previous',
    editOnGithub: 'Edit on GitHub',
  },
  'zh-TW': {
    displayName: '繁體中文',
    search: '搜尋文件',
    searchNoResult: '沒有符合的結果',
    toc: '本頁目錄',
    tocNoHeadings: '沒有標題',
    lastUpdate: '最後更新',
    chooseTheme: '主題',
    chooseLanguage: '語言',
    nextPage: '下一頁',
    previousPage: '上一頁',
    editOnGithub: '在 GitHub 編輯',
  },
});
