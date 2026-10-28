import { source } from '@/lib/source';
import { createFromSource } from 'fumadocs-core/search/server';

export const revalidate = false;

/**
 * The static search index.
 *
 * This route deliberately sits outside `app/[lang]/`. It is a route handler
 * rather than a page, and the client fetches one path for every locale —
 * `BASE_PATH + /api/search`. Under a locale segment it would be exported to
 * `/en/api/search` and `/zh-TW/api/search`, which nothing requests, and search
 * would fail on the published site.
 *
 * One index covers both languages: the default tokenizer is multilingual and
 * the loader carries the i18n config, so the exported document is the same for
 * every locale. Exporting it per locale only duplicated 1.6 MB of the export.
 */
export const { staticGET: GET } = createFromSource(source);
