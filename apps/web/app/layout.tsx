// SPDX-License-Identifier: AGPL-3.0-only
import type { Metadata, Viewport } from 'next';
import Script from 'next/script';
import type { ReactNode } from 'react';
import { site, pageMetadata } from '../lib/site';
import '../public/assets/site.css';
export const metadata: Metadata = {
  ...pageMetadata(site.title, site.description, '/'),
  metadataBase: new URL(site.url),
  title: { default: site.title, template: '%s | OpenXplorer' },
  description: site.description,
  applicationName: site.name,
  icons: { icon: '/assets/folder.svg' },
  robots: { index: true, follow: true }
};
export const viewport: Viewport = { themeColor: '#123fc6' };
export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>
        {children}
        <Script src="/assets/site.js" strategy="afterInteractive" />
      </body>
    </html>
  );
}
