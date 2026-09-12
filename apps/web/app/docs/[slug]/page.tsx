// SPDX-License-Identifier: AGPL-3.0-only
import {notFound} from 'next/navigation';
import {DocPage} from '../../../components/site';
import docs from '../../../lib/docs.json';
import {pageMetadata} from '../../../lib/site';
export const dynamicParams=false;
export function generateStaticParams(){return docs.map(({slug})=>({slug}));}
export async function generateMetadata({params}:{params:Promise<{slug:string}>}){const{slug}=await params;const doc=docs.find(d=>d.slug===slug);if(!doc)notFound();return pageMetadata(doc.title,doc.description,'/docs/'+doc.slug+'/');}
export default async function Page({params}:{params:Promise<{slug:string}>}){
 const{slug}=await params;if(!docs.some(d=>d.slug===slug))notFound();return <DocPage slug={slug}/>;
}
