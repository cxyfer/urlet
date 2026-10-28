import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/**
 * The site is a static export: `next build` writes `out/`, which the workflow
 * publishes to the `gh-pages` branch. Nothing runs on a server.
 *
 * `basePath` is the repository name because GitHub Pages serves a project site
 * from `https://<owner>.github.io/<repo>/`, not from the domain root. Without
 * it every asset and route in the export points at `/_next/...` and `/zh-TW/...`
 * on the domain root, which resolves to another site's path — the pages load
 * unstyled, never hydrate, and every internal link 404s. A custom domain would
 * set this to an empty string and add a `CNAME` to `public/`.
 *
 * There is deliberately no `turbopack.root` override. The content lives in
 * `content/docs/` inside this project, generated from the repository's `docs/`
 * tree by `scripts/sync-content.mjs`, so the bundler never needs to reach
 * outside its root.
 */
/** @type {import('next').NextConfig} */
const config = {
  output: 'export',
  reactStrictMode: true,
  basePath: '/urlet',
  // Every route becomes a directory with an index.html, which is what a plain
  // static host expects: GitHub Pages would otherwise have to map /en/docs/api
  // onto en/docs/api.html.
  trailingSlash: true,
};

export default withMDX(config);
