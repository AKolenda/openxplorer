# SPDX-License-Identifier: AGPL-3.0-only
from pathlib import Path
from types import SimpleNamespace as NS
import tempfile
import unittest
from tab_transfers import TabTransfers
from native_tab_drag import layout_value
from desktop_integration import DesktopIntegration, APP_ID, TYPES, ZIP_TYPES


class TabTransferTests(unittest.TestCase):
    def setUp(self):
        self.now = 100.; self.events = []; self.live = {1, 2, 3}
        self.hub = TabTransfers(lambda *args: self.events.append(args), lambda w:w in self.live, clock=lambda:self.now)
        self.snapshot = {'uri':'smb://studio-nas/Projects','history':['file:///home/demo','smb://studio-nas/Projects'],'index':1,'selection':['smb://studio-nas/Projects/Readme.md'],'scroll':500,'view':'grid','sort':'size','descending':True}
    def offer(self):return self.hub.offer(1, 'tab-1', self.snapshot)
    def test_capability_is_random_and_never_contains_a_uri(self):
        a=self.offer();self.hub.cancel(a);b=self.offer();self.assertNotEqual(a,b);self.assertEqual(len(a),64);self.assertNotIn('smb:',a)
    def test_claim_does_not_remove_source(self):
        t=self.offer();self.hub.claim(t,2);self.assertEqual([e[1] for e in self.events],['tabReceive']);self.assertTrue(self.hub.busy(1))
    def test_ack_commits_once_to_original_source(self):
        t=self.offer();self.hub.claim(t,2,'existing-tab');result=self.hub.ready(t,2,True);self.assertTrue(result['committed']);self.assertFalse(self.hub.pending);self.assertEqual(self.events[-1][0],1);self.assertEqual(self.events[-1][1],'tabTransferDone');self.assertTrue(self.events[-1][2]['committed'])
    def test_receives_snapshot_position_and_selection(self):
        t=self.offer();self.hub.claim(t,2,'first');data=self.events[-1][2];self.assertEqual(data['tab']['selection'],self.snapshot['selection']);self.assertEqual(data['tab']['index'],1);self.assertEqual(data['beforeId'],'first')
    def test_spoofed_ack_cannot_close_source(self):
        t=self.offer();self.hub.claim(t,2);self.assertFalse(self.hub.ready(t,3,True)['committed']);self.assertIn(t,self.hub.pending)
    def test_unknown_token_rejected(self):
        with self.assertRaises(ValueError):self.hub.claim('0'*64,2)
    def test_replayed_token_rejected(self):
        t=self.offer();self.hub.claim(t,2);self.hub.ready(t,2,True)
        with self.assertRaises(ValueError):self.hub.claim(t,3)
    def test_claim_cannot_be_retargeted(self):
        t=self.offer();self.hub.claim(t,2)
        with self.assertRaises(ValueError):self.hub.claim(t,3)
    def test_expiration_rolls_back_destination_and_keeps_source(self):
        t=self.offer();self.hub.claim(t,2);self.now=140;self.hub.expire();self.assertFalse(self.hub.pending);self.assertFalse(self.events[-1][2]['committed']);self.assertEqual(self.events[-2][1],'tabTransferSettled')
    def test_late_ack_after_timeout_never_commits(self):
        t=self.offer();self.hub.claim(t,2);self.now=140;self.assertFalse(self.hub.ready(t,2,True)['committed'])
    def test_negative_ack_keeps_source(self):
        t=self.offer();self.hub.claim(t,2);self.hub.ready(t,2,False);self.assertFalse(self.events[-1][2]['committed'])
    def test_destination_close_rolls_back(self):
        t=self.offer();self.hub.claim(t,2);self.hub.window_closed(2);self.assertFalse(self.hub.pending);self.assertFalse(self.events[-1][2]['committed'])
    def test_source_close_rolls_back(self):
        t=self.offer();self.hub.claim(t,2);self.hub.window_closed(1);self.assertFalse(self.hub.pending)
    def test_unready_destination_not_claimed(self):
        t=self.offer();self.live.remove(2)
        with self.assertRaises(ValueError):self.hub.claim(t,2)
        self.assertIsNone(self.hub.pending[t].destination)
    def test_same_window_rejected_by_cross_window_hub(self):
        with self.assertRaises(ValueError):self.hub.claim(self.offer(),1)
    def test_concurrent_transfer_of_same_tab_rejected(self):
        self.offer()
        with self.assertRaises(ValueError):self.offer()
    def test_no_credentials_or_arbitrary_js_forwarded(self):
        self.snapshot.update(password='fictional-only',script='alert(1)');t=self.offer();self.hub.claim(t,2);self.assertNotIn('password',self.events[-1][2]['tab']);self.assertNotIn('script',self.events[-1][2]['tab'])
    def test_invalid_location_rejected(self):
        with self.assertRaises(ValueError):self.hub.offer(1,'t',{'uri':'javascript:alert(1)'})
    def test_non_integer_destination_rejected(self):
        with self.assertRaises(ValueError):self.hub.claim(self.offer(),True)
    def test_global_cap_is_bounded(self):
        for i in range(64):self.hub.offer(1,str(i),self.snapshot)
        with self.assertRaises(ValueError):self.hub.offer(1,'more',self.snapshot)


class LayoutTests(unittest.TestCase):
    def valid(self):return {'width':1000,'height':42,'end':840,'tabs':[{'id':'t1','left':0,'right':210,'close':182}]}
    def test_accepts_normal_geometry(self):self.assertEqual(layout_value(self.valid())['tabs'][0]['id'],'t1')
    def test_accepts_scrolled_tab(self):
        v=self.valid();v['tabs'][0].update(left=-100,right=110,close=80);self.assertEqual(layout_value(v)['tabs'][0]['left'],-100)
    def test_rejects_nan(self):
        v=self.valid();v['width']=float('nan')
        with self.assertRaises(ValueError):layout_value(v)
    def test_rejects_infinite(self):
        v=self.valid();v['tabs'][0]['left']=float('inf')
        with self.assertRaises(ValueError):layout_value(v)
    def test_rejects_overlarge_drop_zone(self):
        v=self.valid();v['height']=500
        with self.assertRaises(ValueError):layout_value(v)
    def test_rejects_too_many_tabs(self):
        v=self.valid();v['tabs']*=201
        with self.assertRaises(ValueError):layout_value(v)
    def test_rejects_invalid_width(self):
        v=self.valid();v['width']=True
        with self.assertRaises(ValueError):layout_value(v)
    def test_rejects_reversed_rect(self):
        v=self.valid();v['tabs'][0]['right']=-1
        with self.assertRaises(ValueError):layout_value(v)


class DefaultsTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.mapping={k:'org.kde.dolphin.desktop' for k in TYPES+ZIP_TYPES};self.commands=[]
        def run(args):
            self.commands.append(args)
            if args[1:3]==['query','default']:return self.mapping[args[3]]
            self.assertEqual(args[:2],['xdg-mime','default']);self.mapping[args[3]]=args[2];return ''
        self.integration=DesktopIntegration(Path(self.tmp.name),run)
    def test_folders_and_zip_report_independently(self):
        self.integration.make_default();s=self.integration.status();self.assertTrue(s['allDefault']);self.assertFalse(s['zipDefault']);self.assertEqual(s['current']['application/zip'],'org.kde.dolphin.desktop')
    def test_existing_make_default_does_not_hijack_zip(self):
        self.integration.make_default();self.assertTrue(all(self.mapping[k]=='org.kde.dolphin.desktop' for k in ZIP_TYPES))
    def test_explicit_zip_option_changes_all_zip_aliases(self):
        self.assertTrue(self.integration.make_default(include_zip=True)['zipDefault']);self.assertTrue(all(self.mapping[k]==APP_ID for k in TYPES+ZIP_TYPES))
    def test_zip_only_keeps_folder_default(self):
        self.integration.zip_default();self.assertTrue(all(self.mapping[k]=='org.kde.dolphin.desktop' for k in TYPES));self.assertTrue(self.integration.status()['zipDefault'])
    def test_zip_restore_does_not_restore_folders(self):
        self.integration.make_default(include_zip=True);self.integration.restore(zip_only=True);self.assertTrue(self.integration.status()['allDefault']);self.assertFalse(self.integration.status()['zipDefault'])
    def test_restore_does_not_overwrite_new_user_choice(self):
        self.integration.zip_default();self.mapping['application/zip']='org.gnome.FileRoller.desktop';self.integration.restore(zip_only=True);self.assertEqual(self.mapping['application/zip'],'org.gnome.FileRoller.desktop')
    def test_reapply_preserves_original_backup(self):
        self.integration.zip_default();self.integration.zip_default();self.integration.restore(zip_only=True);self.assertEqual(self.mapping['application/zip'],'org.kde.dolphin.desktop')
    def test_unconfirmed_install_is_read_only(self):
        self.integration.status();self.assertTrue(all(a[1]=='query' for a in self.commands))
    def test_restore_all(self):
        self.integration.make_default(include_zip=True);self.integration.restore();self.assertTrue(all(v=='org.kde.dolphin.desktop' for v in self.mapping.values()))
    def test_bad_previous_handler_rejected(self):
        self.mapping['application/zip']='evil;command.desktop'
        with self.assertRaises(ValueError):self.integration.zip_default()
    def test_missing_handler_not_overpromised(self):
        self.mapping['application/zip']='';s=self.integration.status();self.assertEqual(s['current']['application/zip'],'');self.assertFalse(s['zipDefault'])
    def test_backup_is_private(self):
        self.integration.zip_default();self.assertEqual(self.integration.path.stat().st_mode&0o777,0o600)

if __name__=='__main__':unittest.main()
