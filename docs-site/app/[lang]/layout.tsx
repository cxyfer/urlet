import { notFound } from 'next/navigation';
import { I18nProvider } from 'fumadocs-ui/contexts/i18n';
import { Provider } from '@/components/provider';
import { LANGUAGES, isLanguage } from '@/lib/i18n';
import { i18nUI } from '@/lib/i18n-ui';
import './global.css';

/**
 * The root layout, one per language.
 *
 * It lives under `[lang]` rather than at the root because the `lang` attribute
 * and the interface strings both depend on the locale, and Next.js allows only
 * one root layout to render `<html>`.
 *
 * No web font is loaded: the template pulled Inter from Google Fonts at build
 * time, which would make the build depend on an external service and contradict
 * how the binary serves its own reference. The stylesheet falls back to the
 * system stack, which is what `withDefaultFonts: false` does for Scalar.
 */
export function generateStaticParams() {
  return LANGUAGES.map((lang) => ({ lang }));
}

export default async function Layout({
  params,
  children,
}: LayoutProps<'/[lang]'>) {
  const { lang } = await params;
  if (!isLanguage(lang)) notFound();

  return (
    <html lang={lang} suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <Provider i18n={i18nUI.provider(lang)}>{children}</Provider>
      </body>
    </html>
  );
}
