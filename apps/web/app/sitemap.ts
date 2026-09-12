// SPDX-License-Identifier: AGPL-3.0-only
import type {MetadataRoute} from 'next';
import {site} from '../lib/site';
import docs from '../lib/docs.json';

export const dynamic='force-static';
export default function sitemap():MetadataRoute.Sitemap{
  return ['/','/source/',...docs.map(doc=>'/docs/'+doc.slug+'/')]
    .map(path=>({url:new URL(path,site.url).href}));
}
