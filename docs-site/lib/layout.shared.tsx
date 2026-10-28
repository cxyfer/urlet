import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { appName, gitConfig } from './shared';

/**
 * Options shared by the documentation and home layouts.
 *
 * The language switcher is not configured here: with an `I18nProvider` above
 * the layout, Fumadocs renders it from the languages declared in `lib/i18n.ts`.
 */
export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: appName,
    },
    githubUrl: `https://github.com/${gitConfig.user}/${gitConfig.repo}`,
  };
}
