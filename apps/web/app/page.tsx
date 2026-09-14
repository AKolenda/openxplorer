// SPDX-License-Identifier: AGPL-3.0-only
import {Home} from '../components/site';
import {PrototypeHome} from '../components/prototype-home';
import {site} from '../lib/site';
// PROTOTYPE: in non-production builds `/` serves the homepage prototype (`?variant=`).
// `pnpm build` (NODE_ENV=production) still exports today's page.
export default function Page(){return process.env.NODE_ENV==='production'?<Home vibe={site.defaultDesign}/>:<PrototypeHome/>;}
