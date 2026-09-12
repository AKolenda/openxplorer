# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Whitelisted tab handoff and FileManager1 argument validation (no GI imports)."""
from core import normalise_location
import math

VIRTUAL = {'settings:', 'pc:', 'network:', 'home:'}


def location(value):
    if not isinstance(value,str):raise ValueError('Location must be a string.')
    return value if value in VIRTUAL else normalise_location(value)


def tab_snapshot(value):
    if not isinstance(value, dict): raise ValueError('Invalid tab state.')
    uri = location(value.get('uri', 'home:'))
    history = value.get('history', [uri])
    if not isinstance(history, list) or not 1 <= len(history) <= 200:
        raise ValueError('Tab history must contain 1–200 locations.')
    history = [location(u) for u in history]
    index = value.get('index', len(history)-1)
    if type(index) is not int or not 0 <= index < len(history): raise ValueError('Invalid history position.')
    if history[index] != uri: history, index = [uri], 0
    selected = value.get('selection', [])
    if not isinstance(selected, list) or len(selected)>10000: raise ValueError('Invalid selection.')
    try:scroll=float(value.get('scroll',0))
    except (ValueError,TypeError):raise ValueError('Invalid scroll position.')
    if not math.isfinite(scroll):raise ValueError('Invalid scroll position.')
    return {'uri':uri, 'history':history, 'index':index,
            'scroll':max(0,min(1e9,scroll)),
            'selection':[normalise_location(u) for u in selected],
            'view':'grid' if value.get('view')=='grid' else 'details',
            'sort':value.get('sort') if value.get('sort') in ('name','modified','type','size') else 'name',
            'descending':value.get('descending') is True,
            'settingsSection':value.get('settingsSection') if value.get('settingsSection') in ('appearance','search','default','brave','windows','sizes') else None}


def filemanager_request(method, uris):
    if method not in ('ShowFolders','ShowItems','ShowItemProperties'): raise ValueError('Unsupported method.')
    if not isinstance(uris,(list,tuple)) or not 1 <= len(uris) <= 100: raise ValueError('Expected 1–100 file locations.')
    values = [normalise_location(u) for u in uris]
    return {'method':method,'uris':values}
