"""Nuitka standalone probe: cdecl, resources, callback lifetime, and cleanup."""
from pathlib import Path
import sys
import threading
from winsparkle import WinSparkle

library = Path(__file__).resolve().with_name('WinSparkle.dll')
event = threading.Event()
with WinSparkle(library) as updater:
    updater.configure('Test','Lumen Notes','1.0','http://127.0.0.1:1/unreachable',lang='it')
    updater.use_settings({'CheckForUpdates':'0'})
    updater.on('error',event.set)
    updater.init()
    updater.check()
    if not event.wait(10):
        raise RuntimeError('Native callback was not delivered')
print('Nuitka standalone: DLL loaded, UI created, callback received, cleanup completed')
