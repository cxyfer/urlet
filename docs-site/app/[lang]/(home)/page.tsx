import Link from 'next/link';
import { appName, docsRoute } from '@/lib/shared';
import { LANGUAGES } from '@/lib/i18n';

const COPY = {
  en: {
    tagline: 'Documentation for urlet, a self-hosted URL shortener.',
    intro:
      'One Rust binary and one SQLite file. No external database, no cache, no frontend build step.',
    docs: 'Read the documentation',
    // The API reference is served by each instance at its own `/docs`; this
    // site cannot know that host, so it is described rather than linked.
    api: 'Every instance also serves its own API reference at /docs',
  },
  'zh-TW': {
    tagline: 'urlet 的文件——自架的多用戶短網址服務。',
    intro: '單一 Rust 執行檔與單一 SQLite 檔案。沒有外部資料庫、沒有快取、沒有前端建置步驟。',
    docs: '閱讀文件',
    api: '每個執行個體也在自己的 /docs 提供 API 參考',
  },
} as const;

export default async function HomePage({ params }: PageProps<'/[lang]'>) {
  const { lang } = await params;
  const copy = COPY[lang as keyof typeof COPY] ?? COPY.en;

  return (
    <div className="flex flex-col justify-center text-center flex-1 gap-4 px-6">
      <h1 className="text-3xl font-bold">{appName}</h1>
      <p className="text-fd-muted-foreground">{copy.tagline}</p>
      <p className="text-sm text-fd-muted-foreground">{copy.intro}</p>
      <p className="flex flex-wrap justify-center gap-4">
        <Link href={`/${lang}${docsRoute}`} className="font-medium underline">
          {copy.docs}
        </Link>
      </p>
      <p className="text-sm text-fd-muted-foreground">{copy.api}</p>
      <p className="text-xs text-fd-muted-foreground">
        {LANGUAGES.map((code, index) => (
          <span key={code}>
            {index > 0 && ' · '}
            <Link href={`/${code}`} className="underline">
              {code}
            </Link>
          </span>
        ))}
      </p>
    </div>
  );
}
