"""Real DLL integration tests. No installer is executed; the host callback consumes it."""
from __future__ import annotations
import base64
import ctypes as C
from ctypes import wintypes as W
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import re
import ssl
import sys
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'bindings/python'))
from winsparkle import WinSparkle
from winsparkle import Theme
LIBRARY = Path(os.environ.get('WINSPARKLE_DLL', ROOT / 'target/x86_64-pc-windows-msvc/debug/WinSparkle.dll'))
U = C.WinDLL('user32', use_last_error=True)
U.GetDlgItem.argtypes, U.GetDlgItem.restype = [W.HWND, C.c_int], W.HWND
U.GetDlgItemTextW.argtypes = [W.HWND, C.c_int, W.LPWSTR, C.c_int]
U.IsWindowVisible.argtypes = [W.HWND]
U.PostMessageW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM]
U.GetWindowThreadProcessId.argtypes = [W.HWND, C.POINTER(W.DWORD)]
ENUM = C.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
U.EnumWindows.argtypes = [ENUM, W.LPARAM]

def wait_for(predicate, timeout=10):
    deadline = time.monotonic()+timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.02)
    raise AssertionError('Timed out waiting for updater state')

def window():
    found = []
    @ENUM
    def visit(hwnd, _):
        pid = W.DWORD()
        U.GetWindowThreadProcessId(hwnd, C.byref(pid))
        if pid.value == os.getpid() and U.GetDlgItem(hwnd, 201):
            found.append(hwnd)
        return True
    U.EnumWindows(visit, 0)
    return found[0] if found else None

def text(control):
    hwnd = window()
    if not hwnd:
        return ''
    buffer = C.create_unicode_buffer(8192)
    U.GetDlgItemTextW(hwnd, control, buffer, len(buffer))
    return buffer.value

def command(control):
    hwnd = wait_for(window)
    if not U.PostMessageW(hwnd, 0x111, control, 0):
        raise OSError(C.get_last_error())

def control_colors(control=202):
    """Query the live dialog's actual text/background painting contract."""
    hwnd = wait_for(window)
    child = U.GetDlgItem(hwnd, control)
    U.GetDC.argtypes, U.GetDC.restype = [W.HWND], W.HDC
    U.ReleaseDC.argtypes = [W.HWND, W.HDC]
    U.SendMessageW.argtypes, U.SendMessageW.restype = [W.HWND, W.UINT, W.WPARAM, W.LPARAM], C.c_ssize_t
    gdi = C.WinDLL('gdi32')
    gdi.GetBkColor.argtypes = gdi.GetTextColor.argtypes = [W.HDC]
    gdi.GetBkColor.restype = gdi.GetTextColor.restype = W.DWORD
    dc = U.GetDC(child)
    try:
        U.SendMessageW(hwnd, 0x138, dc, child)  # WM_CTLCOLORSTATIC (including read-only notes)
        return gdi.GetBkColor(dc), gdi.GetTextColor(dc)
    finally:
        U.ReleaseDC(child, dc)

def high_contrast_enabled():
    class HighContrast(C.Structure):
        _fields_ = [('size', W.UINT), ('flags', W.DWORD), ('scheme', W.LPWSTR)]
    value = HighContrast()
    value.size = C.sizeof(value)
    U.SystemParametersInfoW.argtypes = [W.UINT, W.UINT, C.c_void_p, W.UINT]
    return bool(U.SystemParametersInfoW(0x42, value.size, C.byref(value), 0) and value.flags & 1)

class ProgressMessages:
    """Observe messages on the actual UI thread without replacing its window procedure."""
    class Call(C.Structure):
        _fields_ = [('lparam', W.LPARAM), ('wparam', W.WPARAM),
                    ('message', W.UINT), ('hwnd', W.HWND)]
    Hook = C.WINFUNCTYPE(C.c_ssize_t, C.c_int, W.WPARAM, W.LPARAM)

    def __enter__(self):
        self.messages = []
        hwnd = wait_for(window)
        bar = U.GetDlgItem(hwnd, 205)
        thread = U.GetWindowThreadProcessId(hwnd, None)
        U.SetWindowsHookExW.argtypes = [C.c_int, self.Hook, W.HINSTANCE, W.DWORD]
        U.SetWindowsHookExW.restype = W.HANDLE
        U.CallNextHookEx.argtypes = [W.HANDLE, C.c_int, W.WPARAM, W.LPARAM]
        U.CallNextHookEx.restype = C.c_ssize_t
        U.UnhookWindowsHookEx.argtypes = [W.HANDLE]
        @self.Hook
        def observe(code, wparam, lparam):
            if code >= 0:
                message = C.cast(lparam, C.POINTER(self.Call)).contents
                if message.hwnd == bar and message.message in (0x18, 0x402, 0x40A):
                    self.messages.append((message.message, message.wparam))
            return U.CallNextHookEx(None, code, wparam, lparam)
        self.callback = observe
        self.handle = U.SetWindowsHookExW(4, self.callback, None, thread)  # WH_CALLWNDPROC
        if not self.handle:
            raise OSError(C.get_last_error())
        return self.messages

    def __exit__(self, *_):
        if not U.UnhookWindowsHookEx(self.handle):
            raise OSError(C.get_last_error())

def owned_native_threads():
    kernel=C.WinDLL('kernel32',use_last_error=True)
    class Entry(C.Structure):
        _fields_=[('size',W.DWORD),('usage',W.DWORD),('tid',W.DWORD),('pid',W.DWORD),('priority',W.LONG),('delta',W.LONG),('flags',W.DWORD)]
    kernel.CreateToolhelp32Snapshot.argtypes,kernel.CreateToolhelp32Snapshot.restype=[W.DWORD,W.DWORD],W.HANDLE
    kernel.OpenThread.argtypes,kernel.OpenThread.restype=[W.DWORD,W.BOOL,W.DWORD],W.HANDLE
    kernel.GetThreadDescription.argtypes=[W.HANDLE,C.POINTER(C.c_void_p)]
    kernel.Thread32First.argtypes=kernel.Thread32Next.argtypes=[W.HANDLE,C.POINTER(Entry)]
    kernel.CloseHandle.argtypes=[W.HANDLE];kernel.LocalFree.argtypes=[C.c_void_p]
    snapshot=kernel.CreateToolhelp32Snapshot(4,0);names=[]
    try:
        entry=Entry();entry.size=C.sizeof(entry)
        success=kernel.Thread32First(snapshot,C.byref(entry))
        while success:
            if entry.pid==os.getpid():
                handle=kernel.OpenThread(0x0800,False,entry.tid)
                if handle:
                    pointer=C.c_void_p()
                    try:
                        if kernel.GetThreadDescription(handle,C.byref(pointer))==0 and pointer:
                            name=C.wstring_at(pointer)
                            if name.startswith('WinSparkle '):names.append(name)
                    finally:
                        if pointer:kernel.LocalFree(pointer)
                        kernel.CloseHandle(handle)
            success=kernel.Thread32Next(snapshot,C.byref(entry))
    finally:kernel.CloseHandle(snapshot)
    return names

PAYLOAD = b'WinSparkle integration test payload\n' * 65536
class Handler(BaseHTTPRequestHandler):
    feed = b''
    slow = False
    headers_seen = []
    def log_message(self, *_): pass
    def do_GET(self):
        type(self).headers_seen.append(dict(self.headers))
        if self.path == '/redirect':
            self.send_response(302); self.send_header('Location', '/feed'); self.end_headers(); return
        data = type(self).feed if self.path == '/feed' else PAYLOAD
        self.send_response(200)
        if self.path != '/unknown.exe': self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        try:
            for start in range(0, len(data), 16384):
                self.wfile.write(data[start:start+16384])
                if type(self).slow and self.path != '/feed': time.sleep(.03)
        except (ConnectionError, OSError): pass

class Integration(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(('127.0.0.1',0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = f'http://127.0.0.1:{cls.server.server_port}'
    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown(); cls.server.server_close(); cls.thread.join()
    def setUp(self):
        self.ws = WinSparkle(LIBRARY)
        # Each test deliberately configures every persistent pre-init property it uses.
        self.ws.configure('WinSparkle tests','Lumen Notes','1.0',self.base+'/feed',lang='it',one_click=False)
        self.values = {'CheckForUpdates':'0'}
        self.ws.use_settings(self.values)
        Handler.slow = False; Handler.headers_seen.clear()
        self.events = []
        for event in ('did_find_update','did_not_find_update','error','update_cancelled','update_skipped','update_postponed','update_dismissed','shutdown_request'):
            self.ws.on(event, lambda e=event: self.events.append(e))
        self.ws.on('can_shutdown', lambda: 1)
        self.installed = []
        self.ws.on('user_run_installer', self.installer)
        self.feed()
    def tearDown(self):
        self.ws.close(); self.ws.close()
        self.assertIsNone(window(), 'cleanup left the UI thread/window alive')
        self.assertEqual(owned_native_threads(),[], 'cleanup left owned native threads alive')
        # Fixture callbacks consume the payload; remove only this test's validated direct temp child.
        saved=self.values.get('UpdateTempDir')
        if saved:
            directory=Path(saved)
            suffix=directory.name.removeprefix('WinSparkle-NG-')
            if directory.parent==Path(tempfile.gettempdir()) and re.fullmatch(r'[0-9a-f]{32}',suffix) and not directory.is_symlink() and directory.is_dir():
                for child in directory.iterdir():
                    if child.is_file() and not child.is_symlink():child.unlink()
                directory.rmdir()
    def installer(self, path):
        self.assertEqual(Path(path).read_bytes(), PAYLOAD)
        self.installed.append(path)
        return 1
    def feed(self, version='2.0', signature='', enclosure=True, critical=False, url='/payload.exe'):
        Handler.feed = f'''<rss xmlns:s="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item>
        <title>Lumen Notes {version}</title><description><![CDATA[<h2>Lumen Notes {version}</h2><p>A faster, more reliable update.</p><ul><li>Improved note organization</li><li>Fixes and new features</li></ul>]]></description>
        {'<s:criticalUpdate/>' if critical else ''}<s:version>{version}</s:version>
        <s:releaseNotesLink>https://example.org/notes</s:releaseNotesLink>
        {f'<enclosure url="{self.base+url}" s:edSignature="{signature}"/>' if enclosure else '<link>https://example.org/download</link>'}
        </item></channel></rss>'''.encode()
    def available(self):
        self.ws.check()
        wait_for(lambda: text(207) in ('Scarica','Scarica e installa','Apri pagina di download'))
    def test_01_exports_and_settings(self):
        names = re.findall(r'WIN_SPARKLE_API\s+\w+\s+__cdecl\s+(win_sparkle_\w+)\s*\(', (ROOT/'include/winsparkle.h').read_text())
        for name in names: self.assertTrue(hasattr(self.ws.dll,name), name)
        self.ws.dll.win_sparkle_set_update_check_interval(10)
        self.assertEqual(self.ws.dll.win_sparkle_get_update_check_interval(),3600)
        self.assertEqual(self.ws.dll.win_sparkle_get_last_check_time(),-1)
        self.ws.init(); self.ws.init()
    def test_02_no_update_redirect_header(self):
        self.feed('1.0'); self.ws.dll.win_sparkle_set_appcast_url((self.base+'/redirect').encode())
        self.ws.dll.win_sparkle_set_http_header(b'X-Integration',b'Lumen Notes')
        self.ws.check(); wait_for(lambda: 'did_not_find_update' in self.events)
        self.assertTrue(any(h.get('X-Integration') == 'Lumen Notes' for h in Handler.headers_seen))
        self.assertGreater(self.ws.dll.win_sparkle_get_last_check_time(),0)
    def test_03_legacy_two_step(self):
        self.available(); command(207); wait_for(lambda: text(207) == 'Installa ora')
        self.assertEqual(self.installed,[])
        command(207); wait_for(lambda: 'shutdown_request' in self.events)
        self.assertEqual(len(self.installed),1)
    def test_04_one_click_default(self):
        self.ws.configure('WinSparkle tests','Lumen Notes','1.0',self.base+'/feed',lang='it')
        self.available(); self.assertEqual(text(207),'Scarica e installa'); command(207)
        wait_for(lambda: 'shutdown_request' in self.events); self.assertEqual(len(self.installed),1)
    def test_05_automatic_install_veto_retry(self):
        self.ws.on('can_shutdown',lambda: 0)
        self.ws.check(install=True); wait_for(lambda: text(207) == 'Riprova installazione')
        self.assertEqual(self.installed,[]); self.assertNotIn('shutdown_request',self.events)
        self.ws.on('can_shutdown',lambda: 1); command(207)
        wait_for(lambda: 'shutdown_request' in self.events)
    def test_06_cancel_download(self):
        Handler.slow = True
        self.available(); command(207); wait_for(lambda: 'Download' in text(201)); command(2)
        wait_for(lambda: 'update_cancelled' in self.events)
        self.assertIn('update_dismissed',self.events); self.assertEqual(self.installed,[])
    def test_07_skip_and_postpone(self):
        self.available(); command(209); wait_for(lambda: 'update_skipped' in self.events)
        self.assertEqual(self.values['SkipThisVersion'],'2.0')
        self.ws.check(background=True); wait_for(lambda: 'did_not_find_update' in self.events)
        self.available(); command(208); wait_for(lambda: 'update_postponed' in self.events)
    def test_08_missing_enclosure_and_critical(self):
        self.feed(enclosure=False,critical=True); self.available()
        self.assertFalse(U.IsWindowVisible(U.GetDlgItem(window(),209)))
        self.assertEqual(text(207),'Apri pagina di download')
    def test_09_cleanup_during_check_and_download(self):
        for _ in range(3):
            self.ws.check(); self.ws.dll.win_sparkle_cleanup()
            self.assertIsNone(window())
        Handler.slow = True
        self.available(); command(207); wait_for(lambda: 'Download' in text(201))
        start = time.monotonic(); self.ws.dll.win_sparkle_cleanup()
        self.assertLess(time.monotonic()-start,5); self.assertEqual(self.installed,[])
    def test_10_launch_failure_retains_download(self):
        calls = []
        def launch(path):
            calls.append(path)
            return -1 if len(calls) == 1 else 1
        self.ws.on('user_run_installer',launch); self.ws.check(install=True)
        wait_for(lambda: text(207) == 'Riprova installazione')
        self.assertNotIn('shutdown_request',self.events); self.assertTrue(Path(calls[0]).exists())
        command(207); wait_for(lambda: 'shutdown_request' in self.events)
        self.assertEqual(calls[0],calls[1])
    def test_11_unknown_content_length(self):
        self.feed(url='/unknown.exe'); self.ws.check(install=True)
        wait_for(lambda: 'shutdown_request' in self.events)
    def test_13_language_fallback(self):
        self.ws.dll.win_sparkle_set_lang(b'it-XX')
        self.available()
        self.assertEqual(text(207),'Scarica')
    def test_14_cleanup_from_callback(self):
        done = threading.Event()
        def found():
            self.ws.dll.win_sparkle_cleanup()
            done.set()
        self.ws.on('did_find_update',found)
        self.ws.check()
        self.assertTrue(done.wait(5))
        wait_for(lambda: window() is None)
        self.assertEqual(self.installed,[])
    def test_15_untrusted_tls_certificate(self):
        try:
            from cryptography import x509
            from cryptography.x509.oid import NameOID
            from cryptography.hazmat.primitives import hashes, serialization
            from cryptography.hazmat.primitives.asymmetric import rsa
            from datetime import datetime, timedelta, timezone
        except ImportError: self.skipTest('cryptography required for TLS fixture')
        key = rsa.generate_private_key(public_exponent=65537,key_size=2048)
        subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,'localhost')])
        now = datetime.now(timezone.utc)
        cert = (x509.CertificateBuilder().subject_name(subject).issuer_name(subject).public_key(key.public_key())
                .serial_number(1).not_valid_before(now-timedelta(days=1)).not_valid_after(now+timedelta(days=1))
                .add_extension(x509.SubjectAlternativeName([x509.DNSName('localhost')]),critical=False).sign(key,hashes.SHA256()))
        with tempfile.TemporaryDirectory(prefix='winsparkle-tls-test-') as directory:
            pem = Path(directory)/'fixture.pem'
            pem.write_bytes(cert.public_bytes(serialization.Encoding.PEM)+key.private_bytes(serialization.Encoding.PEM,serialization.PrivateFormat.PKCS8,serialization.NoEncryption()))
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); context.load_cert_chain(pem)
            class TLSServer(ThreadingHTTPServer):
                def handle_error(self, request, address): pass  # Expected TLS rejection can reset the accepted socket.
            server = TLSServer(('127.0.0.1',0),Handler)
            server.socket = context.wrap_socket(server.socket,server_side=True)
            thread = threading.Thread(target=server.serve_forever,daemon=True); thread.start()
            try:
                self.ws.dll.win_sparkle_set_appcast_url(f'https://localhost:{server.server_port}/feed'.encode())
                self.ws.check(); wait_for(lambda:'error' in self.events)
                self.assertNotIn('did_find_update',self.events);self.assertEqual(self.installed,[])
            finally:
                server.shutdown();server.server_close();thread.join()
    def test_16_rtl_resources(self):
        self.ws.dll.win_sparkle_set_lang(b'ar')
        self.ws.check();wait_for(lambda:'did_find_update' in self.events)
        U.GetWindowLongW.argtypes, U.GetWindowLongW.restype = [W.HWND,C.c_int],C.c_long
        self.assertTrue(U.GetWindowLongW(window(),-20)&0x00400000)
        self.assertIn('ت',text(1000) or text(209))
    def test_17_download_progress_does_not_restart(self):
        Handler.slow = True
        self.available()
        with ProgressMessages() as messages:
            command(207)
            wait_for(lambda: text(207) == 'Installa ora')
        positions = [value for message, value in messages if message == 0x402]
        self.assertEqual(positions[0], 0, 'download did not start at zero')
        self.assertGreater(len(positions), 3, 'expected multiple download progress updates')
        self.assertEqual(positions, sorted(positions), 'download progress moved backwards')
        self.assertEqual(positions[-1], 1000)
        self.assertNotIn((0x40A, 1), messages, 'known-length download briefly started marquee animation')
        self.assertLess(messages.index((0x402, 0)), messages.index((0x18, 1)),
                        'progress must be reset before the bar is shown')
        stops = [value for message, value in messages if message == 0x40A and value == 0]
        self.assertEqual(len(stops), 1, 'marquee was stopped repeatedly, restarting the fill animation')

    def test_18_unknown_length_progress_keeps_marquee(self):
        Handler.slow = True
        self.feed(url='/unknown.exe')
        self.available()
        with ProgressMessages() as messages:
            command(207)
            wait_for(lambda: text(207) == 'Installa ora')
        self.assertEqual([value for message, value in messages if message == 0x402], [0])
        self.assertEqual(messages.count((0x40A, 1)), 1, 'unknown-length download must keep one continuous marquee')

    def test_19_theme_override_and_live_switch(self):
        U.GetSysColor.argtypes, U.GetSysColor.restype = [C.c_int], W.DWORD
        light = (U.GetSysColor(5), U.GetSysColor(8))
        self.ws.set_theme(Theme.LIGHT)
        self.available()
        hwnd = window()
        self.assertEqual(control_colors(), light)
        for invalid in (-1, 3, 2147483647):
            self.assertEqual(self.ws.dll.win_sparkle_set_theme(invalid), 0)
        self.assertEqual(control_colors(), light)
        self.ws.set_theme('dark')
        dark = light if high_contrast_enabled() else (0x202020, 0xf0f0f0)
        wait_for(lambda: control_colors() == dark)
        if not high_contrast_enabled():
            self.assertEqual(control_colors(204), (0x181818, 0xf0f0f0))
        self.assertEqual(window(), hwnd, 'live theme switch recreated the dialog')
        self.ws.set_theme('light')
        wait_for(lambda: control_colors() == light)
        self.assertNotIn('update_cancelled', self.events)

    def test_20_theme_auto_and_setting_change(self):
        import winreg
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r'Software\Microsoft\Windows\CurrentVersion\Themes\Personalize') as key:
                system_dark = winreg.QueryValueEx(key, 'AppsUseLightTheme')[0] == 0
        except OSError:
            system_dark = False
        U.GetSysColor.argtypes, U.GetSysColor.restype = [C.c_int], W.DWORD
        expected = (0x202020, 0xf0f0f0) if system_dark and not high_contrast_enabled() else (U.GetSysColor(5), U.GetSysColor(8))
        self.available()  # configure defaults to auto, without an explicit theme setter.
        self.assertEqual(control_colors(), expected)
        self.ws.set_theme('light')
        self.ws.set_theme(Theme.AUTO)
        wait_for(lambda: control_colors() == expected)
        U.PostMessageW(window(), 0x1a, 0, 0)  # WM_SETTINGCHANGE; do not modify OS preferences.
        wait_for(lambda: control_colors() == expected)

    def test_21_wrapper_rejects_unknown_theme(self):
        for invalid in ('sepia', '', -1, 3, None):
            with self.assertRaises(ValueError):
                self.ws.set_theme(invalid)

    def test_22_dark_download_progress_and_install(self):
        Handler.slow = True
        self.ws.set_theme('dark')
        self.available()
        with ProgressMessages() as messages:
            command(207)
            wait_for(lambda: text(207) == 'Installa ora')
        positions = [value for message, value in messages if message == 0x402]
        self.assertEqual(positions[0], 0)
        self.assertEqual(positions[-1], 1000)
        self.assertEqual(positions, sorted(positions))
        self.assertNotIn((0x40A, 1), messages)
        command(207)
        wait_for(lambda: 'shutdown_request' in self.events)
        self.assertEqual(len(self.installed), 1)

    def test_23_theme_change_during_download_keeps_progress(self):
        Handler.slow = True
        self.ws.set_theme('dark')
        self.available()
        with ProgressMessages() as messages:
            command(207)
            wait_for(lambda: sum(message == 0x402 for message, _ in messages) >= 4)
            self.ws.set_theme('light')
            self.ws.set_theme('dark')
            wait_for(lambda: text(207) == 'Installa ora')
        positions = [value for message, value in messages if message == 0x402]
        self.assertEqual(positions, sorted(positions))
        self.assertEqual(positions[-1], 1000)
        self.assertEqual(self.installed, [])

    def test_89_signature_without_trusted_key_is_unsigned(self):
        self.feed(signature='not-valid-base64')
        self.ws.check(install=True);wait_for(lambda:'shutdown_request' in self.events)
        self.assertEqual(len(self.installed),1)
    def test_90_signatures(self):
        try:
            from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
            from cryptography.hazmat.primitives import serialization
        except ImportError: self.skipTest('cryptography is required for cross-implementation signature tests')
        signing = Ed25519PrivateKey.from_private_bytes(b'\x07'*32)
        public = base64.b64encode(signing.public_key().public_bytes(serialization.Encoding.Raw,serialization.PublicFormat.Raw))
        self.assertEqual(self.ws.dll.win_sparkle_set_eddsa_public_key(public),1)
        signature = base64.b64encode(signing.sign(PAYLOAD)).decode()
        self.feed(signature=signature); self.ws.check(install=True)
        wait_for(lambda: 'shutdown_request' in self.events)
    def test_91_missing_signature_fails_closed(self):
        public = base64.b64encode(b'\x58'+b'\x66'*31)
        self.assertEqual(self.ws.dll.win_sparkle_set_eddsa_public_key(public),1)
        self.ws.check(install=True)
        wait_for(lambda: 'error' in self.events)
        self.assertEqual(self.installed,[]); self.assertNotIn('shutdown_request',self.events)
        self.assertIn('firma', text(202))

if __name__ == '__main__': unittest.main(verbosity=2)
