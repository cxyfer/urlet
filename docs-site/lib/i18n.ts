import { defineI18n } from 'fumadocs-core/i18n';

/**
 * Site languages, in switcher order.
 *
 * The names match the directory names the content sync writes under
 * `content/docs/`, which in turn mirror the repository's own layout: English at
 * the root of `docs/` and Traditional Chinese in `docs/zh-TW/`.
 */
export const LANGUAGES = ['en', 'zh-TW'] as const;

export type Language = (typeof LANGUAGES)[number];

export function isLanguage(value: string): value is Language {
  return (LANGUAGES as readonly string[]).includes(value);
}

/**
 * The locale prefix is never hidden. The site is a static export, and Next.js
 * does not run middleware in an export, so the rewrite `hideLocale` relies on
 * would never happen. Both languages therefore carry their prefix —
 * `/en/docs` and `/zh-TW/docs` — and `public/index.html` sends the bare root to
 * English.
 *
 * `parser` must be set explicitly: it defaults to `dot`, which reads the locale
 * from a file name suffix, while the content sync writes one directory per
 * language. With the default, the locale directory becomes part of the slug and
 * every page is generated twice under a doubled prefix.
 */
export const i18n = defineI18n({
  defaultLanguage: 'en',
  languages: [...LANGUAGES],
  hideLocale: 'never',
  parser: 'dir',
});
