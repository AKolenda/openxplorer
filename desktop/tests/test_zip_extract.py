# SPDX-License-Identifier: AGPL-3.0-only
"""Real local ZIP I/O against the shipped engine; not native GIO/SMB tests."""
from contextlib import contextmanager
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch
import warnings
import zipfile
from archives import Archives
from core import Settings
from operations import Cancelled
from previous_versions import PreviousVersions
from zip_extraction import Limits, ZipExtractor, plan, suggested_name
from tests.local_provider import LocalNode, Cancellation

@contextmanager
def source_open(uri, cancel):
    cancel.check()
    with LocalNode(uri).p.open('rb') as stream:yield stream

@contextmanager
def writer_open(node, cancel):
    cancel.check()
    with node.p.open('xb') as stream:
        os.fchmod(stream.fileno(), 0o600)
        yield stream

class ZipExtractTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
        self.zip=self.root/'sample.zip';self.dest=self.root/'destination';self.dest.mkdir()
        self.cancel=Cancellation();self.events=[]
        self.extractor=ZipExtractor(Archives(source_open),LocalNode,writer_open,self.events.append)
    def tearDown(self):self.tmp.cleanup()
    def make_zip(self,items=None,compression=zipfile.ZIP_DEFLATED):
        with warnings.catch_warnings():
            warnings.simplefilter('ignore',UserWarning)
            with zipfile.ZipFile(self.zip,'w',compression=compression) as z:
                for name,content in (items if items is not None else [('Docs/',b''),('Docs/Guide.txt',b'hello world'),('Zero.bin',b'')]):z.writestr(name,content)
        return self.zip.as_uri()
    def run_extract(self,name='Unpacked'):return self.extractor.extract(self.zip.as_uri(),self.dest.as_uri(),name,self.cancel)
    def assert_no_output(self):self.assertEqual(list(self.dest.iterdir()),[])
    def test_round_trip(self):
        self.make_zip();r=self.run_extract()
        self.assertEqual((self.dest/'Unpacked/Docs/Guide.txt').read_bytes(),b'hello world');self.assertEqual((self.dest/'Unpacked/Zero.bin').stat().st_size,0)
        self.assertEqual((r['files'],r['folders'],r['bytes']),(2,1,11));self.assertEqual(r['uri'],(self.dest/'Unpacked').as_uri())
    def test_implied_directories(self):
        self.make_zip([('a/b/c.txt',b'yes')]);self.run_extract();self.assertEqual((self.dest/'Unpacked/a/b/c.txt').read_text(),'yes')
    def test_parent_after_child(self):
        self.make_zip([('a/b.txt',b'data'),('a/',b'')]);self.run_extract();self.assertTrue((self.dest/'Unpacked/a/b.txt').is_file())
    def test_empty_archive(self):
        self.make_zip([]);r=self.run_extract();self.assertEqual(r['files'],0);self.assertTrue((self.dest/'Unpacked').is_dir())
    def test_device_destination_does_not_require_unix_chmod(self):
        class DeviceNode(LocalNode):
            def __init__(self,uri=None,path=None):
                super().__init__(uri=uri,path=path)
                self.uri='mtp://test-device'+self.p.as_posix()
        self.make_zip()
        self.extractor.factory=DeviceNode
        # The simulated device writer creates files without applying Unix modes.
        @contextmanager
        def device_writer(node,cancel):
            with node.p.open('xb') as stream:yield stream
        self.extractor.writer=device_writer
        with patch('operations.os.fchmod',side_effect=OSError('Device does not support chmod')) as chmod:
            result=self.run_extract()
        self.assertEqual(result['files'],2)
        self.assertEqual((self.dest/'Unpacked'/'Docs'/'Guide.txt').read_text(),'hello world')
        chmod.assert_not_called()
    def test_protected_extraction_descendant_fails_before_writing(self):
        self.make_zip([('ordinary.txt',b'first'),('.snapshot/version.txt',b'backup')])
        versions=PreviousVersions(self.root/'configuration')
        self.extractor.assert_writable=versions.assert_writable
        with self.assertRaisesRegex(ValueError,'read-only'):self.run_extract()
        self.assert_no_output()
    def test_configured_extraction_root_is_protected(self):
        self.make_zip();versions=PreviousVersions(self.root/'configuration')
        versions.configure(self.dest.as_uri(),(self.dest/'Unpacked').as_uri())
        self.extractor.assert_writable=versions.assert_writable
        with self.assertRaisesRegex(ValueError,'read-only'):self.run_extract()
        self.assert_no_output()
    def test_empty_folders(self):
        self.make_zip([('a/b/',b'')]);self.run_extract();self.assertTrue((self.dest/'Unpacked/a/b').is_dir())
    def test_unicode_spaces(self):
        self.make_zip([('Design notes/Café.txt',b'fictional')]);self.run_extract();self.assertTrue((self.dest/'Unpacked/Design notes/Café.txt').is_file())
    def test_source_unchanged(self):
        self.make_zip();before=self.zip.read_bytes();self.run_extract();self.assertEqual(before,self.zip.read_bytes())
    def test_existing_folder(self):
        self.make_zip();(self.dest/'Unpacked').mkdir();f=self.dest/'Unpacked/sentinel';f.write_text('keep')
        with self.assertRaises(FileExistsError):self.run_extract()
        self.assertEqual(f.read_text(),'keep');self.assertEqual(len(list(self.dest.iterdir())),1)
    def test_existing_file(self):
        self.make_zip();f=self.dest/'Unpacked';f.write_text('keep')
        with self.assertRaises(FileExistsError):self.run_extract()
        self.assertEqual(f.read_text(),'keep')
    def test_existing_symlink(self):
        self.make_zip();other=self.root/'other';other.mkdir();(self.dest/'Unpacked').symlink_to(other)
        with self.assertRaises(FileExistsError):self.run_extract()
        self.assertTrue((self.dest/'Unpacked').is_symlink());self.assertEqual(list(other.iterdir()),[])
    def test_dangling_symlink(self):
        self.make_zip();(self.dest/'Unpacked').symlink_to(self.root/'missing')
        with self.assertRaises(FileExistsError):self.run_extract()
        self.assertTrue((self.dest/'Unpacked').is_symlink())
    def test_parent_cannot_be_symlink(self):
        self.make_zip();link=self.root/'linked';link.symlink_to(self.dest)
        with self.assertRaises(ValueError):self.extractor.extract(self.zip.as_uri(),link.as_uri(),'Test',self.cancel)
        self.assert_no_output()
    def test_unsafe_paths(self):
        for name in ['../outside','a/../../outside','/etc/x','//server/share','a\\evil','C:/file','a/./b','a//b','a:stream','a/..','a\x01b','a.','a ','CON','a/NUL.txt']:
            with self.subTest(name=name):
                self.make_zip([('good.txt',b'good'),(name,b'bad')])
                with self.assertRaises(ValueError):self.run_extract()
                self.assert_no_output()
    def test_duplicate_names(self):
        self.make_zip([('a.txt',b'first'),('a.txt',b'second')])
        with self.assertRaisesRegex(ValueError,'duplicate'):self.run_extract()
        self.assert_no_output()
    def test_case_and_unicode_aliases(self):
        for paths in [[('Note.txt',b'a'),('note.txt',b'b')],[('Docs/a',b'1'),('docs/b',b'2')],[('Café/a',b'a'),('Cafe\u0301/b',b'b')]]:
            with self.subTest(paths=paths):
                self.make_zip(paths)
                with self.assertRaises(ValueError):self.run_extract()
                self.assert_no_output()
    def test_file_directory_conflicts(self):
        for paths in [[('a',b'f'),('a/b',b'g')],[('a/b',b'g'),('a',b'f')]]:
            self.make_zip(paths)
            with self.assertRaises(ValueError):self.run_extract()
            self.assert_no_output()
    def test_archive_symlink(self):
        i=zipfile.ZipInfo('link');i.create_system=3;i.external_attr=(stat.S_IFLNK|0o777)<<16;self.make_zip([(i,b'../../outside')])
        with self.assertRaisesRegex(ValueError,'symbolic link'):self.run_extract()
        self.assert_no_output()
    def test_special_files(self):
        for mode in [stat.S_IFIFO,stat.S_IFCHR,stat.S_IFBLK,stat.S_IFSOCK]:
            i=zipfile.ZipInfo('special');i.create_system=3;i.external_attr=(mode|0o600)<<16;self.make_zip([(i,b'')])
            with self.assertRaisesRegex(ValueError,'special file'):self.run_extract()
            self.assert_no_output()
    def test_never_restores_executable_or_setuid_mode(self):
        i=zipfile.ZipInfo('script.sh');i.create_system=3;i.external_attr=(stat.S_IFREG|0o4755)<<16;self.make_zip([(i,b'#!/bin/sh\necho never run\n')]);self.run_extract()
        self.assertEqual(stat.S_IMODE((self.dest/'Unpacked/script.sh').stat().st_mode),0o600);self.assertEqual(stat.S_IMODE((self.dest/'Unpacked').stat().st_mode),0o700)
    def test_inspection_writes_nothing(self):
        self.make_zip();r=self.extractor.inspect(self.zip.as_uri(),self.cancel);self.assertEqual(r['files'],2);self.assert_no_output()
    def test_encrypted_flag(self):
        self.make_zip()
        with zipfile.ZipFile(self.zip) as z:
            z.infolist()[1].flag_bits|=1
            with self.assertRaisesRegex(ValueError,'Password-protected'):plan(z,self.cancel)
    def test_nul_truncation(self):
        self.make_zip()
        with zipfile.ZipFile(self.zip) as z:
            z.infolist()[0].orig_filename+='\0suffix'
            with self.assertRaisesRegex(ValueError,'invalid'):plan(z,self.cancel)
    def test_directory_payload(self):
        self.make_zip([('bad/',b'payload')])
        with self.assertRaises(ValueError):self.run_extract()
        self.assert_no_output()
    def test_unsupported_compression(self):
        self.make_zip()
        with zipfile.ZipFile(self.zip) as z:
            z.infolist()[0].compress_type=99
            with self.assertRaisesRegex(ValueError,'compression'):plan(z,self.cancel)
    def test_entries_limit(self):
        self.make_zip();self.extractor.limits=Limits(entries=2)
        with self.assertRaisesRegex(ValueError,'entries'):self.run_extract()
        self.assert_no_output()
    def test_paths_limit(self):
        self.make_zip([('a/b/c/d.txt',b'data')]);self.extractor.limits=Limits(paths=3)
        with self.assertRaisesRegex(ValueError,'paths'):self.run_extract()
        self.assert_no_output()
    def test_depth_limit(self):
        self.make_zip([('/'.join(['folder']*129)+'/a',b'')])
        with self.assertRaisesRegex(ValueError,'nesting'):self.run_extract()
        self.assert_no_output()
    def test_filename_size_limit(self):
        self.make_zip([('x'*256,b'data')])
        with self.assertRaises(ValueError):self.run_extract()
        self.assert_no_output()
    def test_member_bytes_limit(self):
        self.make_zip([('large',b'0123456789')]);self.extractor.limits=Limits(member_bytes=5)
        with self.assertRaisesRegex(ValueError,'decompression'):self.run_extract()
        self.assert_no_output()
    def test_total_bytes_limit(self):
        self.make_zip([('a',b'1234'),('b',b'1234')]);self.extractor.limits=Limits(total_bytes=7)
        with self.assertRaisesRegex(ValueError,'extraction limit'):self.run_extract()
        self.assert_no_output()
    def test_ratio_limit(self):
        self.make_zip([('large',b'0'*50000)]);self.extractor.limits=Limits(ratio=3)
        with self.assertRaisesRegex(ValueError,'decompression'):self.run_extract()
        self.assert_no_output()
    def test_progress_and_cleanup(self):
        self.make_zip();self.run_extract();self.assertEqual(self.events[-1]['fraction'],1);self.assertTrue(any(e['label'].startswith('Extracting ') for e in self.events));self.assertEqual(len(list(self.dest.iterdir())),1)
    def test_cancel_before_start(self):
        self.make_zip();self.cancel.cancel()
        with self.assertRaises(Cancelled):self.run_extract()
        self.assert_no_output()
    def test_cancel_during_write(self):
        self.make_zip([('large.bin',os.urandom(150000))])
        def progress(data):
            if data['label'].startswith('Extracting '):self.cancel.cancel()
        self.extractor.emit=progress
        with self.assertRaises(Cancelled):self.run_extract()
        self.assert_no_output();self.assertTrue(self.zip.is_file())
    def test_crc_failure_rolls_back(self):
        self.make_zip([('doc.txt',b'contents')],zipfile.ZIP_STORED)
        with zipfile.ZipFile(self.zip) as z:i=z.infolist()[0];start=i.header_offset+30+len(i.filename.encode())+len(i.extra)
        data=bytearray(self.zip.read_bytes());data[start]^=1;self.zip.write_bytes(data)
        with self.assertRaises(zipfile.BadZipFile):self.run_extract()
        self.assert_no_output()
    def test_disk_error_rolls_back(self):
        self.make_zip()
        @contextmanager
        def broken(node,cancel):
            with writer_open(node,cancel) as stream:
                class Broken:
                    def write(self,data):stream.write(data[:2]);raise OSError('Disk full')
                yield Broken()
        self.extractor.writer=broken
        with self.assertRaisesRegex(OSError,'Disk full'):self.run_extract()
        self.assert_no_output()
    def test_short_write_rolls_back(self):
        self.make_zip()
        @contextmanager
        def short(node,cancel):
            with writer_open(node,cancel) as stream:
                class Short:
                    def write(self,data):return stream.write(data[:1])
                yield Short()
        self.extractor.writer=short
        with self.assertRaisesRegex(OSError,'all extracted bytes'):self.run_extract()
        self.assert_no_output()
    def test_racing_destination_is_not_replaced(self):
        self.make_zip()
        class Racing(LocalNode):
            def move_native(self,target,cancel=None):
                target.p.mkdir();(target.p/'competitor').write_text('keep');super().move_native(target,cancel)
        self.extractor.factory=Racing
        with self.assertRaises(OSError):self.run_extract()
        self.assertEqual((self.dest/'Unpacked/competitor').read_text(),'keep');self.assertEqual(len(list(self.dest.iterdir())),1)
    def test_not_a_zip(self):
        self.zip.write_bytes(b'not a zip')
        with self.assertRaises(zipfile.BadZipFile):self.run_extract()
        self.assert_no_output()
    def test_invalid_destination_names(self):
        self.make_zip()
        for name in ['','.', '..','../escape','a/b','a\\b','\0bad']:
            with self.subTest(name=name),self.assertRaises(ValueError):self.run_extract(name)
        self.assert_no_output()
    def test_supported_compression_round_trip(self):
        for method in [zipfile.ZIP_STORED,zipfile.ZIP_DEFLATED,zipfile.ZIP_BZIP2,zipfile.ZIP_LZMA]:
            self.make_zip([('a.txt',b'data')],method);self.run_extract(str(method));self.assertEqual((self.dest/str(method)/'a.txt').read_bytes(),b'data')
    def test_suggested_names(self):
        self.assertEqual(suggested_name('Assets.ZIP'),'Assets');self.assertEqual(suggested_name('.zip'),'Extracted files');self.assertEqual(suggested_name('Multi.part.zip'),'Multi.part')

class TextPreferenceTests(unittest.TestCase):
    def test_default_round_trip(self):
        with tempfile.TemporaryDirectory() as d:
            s=Settings(Path(d));self.assertEqual(s.snapshot()['preferences']['textSize'],100);s.update_preferences({'textSize':150});self.assertEqual(Settings(Path(d)).snapshot()['preferences']['textSize'],150)
    def test_invalid_values_ignored(self):
        with tempfile.TemporaryDirectory() as d:
            s=Settings(Path(d));s.update_preferences({'textSize':125})
            for v in [True,False,'150',150.0,0,201,-1,10000,101,None,{},float('nan')]:
                s.update_preferences({'textSize':v});self.assertEqual(s.snapshot()['preferences']['textSize'],125)
    def test_all_sizes(self):
        with tempfile.TemporaryDirectory() as d:
            s=Settings(Path(d))
            for value in [80,90,100,110,125,150,175,200]:self.assertEqual(s.update_preferences({'textSize':value})['textSize'],value)
    def test_preserves_other_settings(self):
        with tempfile.TemporaryDirectory() as d:
            s=Settings(Path(d));s.update_preferences({'theme':'dark','sidebarWidth':310,'showHidden':True});s.update_preferences({'textSize':150});p=s.snapshot()['preferences'];self.assertEqual(p['theme'],'dark');self.assertEqual(p['sidebarWidth'],310);self.assertTrue(p['showHidden'])
    def test_multiple_instances_merge(self):
        with tempfile.TemporaryDirectory() as d:
            a=Settings(Path(d));b=Settings(Path(d));a.update_preferences({'textSize':175});b.update_preferences({'theme':'dark'});p=Settings(Path(d)).snapshot()['preferences'];self.assertEqual(p['textSize'],175);self.assertEqual(p['theme'],'dark')
if __name__=='__main__':unittest.main()
