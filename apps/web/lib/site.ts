// SPDX-License-Identifier: AGPL-3.0-only
import type {Metadata} from 'next';
/** Edit public identity here. Never invent a repository, statistics or support claims. */
export const site = {
  name: 'OpenXplorer',
  url: 'https://openxplorer.app',
  version: '1.1.1',
  title: 'OpenXplorer — A familiar file manager for Linux',
  description: 'Browse local files and SMB shares, drag files into compatible apps, and keep folders close with tabs and pins. An open-source Linux file manager built for Zorin OS.',
  license: 'AGPL-3.0-only',
  repository: 'https://github.com/AKolenda/openxplorer',
  releases: 'https://github.com/AKolenda/openxplorer/releases',
  defaultDesign: 'zorin' as 'vercel' | 'windows' | 'zorin',
};
export type Vibe = 'windows' | 'zorin' | 'vercel';

export function pageMetadata(title:string,description:string,path:string):Metadata{
  const image={url:'/assets/screenshots/explorer-light.png',width:1440,height:900,
    alt:'OpenXplorer file manager showing fictional sample files and network folders'};
  return {title,description,alternates:{canonical:path},
    openGraph:{type:'website',siteName:site.name,title,description,url:path,locale:'en_US',images:[image]},
    twitter:{card:'summary_large_image',title,description,images:[image]}};
}
