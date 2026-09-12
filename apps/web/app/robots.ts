// SPDX-License-Identifier: AGPL-3.0-only
import type {MetadataRoute} from 'next';
import {site} from '../lib/site';

export const dynamic='force-static';
export default function robots():MetadataRoute.Robots{
  return {rules:{userAgent:'*',allow:'/',disallow:['/app-preview.html','/docs-markdown/']},
    sitemap:new URL('/sitemap.xml',site.url).href};
}
