// SPDX-License-Identifier: AGPL-3.0-only
import {Header,Footer,GlobalSearch} from '../components/site';
export default function NotFound(){return <div className="site theme-vercel"><Header/><main id="main" className="section"><span className="eyebrow">404 / NOT FOUND</span><h1>This path doesn’t exist.</h1><p style={{margin:'25px 0'}}>The page may have moved. Your next stop is still close by.</p><a className="button primary" href="/docs/introduction/">Open the documentation →</a></main><Footer/><GlobalSearch/></div>;}
