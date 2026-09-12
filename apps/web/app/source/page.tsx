// SPDX-License-Identifier: AGPL-3.0-only
import {SourcePage} from '../../components/site';
import {pageMetadata} from '../../lib/site';
export const metadata=pageMetadata('License & source code','Browse the complete OpenXplorer source, build tools and AGPL-3.0-only license in the public GitHub repository.','/source/');
export default function Page(){return <SourcePage/>;}
