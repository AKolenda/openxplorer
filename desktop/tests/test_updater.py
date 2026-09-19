# SPDX-License-Identifier: AGPL-3.0-only
"""Updater verification with fictional HTTP bytes and mocked package commands.

No network connection, administrator prompt or package installation is made.
"""
import ast
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch
from urllib.error import HTTPError, URLError
from urllib.request import Request

from updater import (HOSTS, LATEST, MAX_PACKAGE, REPOSITORY, TrustedRedirect,
                     Updater, open_url, release_metadata, trusted_url, version_tuple)


CURRENT = '1.0.0'
NEXT = '1.0.1'
PACKAGE = b'Fictional package bytes. Never an executable Debian package.\n'


def release(version=NEXT):
    name = f'openxplorer_{version}_all.deb'
    return {'tag_name': 'v' + version, 'draft': False, 'prerelease': False,
            'body': 'Fictional release notes.', 'assets': [{
                'name': name, 'browser_download_url':
                    f'{REPOSITORY}/releases/download/v{version}/{name}',
                'digest': 'sha256:' + hashlib.sha256(PACKAGE).hexdigest(),
                'size': len(PACKAGE)}]}


class MetadataTests(unittest.TestCase):
    def test_stable_versions_are_compared_numerically(self):
        self.assertEqual(version_tuple('1.10.0'), (1, 10, 0))
        self.assertTrue(release_metadata(release('1.10.0'), '1.9.9')['available'])
        self.assertFalse(release_metadata(release(CURRENT), CURRENT)['available'])
        self.assertFalse(release_metadata(release(CURRENT), NEXT)['available'])

    def test_versions_reject_prereleases_path_fragments_and_shell_content(self):
        for value in (None, 1, 'v1.0.1', '01.0.1', '1.0', '1.0.1-rc1',
                      '1.0.1/../../evil', '1.0.1;touch /tmp/example', '1.0.1\n'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                version_tuple(value)

    def test_only_stable_public_release_records_are_used(self):
        for value in (None, [], {'draft': True}, {'prerelease': True}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                release_metadata(value, CURRENT)
        for tag in ('1.0.1', 'v../../evil', 'v1.0.1-rc1', None, 123):
            data = release()
            data['tag_name'] = tag
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release_metadata(data, CURRENT)

    def test_installer_must_have_exact_upstream_name_and_url(self):
        for name in ('../../example.deb', '/tmp/example.deb', 'other.deb'):
            data = release()
            data['assets'][0]['name'] = name
            with self.subTest(name=name), self.assertRaises(ValueError):
                release_metadata(data, CURRENT)
        for url in ('https://example.invalid/package.deb',
                    'https://github.com/other/project/releases/download/v1.0.1/openxplorer_1.0.1_all.deb',
                    release()['assets'][0]['browser_download_url'] + '?path=elsewhere',
                    'file:///tmp/example.deb'):
            data = release()
            data['assets'][0]['browser_download_url'] = url
            with self.subTest(url=url), self.assertRaises(ValueError):
                release_metadata(data, CURRENT)

    def test_installer_size_and_digest_are_required(self):
        for size in (None, False, True, 0, -1, MAX_PACKAGE + 1, str(len(PACKAGE))):
            data = release()
            data['assets'][0]['size'] = size
            with self.subTest(size=size), self.assertRaises(ValueError):
                release_metadata(data, CURRENT)
        for digest in (None, '', 123, {'sha256': 'a' * 64}, 'sha256:no',
                       'md5:' + 'a' * 32, 'sha256:' + 'A' * 64):
            data = release()
            data['assets'][0]['digest'] = digest
            with self.subTest(digest=digest), self.assertRaises(ValueError):
                release_metadata(data, CURRENT)

    def test_malformed_asset_list_is_rejected_with_actionable_error(self):
        for assets in (None, {}, 'invalid', 1):
            data = release()
            data['assets'] = assets
            with self.subTest(assets=assets), self.assertRaisesRegex(ValueError, 'asset list'):
                release_metadata(data, CURRENT)

    def test_notes_and_release_link_are_bounded_or_derived_locally(self):
        data = release()
        data['body'] = 'x' * 25000
        data['html_url'] = 'https://example.invalid/untrusted'
        parsed = release_metadata(data, CURRENT)
        self.assertEqual(len(parsed['notes']), 20000)
        self.assertEqual(parsed['releaseUrl'], REPOSITORY + '/releases/tag/v' + NEXT)


class UrlTests(unittest.TestCase):
    def test_only_allowlisted_https_hosts_and_standard_ports_are_accepted(self):
        for host in HOSTS:
            self.assertEqual(trusted_url('https://' + host + '/fixture'), 'https://' + host + '/fixture')
        for url in ('http://github.com/fixture', 'file:///tmp/example',
                    'https://github.com.example.invalid/fixture',
                    'https://user@github.com/fixture', 'https://user:secret@github.com/fixture',
                    'https://github.com:444/fixture', 'https://example.invalid/fixture'):
            with self.subTest(url=url), self.assertRaises(ValueError):
                trusted_url(url)

    def test_redirects_apply_the_same_trust_boundary(self):
        handler = TrustedRedirect()
        request = Request(LATEST)
        target = 'https://release-assets.githubusercontent.com/fixture'
        redirected = handler.redirect_request(request, None, 302, 'Found', {}, target)
        self.assertEqual(redirected.full_url, target)
        with self.assertRaises(ValueError):
            handler.redirect_request(request, None, 302, 'Found', {}, 'https://example.invalid/fixture')

    def test_http_opener_sets_timeout_and_uses_validating_redirect_handler(self):
        with patch('updater.build_opener') as build:
            open_url(LATEST)
            self.assertIsInstance(build.call_args.args[0], TrustedRedirect)
            args, kwargs = build.return_value.open.call_args
            self.assertEqual(args[0].full_url, LATEST)
            self.assertEqual(args[0].get_header('Accept'), 'application/vnd.github+json')
            self.assertEqual(kwargs, {'timeout': 30})
            build.reset_mock()
            with self.assertRaises(ValueError):
                open_url('https://example.invalid/fixture')
            build.assert_not_called()

    def test_install_capability_requires_exact_system_install_and_executable_tools(self):
        with patch('updater.Path.is_file', return_value=True), patch('updater.os.access', return_value=True):
            self.assertTrue(Updater(root='/opt/openxplorer').can_install())
            self.assertFalse(Updater(root='/home/demo/openxplorer').can_install())
        with patch('updater.Path.is_file', return_value=True), patch('updater.os.access', return_value=False):
            self.assertFalse(Updater(root='/opt/openxplorer').can_install())


class UpdaterTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='openxplorer-updater-test-')
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name) / 'updates'
        self.data = release()
        self.payload = PACKAGE
        self.opened = []
        self.commands = []
        self.package_paths = []
        self.opener = Mock(side_effect=self.open_fixture)
        self.run = Mock(side_effect=self.run_fixture)
        self.updater = Updater(current=CURRENT, root='/opt/openxplorer',
                               directory=self.directory, opener=self.opener, run=self.run)
        self.installable = patch.object(self.updater, 'can_install', return_value=True)
        self.installable.start()
        self.addCleanup(self.installable.stop)

    def open_fixture(self, url):
        self.opened.append(url)
        if url == LATEST:
            return io.BytesIO(json.dumps(self.data).encode())
        self.assertEqual(url, release()['assets'][0]['browser_download_url'])
        return io.BytesIO(self.payload)

    def run_fixture(self, args, **kwargs):
        self.commands.append(args)
        self.assertNotIn('shell', kwargs)
        if args[0] == '/usr/bin/dpkg-deb':
            package = Path(args[2])
            self.package_paths.append(package)
            self.assertEqual(package.read_bytes(), PACKAGE)
            self.assertEqual(package.name, f'openxplorer_{NEXT}_all.deb')
            self.assertEqual(package.parent.parent, self.directory)
            self.assertEqual(stat.S_IMODE(package.stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE(package.parent.stat().st_mode), 0o700)
            self.assertEqual(args, ['/usr/bin/dpkg-deb', '-f', str(package), 'Package', 'Version', 'Architecture'])
            return subprocess.CompletedProcess(args, 0,
                f'Package: openxplorer\nVersion: {NEXT}\nArchitecture: all\n', '')
        if args[0] == '/usr/bin/pkexec':
            self.assertEqual(args, ['/usr/bin/pkexec', '/usr/bin/apt-get', '-y', '--no-remove',
                                    'install', str(self.package_paths[-1])])
            self.assertNotIn('timeout', kwargs)
            self.assertTrue(self.package_paths[-1].is_file())
            return subprocess.CompletedProcess(args, 0, '', '')
        if args[0] == '/usr/bin/dpkg-query':
            self.assertEqual(args, ['/usr/bin/dpkg-query', '-W', '-f=${Status}\n${Version}', 'openxplorer'])
            return subprocess.CompletedProcess(args, 0, 'install ok installed\n' + NEXT, '')
        self.fail('Unexpected command: ' + repr(args))

    def checked(self):
        return self.updater.check()

    def assert_idle_and_clean(self):
        self.assertFalse(self.updater.busy)
        self.assertTrue(self.updater.lock.acquire(blocking=False))
        self.updater.lock.release()
        if self.directory.exists():
            self.assertEqual(list(self.directory.iterdir()), [])
        self.assertTrue(all(not path.exists() for path in self.package_paths))

    def test_check_uses_fixed_endpoint_and_only_returns_public_metadata(self):
        value = self.checked()
        self.assertEqual(self.opened, [LATEST])
        self.assertTrue(value['available'])
        self.assertTrue(value['canInstall'])
        self.assertFalse(value['restartRequired'])
        self.assertEqual(value['version'], NEXT)
        for key in ('url', 'sha256', 'size', 'name'):
            self.assertNotIn(key, value)
        self.run.assert_not_called()
        self.assert_idle_and_clean()

    def test_success_verifies_bytes_and_metadata_before_admin_prompt_and_cleans_up(self):
        self.checked()
        progress = []
        result = self.updater.install(NEXT, True, progress.append)
        self.assertEqual(result, {'installed': True, 'version': NEXT})
        self.assertEqual(self.updater.installed_version, NEXT)
        self.assertEqual([call[0] for call in self.commands],
                         ['/usr/bin/dpkg-deb', '/usr/bin/pkexec', '/usr/bin/dpkg-query'])
        self.assertEqual(len(progress), 3)
        self.assertEqual(stat.S_IMODE(self.directory.stat().st_mode), 0o700)
        self.assert_idle_and_clean()

    def test_install_requires_literal_confirmation_before_any_network_or_process(self):
        self.checked()
        self.opener.reset_mock()
        for confirmed in (False, None, 1, 'true', [], {}):
            with self.subTest(confirmed=confirmed), self.assertRaisesRegex(ValueError, 'Confirm'):
                self.updater.install(NEXT, confirmed)
        self.opener.assert_not_called()
        self.run.assert_not_called()
        self.assert_idle_and_clean()

    def test_install_requires_matching_previously_checked_newer_version(self):
        with self.assertRaisesRegex(ValueError, 'Check for updates'):
            self.updater.install(NEXT, True)
        self.checked()
        self.opener.reset_mock()
        for version in (CURRENT, '../../example.deb', '/tmp/example.deb',
                        '--allow-unauthenticated', '1.0.1; echo example', {'url': 'https://example.invalid'}):
            with self.subTest(version=version), self.assertRaises(ValueError):
                self.updater.install(version, True)
        self.updater.release['available'] = False
        with self.assertRaises(ValueError):
            self.updater.install(NEXT, True)
        self.opener.assert_not_called()
        self.run.assert_not_called()
        self.assert_idle_and_clean()

    def test_source_build_cannot_install(self):
        self.checked()
        self.opener.reset_mock()
        self.updater.can_install.return_value = False
        with self.assertRaisesRegex(ValueError, 'installed Debian package'):
            self.updater.install(NEXT, True)
        self.opener.assert_not_called()
        self.run.assert_not_called()
        self.assert_idle_and_clean()

    def test_bad_checksum_short_download_and_oversized_download_never_prompt(self):
        self.checked()
        for payload in (b'X' * len(PACKAGE), PACKAGE[:-1], PACKAGE + b'oversize'):
            self.payload = payload
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                self.updater.install(NEXT, True)
            self.run.assert_not_called()
            self.assertIsNone(self.updater.installed_version)
            self.assert_idle_and_clean()

    def test_wrong_package_identity_version_or_architecture_never_prompt(self):
        self.checked()
        for fields in ({'Package': 'different', 'Version': NEXT, 'Architecture': 'all'},
                       {'Package': 'openxplorer', 'Version': CURRENT, 'Architecture': 'all'},
                       {'Package': 'openxplorer', 'Version': NEXT, 'Architecture': 'amd64'},
                       {'Package': 'openxplorer', 'Version': NEXT}):
            self.run.reset_mock()
            self.run.side_effect = lambda args, **kwargs: subprocess.CompletedProcess(
                args, 0, ''.join(f'{key}: {value}\n' for key, value in fields.items()), '')
            with self.subTest(fields=fields), self.assertRaisesRegex(ValueError, 'metadata'):
                self.updater.install(NEXT, True)
            self.assertEqual(self.run.call_count, 1)
            self.assertEqual(self.run.call_args.args[0][0], '/usr/bin/dpkg-deb')
            self.assert_idle_and_clean()

    def test_download_failure_cleans_partial_files_and_releases_lock(self):
        self.checked()
        class Interrupted(io.BytesIO):
            def read(self, size=-1):
                if self.tell():
                    raise URLError('Fictional disconnect')
                return super().read(10)
        self.opener.side_effect = lambda url: Interrupted(PACKAGE)
        with self.assertRaises(URLError):
            self.updater.install(NEXT, True)
        self.run.assert_not_called()
        self.assert_idle_and_clean()

    def test_polkit_cancellation_or_apt_failure_cleans_up_without_marking_installed(self):
        self.checked()
        for code in (126, 127, 100):
            def fail_install(args, **kwargs):
                if args[0] == '/usr/bin/pkexec':
                    return subprocess.CompletedProcess(args, code, '', 'Fictional refusal')
                return self.run_fixture(args, **kwargs)
            self.run.reset_mock()
            self.run.side_effect = fail_install
            with self.subTest(code=code), self.assertRaisesRegex(ValueError, 'cancelled or failed'):
                self.updater.install(NEXT, True)
            self.assertEqual(self.run.call_count, 2)
            self.assertIsNone(self.updater.installed_version)
            self.assert_idle_and_clean()

    def test_package_inspection_failure_cleans_up_and_does_not_prompt(self):
        self.checked()
        self.run.side_effect = subprocess.CalledProcessError(2, '/usr/bin/dpkg-deb')
        with self.assertRaises(subprocess.CalledProcessError):
            self.updater.install(NEXT, True)
        self.assertEqual(self.run.call_count, 1)
        self.assert_idle_and_clean()

    def test_installed_version_is_verified_after_package_manager_returns_success(self):
        self.checked()
        def wrong_installed_version(args, **kwargs):
            if args[0] == '/usr/bin/dpkg-query':
                return subprocess.CompletedProcess(args, 0, 'install ok installed\n' + CURRENT, '')
            return self.run_fixture(args, **kwargs)
        self.run.side_effect = wrong_installed_version
        with self.assertRaisesRegex(ValueError, 'expected installed version'):
            self.updater.install(NEXT, True)
        self.assertIsNone(self.updater.installed_version)
        self.assert_idle_and_clean()

    def test_failed_check_discards_stale_release_and_releases_lock(self):
        self.checked()
        for error in (HTTPError(LATEST, 403, 'Fictional rate limit', None, None),
                      URLError('Fictional offline state'), TimeoutError()):
            self.opener.side_effect = error
            with self.subTest(error=type(error).__name__), self.assertRaises(ValueError):
                self.updater.check()
            self.assertIsNone(self.updater.release)
            self.assert_idle_and_clean()

    def test_oversized_or_invalid_metadata_response_fails_and_releases_lock(self):
        for payload in (b'x' * (2 * 1024 * 1024 + 1), b'not JSON'):
            self.opener.side_effect = lambda url: io.BytesIO(payload)
            with self.subTest(size=len(payload)), self.assertRaises(ValueError):
                self.updater.check()
            self.assertIsNone(self.updater.release)
            self.assert_idle_and_clean()

    def test_concurrent_tasks_are_rejected_without_network_or_processes(self):
        self.updater.lock.acquire()
        try:
            with self.assertRaisesRegex(ValueError, 'already running'):
                self.updater.check()
            with self.assertRaisesRegex(ValueError, 'already running'):
                self.updater.install(NEXT, True)
        finally:
            self.updater.lock.release()
        self.opener.assert_not_called()
        self.run.assert_not_called()
        self.assert_idle_and_clean()


class BridgeTests(unittest.TestCase):
    """Run actual dispatcher/closing methods without importing GTK or WebKit."""
    @classmethod
    def setUpClass(cls):
        path = Path(__file__).resolve().parents[1] / 'winspace.py'
        tree = ast.parse(path.read_text())
        cls.methods = []
        for node in tree.body:
            wanted = {'OpenXplorerWindow': {'dispatch', 'on_delete'},
                      'OpenXplorer': {'quit_safely', 'create_window'}}.get(getattr(node, 'name', ''), set())
            if wanted:
                cls.methods.extend(copy.deepcopy(method) for method in node.body
                                   if isinstance(method, ast.FunctionDef) and method.name in wanted)

    def setUp(self):
        self.runtime = {'version': CURRENT, 'build': 'fictional-current-build'}
        self.identity = Mock(return_value=self.runtime)
        self.popen = Mock()
        self.scope = {'VERSION': CURRENT, 'ROOT': Path('/opt/openxplorer'),
                      'RUNTIME': self.runtime, 'identity': self.identity,
                      'subprocess': SimpleNamespace(Popen=self.popen)}
        exec(compile(ast.Module(body=self.methods, type_ignores=[]), 'updater-bridge-test', 'exec'), self.scope)
        self.app = SimpleNamespace(update_busy=False, update_restart_required=False,
            updater=SimpleNamespace(check=Mock(), install=Mock(return_value={'installed': True, 'version': NEXT}),
                                    installed_version=NEXT),
            controllers=[], tab_transfers=SimpleNamespace(pending={}), broadcast=Mock(),
            quit=Mock(), bus_endpoint=None, service_held=False)
        self.first = self.controller()
        self.second = self.controller()

    def controller(self):
        controller = SimpleNamespace(app=self.app, closed=False, jobs={}, writes=0,
            mount_ops={}, handoff=None, respond=Mock(), emit=Mock(), start_worker=Mock(),
            environment=Mock(return_value={'fictional': True}), file_clipboard=None)
        controller.window = SimpleNamespace(close=Mock(side_effect=lambda: setattr(controller, 'closed', True)))
        self.app.controllers.append(controller)
        return controller

    def dispatch(self, method, args=None, controller=None):
        return self.scope['dispatch'](controller or self.first,
            {'id': 10, 'method': method, 'args': args or {}})

    def queue_install(self, **extra):
        self.dispatch('updateInstall', {'version': NEXT, 'confirmed': True, **extra})
        self.assertTrue(self.app.update_busy)
        self.assertTrue(self.first.start_worker.call_args.kwargs['write'])
        return self.first.start_worker.call_args.args[1]

    def test_running_update_blocks_file_actions_and_other_updates_in_every_window(self):
        self.app.update_busy = True
        for controller in self.app.controllers:
            for method in ('list', 'search', 'operate', 'clipboardPaste', 'updateCheck',
                           'updateInstall', 'updateRestart', 'environment', 'quit'):
                with self.subTest(method=method), self.assertRaisesRegex(ValueError, 'update is running'):
                    self.dispatch(method, controller=controller)
            controller.start_worker.assert_not_called()
        self.app.updater.install.assert_not_called()
        self.app.updater.check.assert_not_called()

    def test_installed_update_blocks_files_but_allows_status_without_new_check(self):
        self.app.update_restart_required = True
        for controller in self.app.controllers:
            for method in ('list', 'search', 'operate', 'clipboardPaste', 'updateInstall'):
                with self.subTest(method=method), self.assertRaisesRegex(ValueError, 'Restart OpenXplorer'):
                    self.dispatch(method, controller=controller)
        self.dispatch('environment')
        self.first.environment.assert_called_once_with()
        self.dispatch('updateCheck')
        status = self.first.respond.call_args.args[1]
        self.assertTrue(status['restartRequired'])
        self.assertFalse(status['canInstall'])
        self.assertFalse(status['available'])
        self.app.updater.check.assert_not_called()

    def test_confirmation_is_required_before_worker_or_busy_state(self):
        for confirmed in (None, False, 1, 'true'):
            with self.subTest(confirmed=confirmed), self.assertRaisesRegex(ValueError, 'Confirm'):
                self.dispatch('updateInstall', {'version': NEXT, 'confirmed': confirmed})
        self.assertFalse(self.app.update_busy)
        self.first.start_worker.assert_not_called()
        self.app.updater.install.assert_not_called()

    def test_active_work_in_any_window_blocks_install(self):
        for field, value in (('jobs', {'fixture-job': object()}), ('writes', 1),
                             ('mount_ops', {1: object()}), ('handoff', object())):
            original = getattr(self.second, field)
            setattr(self.second, field, value)
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'Wait for file operations'):
                self.dispatch('updateInstall', {'version': NEXT, 'confirmed': True})
            setattr(self.second, field, original)
        self.app.tab_transfers.pending['fixture-transfer'] = object()
        with self.assertRaisesRegex(ValueError, 'tab moves'):
            self.dispatch('updateInstall', {'version': NEXT, 'confirmed': True})
        self.assertFalse(self.app.update_busy)
        self.first.start_worker.assert_not_called()

    def test_queued_install_blocks_second_window_before_worker_starts(self):
        worker = self.queue_install()
        with self.assertRaisesRegex(ValueError, 'update is running'):
            self.dispatch('search', controller=self.second)
        worker(None)
        self.assertFalse(self.app.update_busy)

    def test_bridge_only_passes_version_confirmation_and_progress_not_remote_commands(self):
        worker = self.queue_install(url='https://example.invalid/evil.deb', path='/tmp/evil.deb',
                                    command='touch /tmp/fictional', sha256='0' * 64)
        worker(None)
        args, kwargs = self.app.updater.install.call_args
        self.assertEqual(args[:2], (NEXT, True))
        self.assertEqual(len(args), 3)
        self.assertEqual(kwargs, {})
        args[2]('Fictional progress')
        self.app.broadcast.assert_called_once_with('updateProgress', {'message': 'Fictional progress'})
        self.popen.assert_not_called()

    def test_modified_installed_files_require_restart_after_success_or_failure(self):
        self.identity.return_value = {'version': CURRENT, 'build': 'fictional-replacement-build'}
        worker = self.queue_install()
        worker(None)
        self.assertFalse(self.app.update_busy)
        self.assertTrue(self.app.update_restart_required)
        self.app.update_restart_required = False
        self.app.updater.install.side_effect = ValueError('Fictional failed configuration')
        worker = self.queue_install()
        with self.assertRaisesRegex(ValueError, 'failed configuration'):
            worker(None)
        self.assertFalse(self.app.update_busy)
        self.assertTrue(self.app.update_restart_required)

    def test_identity_read_failure_always_clears_busy_and_requires_restart(self):
        self.identity.side_effect = OSError('Fictional read failure')
        worker = self.queue_install()
        self.assertEqual(worker(None), {'installed': True, 'version': NEXT})
        self.assertFalse(self.app.update_busy)
        self.assertTrue(self.app.update_restart_required)

    def test_worker_submission_failure_clears_app_busy(self):
        self.first.start_worker.side_effect = RuntimeError('Fictional unavailable worker')
        with self.assertRaisesRegex(RuntimeError, 'unavailable worker'):
            self.dispatch('updateInstall', {'version': NEXT, 'confirmed': True})
        self.assertFalse(self.app.update_busy)
        self.app.updater.install.assert_not_called()

    def test_quit_and_window_close_refuse_while_update_runs(self):
        self.app.update_busy = True
        self.assertFalse(self.scope['quit_safely'](self.app))
        self.assertTrue(self.scope['on_delete'](self.first))
        self.app.quit.assert_not_called()
        for controller in self.app.controllers:
            controller.window.close.assert_not_called()
            self.assertFalse(controller.closed)

    def test_pending_restart_allows_safe_quit(self):
        self.app.update_restart_required = True
        self.assertTrue(self.scope['quit_safely'](self.app))
        self.app.quit.assert_called_once_with()
        self.assertTrue(all(controller.closed for controller in self.app.controllers))

    def test_new_windows_are_blocked_during_update_and_until_restart(self):
        for field in ('update_busy', 'update_restart_required'):
            setattr(self.app, field, True)
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'Finish the application update'):
                self.scope['create_window'](self.app)
            setattr(self.app, field, False)

    def test_restart_requires_pending_update_idle_writes_and_fixed_launcher(self):
        with self.assertRaisesRegex(ValueError, 'No installed update'):
            self.dispatch('updateRestart')
        self.app.update_restart_required = True
        self.second.writes = 1
        with self.assertRaisesRegex(ValueError, 'file operations'):
            self.dispatch('updateRestart')
        self.popen.assert_not_called()
        self.second.writes = 0
        self.dispatch('updateRestart', {'path': '/tmp/evil', 'command': 'evil'})
        self.popen.assert_called_once_with(['/usr/bin/openxplorer', '--restart'], start_new_session=True)


if __name__ == '__main__':
    unittest.main()
