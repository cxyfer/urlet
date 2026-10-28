'use client';
import SearchDialog from '@/components/search';
import { RootProvider } from 'fumadocs-ui/provider/next';
import type { RootProviderProps } from 'fumadocs-ui/provider/next';

/**
 * The client providers for one locale.
 *
 * `RootProvider` wraps the framework provider (Next's `Link`, router and
 * params), so the i18n configuration has to be passed through it rather than
 * rendered as a sibling: a separate `I18nProvider` above it would call the
 * framework hooks before the provider that supplies them exists.
 */
export function Provider({
  children,
  ...props
}: RootProviderProps & { children: React.ReactNode }) {
  return (
    <RootProvider search={{ SearchDialog }} {...props}>
      {children}
    </RootProvider>
  );
}
