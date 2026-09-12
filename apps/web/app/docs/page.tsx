// SPDX-License-Identifier: AGPL-3.0-only
import {DocPage} from '../../components/site';
import {pageMetadata} from '../../lib/site';
import docs from '../../lib/docs.json';
export const metadata=pageMetadata('Documentation',docs[0].description,'/docs/introduction/');
export default function Page(){return <DocPage/>;}
