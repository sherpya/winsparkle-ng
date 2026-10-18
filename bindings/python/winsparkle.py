"""cdecl binding for WinSparkle-NG, usable unchanged in Nuitka standalone builds."""
from __future__ import annotations
import ctypes as C
from enum import IntEnum
import logging
from pathlib import Path

VOID = C.CFUNCTYPE(None)
CAN_SHUTDOWN = C.CFUNCTYPE(C.c_int)
INSTALLER = C.CFUNCTYPE(C.c_int, C.c_wchar_p)
READ = C.CFUNCTYPE(C.c_int, C.c_char_p, C.POINTER(C.c_wchar), C.c_size_t, C.c_void_p)
WRITE = C.CFUNCTYPE(None, C.c_char_p, C.c_wchar_p, C.c_void_p)
DELETE = C.CFUNCTYPE(None, C.c_char_p, C.c_void_p)

class Theme(IntEnum):
    AUTO = 0
    LIGHT = 1
    DARK = 2

class ConfigMethods(C.Structure):
    _fields_ = [('read', READ), ('write', WRITE), ('delete', DELETE), ('user_data', C.c_void_p)]

class WinSparkle:
    def __init__(self, library: str | Path):
        self.dll = C.CDLL(str(Path(library).resolve()))
        self.callbacks = {}
        self._settings_callbacks = None
        self._initialized = False
        signatures = {
            'init': ([], None), 'cleanup': ([], None),
            'set_app_details': ([C.c_wchar_p]*3, None),
            'set_app_build_version': ([C.c_wchar_p], None),
            'set_lang': ([C.c_char_p], None), 'set_langid': ([C.c_ushort], None),
            'set_appcast_url': ([C.c_char_p], None), 'set_registry_path': ([C.c_char_p], None),
            'set_eddsa_public_key': ([C.c_char_p], C.c_int),
            'set_dsa_pub_pem': ([C.c_char_p], C.c_int),
            'set_http_header': ([C.c_char_p]*2, None), 'clear_http_headers': ([], None),
            'set_config_methods': ([C.POINTER(ConfigMethods)], None),
            'set_install_after_download': ([C.c_int], None),
            'set_automatic_check_for_updates': ([C.c_int], None),
            'get_automatic_check_for_updates': ([], C.c_int),
            'set_update_check_interval': ([C.c_int], None), 'get_update_check_interval': ([], C.c_int),
            'get_last_check_time': ([], C.c_int64),
            'check_update_with_ui': ([], None), 'check_update_without_ui': ([], None),
            'check_update_with_ui_and_install': ([], None),
        }
        for name, (args, result) in signatures.items():
            function = getattr(self.dll, 'win_sparkle_' + name)
            function.argtypes, function.restype = args, result
        self._set_theme = getattr(self.dll, 'win_sparkle_set_theme', None)
        if self._set_theme is not None:
            self._set_theme.argtypes, self._set_theme.restype = [C.c_int], C.c_int
        for event in ('error', 'can_shutdown', 'shutdown_request', 'did_find_update', 'did_not_find_update',
                      'update_cancelled', 'update_skipped', 'update_postponed', 'update_dismissed', 'user_run_installer'):
            function = getattr(self.dll, 'win_sparkle_set_' + event + '_callback')
            function.argtypes, function.restype = [C.c_void_p], None

    def configure(self, company, app, version, feed, *, lang=None, public_key=None, one_click=True, theme='auto'):
        self.dll.win_sparkle_set_app_details(company, app, version)
        self.dll.win_sparkle_set_appcast_url(feed.encode('utf-8'))
        if lang is not None:
            self.dll.win_sparkle_set_lang(lang.encode('utf-8'))
        if public_key is not None and self.dll.win_sparkle_set_eddsa_public_key(public_key.encode('ascii')) != 1:
            raise ValueError('Invalid Ed25519 key')
        self.dll.win_sparkle_set_install_after_download(int(one_click))
        self.set_theme(theme)

    def set_theme(self, theme: str | Theme = Theme.AUTO):
        """Select auto/light/dark before init or while the updater is visible."""
        try:
            mode = Theme[theme.upper()] if isinstance(theme, str) else Theme(theme)
        except (KeyError, ValueError, TypeError) as error:
            raise ValueError('Theme must be auto, light, dark, or a Theme value') from error
        if self._set_theme is None:
            if mode != Theme.AUTO:
                raise NotImplementedError('This DLL does not support theme selection')
        elif self._set_theme(mode) != 1:
            raise ValueError('The DLL rejected the theme mode')

    def on(self, event, callback):
        signature = CAN_SHUTDOWN if event == 'can_shutdown' else INSTALLER if event == 'user_run_installer' else VOID
        def guarded(*args):
            try:
                return callback(*args)
            except Exception:
                logging.exception('WinSparkle callback failed: %s', event)
                return -1 if event == 'user_run_installer' else 0 if event == 'can_shutdown' else None
        pointer = signature(guarded)
        self.callbacks[event] = pointer
        getattr(self.dll, 'win_sparkle_set_' + event + '_callback')(C.cast(pointer, C.c_void_p))

    def use_settings(self, values: dict[str, str]):
        """Optional in-memory settings (also useful to isolate tests from the registry)."""
        def read(name, buffer, length, _):
            value = values.get(name.decode('ascii'))
            if value is None or len(value) + 1 > length:
                return 0
            C.memmove(buffer, C.create_unicode_buffer(value), (len(value)+1)*C.sizeof(C.c_wchar))
            return 1
        callbacks = (READ(read), WRITE(lambda n,v,_: values.__setitem__(n.decode('ascii'), v)),
                     DELETE(lambda n,_: values.pop(n.decode('ascii'), None)))
        self._settings_callbacks = (*callbacks, ConfigMethods(*callbacks, None))
        self.dll.win_sparkle_set_config_methods(C.byref(self._settings_callbacks[-1]))

    def init(self):
        self.dll.win_sparkle_init()
        self._initialized = True

    def check(self, *, background=False, install=False):
        self._initialized = True
        function = 'check_update_with_ui_and_install' if install else 'check_update_without_ui' if background else 'check_update_with_ui'
        getattr(self.dll, 'win_sparkle_' + function)()

    def close(self):
        self.dll.win_sparkle_cleanup()
        self._initialized = False
        for event in self.callbacks:
            getattr(self.dll, 'win_sparkle_set_' + event + '_callback')(None)
        self.callbacks.clear()
        self.dll.win_sparkle_set_config_methods(None)
        self._settings_callbacks = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
