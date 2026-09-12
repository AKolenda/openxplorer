# SPDX-License-Identifier: AGPL-3.0-only
"""Tear-out decision and cancellation policy; no GUI or external processes."""
from types import SimpleNamespace as NS
from enum import IntFlag
from unittest.mock import Mock
import unittest
from native_tab_drag import NativeTabDrag, MIME, ROOT_MIME, INFO, ROOT_INFO
from tab_transfers import TabTransfers


class TearOutTests(unittest.TestCase):
    def setUp(self):
        self.events=[]
        self.hub=TabTransfers(lambda ident,name,data:self.events.append((name,data)),lambda i:True)
        self.token=self.hub.offer(1,'t1',{'uri':'file:///tmp/Sample'})
        self.drag=NativeTabDrag.__new__(NativeTabDrag)
        d=self.drag
        d.c=NS(writes=0,ui_ready=True,app=NS(tab_transfers=self.hub),emit=lambda name,data:self.events.append((name,data)))
        d.view=Mock();d.view.get_allocated_width.return_value=1000
        d.layout={'width':1000,'height':42,'end':870,'tabs':[{'id':'t1','left':0,'right':210,'close':190}]}
        d.token=self.token;d.tab_id='t1';d.context=object();d.press={};d.requested=True;d.tear_out_pending=False;d.last_rejected=False
        d.Gtk=NS(DragResult=NS(NO_TARGET=1));d.atom='private';d.root_atom='root'
        d.Gdk=NS(DragAction=IntFlag('Action',{'MOVE':4,'DEFAULT':1}),drag_status=Mock())
    def end(self):self.drag.drag_end(self.drag.view,self.drag.context)
    def test_own_strip_still_reorders(self):self.assertEqual(self.drag.drop_action(self.drag.view,self.drag,50,20),('tabs','t1'))
    def test_own_body_is_explicit_detach_target(self):self.assertEqual(self.drag.drop_action(self.drag.view,self.drag,50,180),('detach',None))
    def test_strip_buffer_not_a_detach_target(self):self.assertEqual(self.drag.drop_action(self.drag.view,self.drag,50,70),(None,None))
    def test_other_window_body_is_not_detach_target(self):self.assertEqual(self.drag.drop_action(self.drag.view,object(),50,180),(None,None))
    def test_window_controls_are_not_targets(self):self.assertEqual(self.drag.drop_action(self.drag.view,self.drag,990,20),(None,None))
    def test_negative_x_is_not_body(self):self.assertEqual(self.drag.drop_action(self.drag.view,self.drag,-5,180),(None,None))
    def test_respects_scaled_webview(self):
        self.drag.view.get_allocated_width.return_value=2000
        self.assertEqual(self.drag.drop_action(self.drag.view,self.drag,60,140),(None,None))
        self.assertEqual(self.drag.drop_action(self.drag.view,self.drag,60,180),('detach',None))
    def test_busy_source_keeps_tab(self):
        self.drag.c.writes=1
        self.assertEqual(self.drag.drop_action(self.drag.view,self.drag,60,200),(None,None))
    def test_unready_source_keeps_tab(self):
        self.drag.c.ui_ready=False
        self.assertEqual(self.drag.drop_action(self.drag.view,self.drag,60,200),(None,None))
    def test_missing_geometry_not_a_target(self):
        self.drag.layout=None
        self.assertEqual(self.drag.drop_action(self.drag.view,self.drag,60,200),(None,None))
    def test_root_offer_not_a_uri_target(self):
        self.assertEqual(ROOT_MIME,'application/x-rootwindow-drop');self.assertNotEqual(ROOT_INFO,INFO);self.assertNotIn('uri',MIME)
    def test_root_request_always_replies_empty(self):
        data=Mock();self.drag.data_get(self.drag.view,self.drag.context,data,ROOT_INFO,0)
        data.set.assert_called_once_with('root',8,b'');self.assertTrue(self.drag.tear_out_pending)
        self.assertFalse(self.events)  # no window creation while grab still active
    def test_root_drop_defers_until_end(self):
        self.drag.data_get(self.drag.view,self.drag.context,Mock(),ROOT_INFO,0);self.end()
        self.assertEqual(sum(n=='tabDetachRequested' for n,_ in self.events),1)
        self.assertFalse(any(n=='tabTransferDone' and v['committed'] for n,v in self.events))
    def test_cancel_emitted_before_detach_to_unlock_source(self):
        self.drag.tear_out_pending=True;self.end()
        names=[n for n,_ in self.events];self.assertLess(names.index('tabTransferDone'),names.index('tabDetachRequested'))
    def test_private_get_clears_tentative_root_flag(self):
        self.drag.tear_out_pending=True;data=Mock();self.drag.data_get(self.drag.view,self.drag.context,data,INFO,0)
        self.assertFalse(self.drag.tear_out_pending);data.set.assert_called_once_with('private',8,self.token.encode('ascii'))
    def test_unknown_info_gets_no_data(self):
        data=Mock();self.drag.data_get(self.drag.view,self.drag.context,data,99,0);data.set.assert_not_called()
    def test_wrong_context_gets_no_data(self):
        data=Mock();self.drag.data_get(self.drag.view,object(),data,ROOT_INFO,0);data.set.assert_not_called()
    def test_failure_no_target_detaches_on_end_only(self):
        self.drag.drag_failed(self.drag.view,self.drag.context,1);self.assertFalse(self.events);self.end()
        self.assertEqual(sum(n=='tabDetachRequested' for n,_ in self.events),1)
    def test_rejected_receiver_does_not_become_desktop_tear_out(self):
        self.drag.last_rejected=True
        self.drag.drag_failed(self.drag.view,self.drag.context,1);self.end()
        self.assertFalse(any(n=='tabDetachRequested' for n,_ in self.events))
    def test_receiver_without_layout_declines_without_crashing(self):
        self.drag.layout=None;self.drag.source=lambda ctx:NS()
        self.assertTrue(self.drag.drag_motion(self.drag.view,self.drag.context,100,200,0))
        self.assertEqual(self.drag.Gdk.drag_status.call_args.args[1],0)
    def test_generic_error_not_misclassified_as_user_drop(self):
        self.drag.drag_failed(self.drag.view,self.drag.context,5);self.end()
        self.assertFalse(any(n=='tabDetachRequested' for n,_ in self.events))
    def test_escape_cancels_even_after_tentative_root_request(self):
        self.drag.tear_out_pending=True;self.drag.drag_failed(self.drag.view,self.drag.context,2);self.end()
        self.assertFalse(self.hub.pending);self.assertFalse(any(n=='tabDetachRequested' for n,_ in self.events))
    def test_acknowledged_merge_cannot_also_detach(self):
        self.hub.claim(self.token,2);self.hub.ready(self.token,2,True);self.drag.tear_out_pending=True;self.end()
        self.assertFalse(any(n=='tabDetachRequested' for n,_ in self.events))
    def test_pending_merge_waits_for_ack_after_drag_end(self):
        self.hub.claim(self.token,2);self.end();self.assertIn(self.token,self.hub.pending)
        self.hub.ready(self.token,2,True);self.assertFalse(self.hub.pending)
    def test_late_end_after_expiry_does_not_detach(self):
        self.hub.cancel(self.token);self.drag.tear_out_pending=True;self.end()
        self.assertFalse(any(n=='tabDetachRequested' for n,_ in self.events))
    def test_duplicate_end_does_not_duplicate_window(self):
        context=self.drag.context;self.drag.tear_out_pending=True;self.end();self.drag.drag_end(self.drag.view,context)
        self.assertEqual(sum(n=='tabDetachRequested' for n,_ in self.events),1)
    def test_drop_hint_rejects_with_zero_not_default_action(self):
        self.drag.source=lambda ctx:NS();self.drag.drag_motion(self.drag.view,self.drag.context,100,200,0)
        self.assertEqual(self.drag.Gdk.drag_status.call_args.args[1],0)

if __name__=='__main__':unittest.main()
