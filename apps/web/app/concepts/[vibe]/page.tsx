// SPDX-License-Identifier: AGPL-3.0-only
import {notFound} from 'next/navigation';
import {Home} from '../../../components/site';
import type {Vibe} from '../../../lib/site';
const vibes=['windows','zorin','vercel'];
export const dynamicParams=false;
export function generateStaticParams(){return vibes.map(vibe=>({vibe}));}
export async function generateMetadata({params}:{params:Promise<{vibe:string}>}){const{vibe}=await params;return{title:`Design lab · ${vibe}`,alternates:{canonical:'/'},robots:{index:false,follow:true}};}
export default async function Page({params}:{params:Promise<{vibe:string}>}){
 const{vibe}=await params;if(!vibes.includes(vibe))notFound();return <Home vibe={vibe as Vibe} lab/>;
}
