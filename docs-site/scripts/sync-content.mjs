#!/usr/bin/env node
/**
 * Copy the repository's bilingual documentation into the site's content tree.
 *
 * `docs/` is the single source of truth and stays plain Markdown: it is read by
 * people on GitHub, mirrored in two languages, and referenced from the SPEC. The
 * site has two requirements that tree does not satisfy on its own:
 *
 *   * Fumadocs resolves a locale by directory, so every language needs its own
 *     folder. The repository keeps English at the root of `docs/` and
 *     Traditional Chinese in `docs/zh-TW/`.
 *   * Fumadocs requires frontmatter on every page, and adding it to thirty
 *     repository files would be churn for the sake of a build artifact.
 *
 * Both are solved here instead of in the source: this script derives each page
 * title from its own first heading and writes the per-locale directories. The
 * output is generated on every build, so it cannot drift, and it is gitignored
 * so the repository never carries a second copy of its own documentation.
 *
 * Only the documents named in PUBLISHED are written. The specification, the
 * plan, the release contract and the `aegis/` records are internal: they stay
 * out of the published site by being absent from that list, not by a filter
 * someone can forget to update.
 */

import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const SITE = resolve(HERE, "..");
const REPO = resolve(SITE, "..");
const SOURCE = join(REPO, "docs");
const TARGET = join(SITE, "content", "docs");

/**
 * Locale directory name in the site, the repository directory it reads, and the
 * URL prefix its pages live under.
 *
 * The prefix carries the locale for **both** languages. `hideLocale` is `never`
 * — a static export cannot run the middleware that would hide it — so an English
 * page is at `/en/docs/...` just as a Chinese one is at `/zh-TW/docs/...`. A
 * prefix that omitted `/en` would send every rewritten cross-document link on an
 * English page to a path that does not exist.
 */
const LOCALES = [
  { locale: "en", source: SOURCE, prefix: "/en/docs" },
  { locale: "zh-TW", source: join(SOURCE, "zh-TW"), prefix: "/zh-TW/docs" },
];

/**
 * Documents to publish, in sidebar order, keyed by their file name in `docs/`.
 *
 * The slug is the site route below the locale prefix. Anything not listed here
 * is internal and is not published.
 */
const PUBLISHED = [
  { file: "API.md", slug: "api" },
  { file: "DEPLOYMENT.md", slug: "deployment" },
  { file: "THEMES.md", slug: "themes" },
  { file: "MULTI_DOMAIN.md", slug: "multi-domain" },
];

/** Section titles for the generated sidebar. */
const SECTION = { en: "Documentation", "zh-TW": "文件" };

/**
 * The generated documentation index, one per locale.
 *
 * `meta.json` lists the documents for the sidebar, but nothing renders a page at
 * the section root, so `/<locale>/docs/` — which the site's own home page links
 * to — answered 404. The lead sentence is the only prose this script authors;
 * the list below it is built from the titles the documents themselves carry.
 */
const INDEX = {
  en: {
    title: "Documentation",
    lead: "urlet is a self-hosted URL shortener: one Rust binary and one SQLite file, with no external database, cache or frontend build step.",
  },
  "zh-TW": {
    title: "文件",
    lead: "urlet 是自架的多用戶短網址服務：單一 Rust 執行檔與單一 SQLite 檔案，沒有外部資料庫、快取或前端建置步驟。",
  },
};

/**
 * The language switcher line each document carries at the top.
 *
 * The site has its own switcher in the layout, so the in-document one would be
 * a second, inconsistent control pointing at repository paths.
 */
const SWITCHER = /^\s*(\*\*English\*\*|\*\*繁體中文\*\*|\[English\]\(\.\.\/).*$/;

/**
 * Links between repository documents, rewritten to site routes.
 *
 * The keys are the link targets as they appear in the source; the values are
 * built per locale because the prefix differs. A target mapped to `null` has no
 * equivalent page on the site: the link is unwrapped to its own text rather
 * than left pointing at a repository path that does not exist here.
 */
const LINK_TARGETS = {
  "THEMES.md": (prefix) => `${prefix}/themes`,
  "API.md": (prefix) => `${prefix}/api`,
  "DEPLOYMENT.md": (prefix) => `${prefix}/deployment`,
  "MULTI_DOMAIN.md": (prefix) => `${prefix}/multi-domain`,
  // The contract lives in the repository, not on the site; the served reference
  // at `/docs` is the site's own entry point to it.
  "../openapi": () => null,
};

/** Targets that need no rewriting: absolute URLs, anchors and site paths. */
const EXTERNAL = /^(https?:|mailto:|#|\/)/;

function rewriteLinks(body, prefix, file) {
  return body.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (whole, text, target) => {
    // A fragment belongs to the document, not to the mapping.
    const bare = target.split("#")[0];
    if (bare === "") {
      return whole;
    }
    const rewrite = LINK_TARGETS[bare];
    if (!rewrite) {
      if (EXTERNAL.test(bare)) {
        return whole;
      }
      // A relative link the mapping does not know about would be emitted
      // verbatim and 404 on the site, with nothing to notice it. A new document
      // linked from a published one lands here, so fail the build instead and
      // say which link and which file.
      throw new Error(
        `${file}: link target ${target} has no site equivalent; ` +
          "add it to LINK_TARGETS or remove the link",
      );
    }
    const replacement = rewrite(prefix);
    const fragment = target.includes("#") ? target.slice(target.indexOf("#")) : "";
    return replacement === null ? text : `[${text}](${replacement}${fragment})`;
  });
}

/**
 * The first level-one heading, ignoring anything inside a fenced code block.
 *
 * A `#` line inside a shell example is a comment, not the document's title.
 */
function titleOf(body, file) {
  let fenced = false;
  for (const line of body.split("\n")) {
    if (/^\s*(```|~~~)/.test(line)) {
      fenced = !fenced;
      continue;
    }
    if (fenced) {
      continue;
    }
    const match = line.match(/^#\s+(.+?)\s*$/);
    if (match) {
      return { title: match[1], line };
    }
  }
  throw new Error(`${file}: no level-one heading to use as its title`);
}

function convert(body, prefix, file) {
  const { title, line } = titleOf(body, file);
  const withoutTitle = body.replace(line, "").replace(/^\s*\n/, "");
  const kept = withoutTitle
    .split("\n")
    .filter((entry) => !SWITCHER.test(entry))
    .join("\n");
  return { title, body: rewriteLinks(kept, prefix, file) };
}

function frontmatter(title) {
  return `---\ntitle: ${JSON.stringify(title)}\n---\n\n`;
}

async function main() {
  await rm(TARGET, { recursive: true, force: true });

  let count = 0;

  for (const { locale, source, prefix } of LOCALES) {
    const dir = join(TARGET, locale);
    await mkdir(dir, { recursive: true });

    const entries = [];
    for (const { file, slug } of PUBLISHED) {
      const raw = await readFile(join(source, file), "utf8");
      const { title, body } = convert(raw, prefix, file);
      await writeFile(join(dir, `${slug}.mdx`), frontmatter(title) + body, "utf8");
      entries.push({ slug, title });
      count += 1;
    }

    // The section root. Without it `/<locale>/docs/` — which the site's home
    // page links to as the primary call to action — fell through to the
    // catch-all and answered 404.
    const { title, lead } = INDEX[locale];
    const list = entries
      .map((entry) => `- [${entry.title}](${prefix}/${entry.slug})`)
      .join("\n");
    await writeFile(
      join(dir, "index.mdx"),
      `${frontmatter(title)}${lead}\n\n${list}\n`,
      "utf8",
    );
    count += 1;

    const pages = ["index", ...PUBLISHED.map((entry) => entry.slug)];
    await writeFile(
      join(dir, "meta.json"),
      JSON.stringify({ title: SECTION[locale], pages }, null, 2) + "\n",
      "utf8",
    );
  }

  console.log(`synced ${count} pages into content/docs/`);
}

await main();
