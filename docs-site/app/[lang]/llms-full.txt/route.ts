import { docsLlms } from '@/lib/source';
import { LANGUAGES } from '@/lib/i18n';

export const revalidate = false;

export function generateStaticParams() {
  return LANGUAGES.map((lang) => ({ lang }));
}

export async function GET(
  _req: Request,
  { params }: RouteContext<'/[lang]/llms-full.txt'>,
) {
  const { lang } = await params;

  return new Response(await docsLlms.full(lang));
}
