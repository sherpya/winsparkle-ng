"""Generate the reviewed C ABI inventory from the distributed public header."""
from pathlib import Path
import re

root=Path(__file__).resolve().parents[1]
header=(root/'include/winsparkle.h').read_text()
signatures=re.findall(r'WIN_SPARKLE_API\s+(\w+\s+__cdecl\s+win_sparkle_\w+\s*\([^;]*\))\s*;',header,re.S)
behavior={
 'init':'Start one UI/controller thread; first-run consent and periodic scheduling.',
 'cleanup':'Cancel I/O, join owned worker/UI threads, release resources and partial files; idempotent.',
 'set_lang':'Copy ISO locale; takes priority over LANGID. Full/base/English fallback.',
 'set_langid':'Set numeric Windows LANGID if no ISO locale was supplied.',
 'set_appcast_url':'Copy HTTP(S) feed URL; otherwise host FeedURL/APPCAST resource.',
 'set_dsa_pub_pem':'Return 0; retain a fail-closed unsupported DSA policy unless Ed25519 is configured.',
 'set_eddsa_public_key':'Copy Base64 trusted key; return 1 only for valid Ed25519 keys. Failed configuration remains fail-closed.',
 'set_app_details':'Copy UTF-16 company, application and display version; otherwise host VERSIONINFO.',
 'set_app_build_version':'Copy internal comparison version; does not replace display version.',
 'set_http_header':'Copy/replace case-insensitive header; reject CR/LF/NUL injection.',
 'clear_http_headers':'Remove copied custom headers.',
 'set_registry_path':'Copy HKCU/HKLM-relative settings path.',
 'set_config_methods':'Copy all three callbacks and opaque data; NULL restores registry. Incomplete sets are ignored.',
 'set_automatic_check_for_updates':'Write CheckForUpdates as REG_SZ or custom storage.',
 'get_automatic_check_for_updates':'Read CheckForUpdates; default 0.',
 'set_update_check_interval':'Write UpdateInterval; clamp to at least 3600 seconds.',
 'get_update_check_interval':'Read UpdateInterval; default 86400 seconds; clamp malformed lower values.',
 'get_last_check_time':'Read signed 64-bit time_t LastCheckTime; default -1.',
 'check_update_with_ui':'Queue manual check; visible progress and ignore skipped version.',
 'check_update_with_ui_and_install':'Queue manual check; skip initial update prompt and install after download/verification.',
 'check_update_without_ui':'Queue background check; respect skipped version, show update prompt when applicable.',
 'set_install_after_download':'WinSparkle-NG extension; default 1 one-click, 0 opts into legacy two-step installation. Does not change automatic-install API.',
 'set_theme':'WinSparkle-NG extension: 0 auto (default), 1 light, 2 dark. Returns 1 on success, 0 for invalid input without changing the mode. May be changed at runtime; high contrast takes precedence. No host/global theme changes or persisted setting.',
}
text='''# C API compatibility inventory

All entry points retain their undecorated `win_sparkle_*` names and `__cdecl`
calling convention (`extern "C"` in Rust). The original header remains usable;
the distributed header adds `win_sparkle_set_install_after_download` and
`win_sparkle_set_theme` without changing existing signatures or structures.
MSVC time_t defaults to signed 64 bits on both x86 and x64; the getter uses i64.
Pointers and `size_t` retain the architecture's native width. The callback
configuration struct uses `repr(C)` with original field order.

Configuration setters must be called before init, and copy strings/structures
before returning. The host owns callback function code and user_data through
cleanup. Callback setters, settings preferences, and the theme may be changed at runtime.
No pointer returned to the host needs freeing. NULL string inputs are handled
as empty strings. Non-NULL pointers must reference valid terminated storage.

Checks return immediately and coalesce while a check/download/prompt is active.
The UI and host callbacks run on the dedicated updater thread; config methods
can additionally run on the calling API thread. No global mutex is held across
host callbacks. Calls initiated from callbacks can set preferences or request
cleanup; unloading the DLL must wait until the callback has returned and the
updater thread has stopped. Normal external cleanup blocks until teardown.
Every C and Win32 callback boundary catches internal Rust unwinding. Foreign
exceptions and invalid host pointers are outside that boundary contract.

| Signature | Side effects / implemented behavior |
|---|---|
'''
for signature in signatures:
    signature=' '.join(signature.split())
    name=re.search(r'win_sparkle_(\w+)\(',signature.replace(' (','(')).group(1)
    if name.endswith('_callback'):
        event=name.removeprefix('set_').removesuffix('_callback')
        detail=f'Store nullable {event} callback. Invoked without global locks; host owns its lifetime.'
        if event=='user_run_installer': detail+=' 0 delegates to ShellExecuteExW, 1 succeeds, -1/other fails.'
        if event=='can_shutdown': detail+=' 0 retains the downloaded file and offers installation retry.'
        if event=='shutdown_request': detail+=' Called only after successful installer launch.'
    else: detail=behavior[name]
    text+=f'| `{signature}` | {detail} |\n'
text+='''
Intentional differences: DSA unsupported; invalid key configuration never falls
back to unsigned; critical updates cannot be skipped; release notes use safe text
and external links; malformed length/XML/URL inputs are rejected. Numeric version
segments are compared without the original atoi overflow. Original HKCU then HKLM
settings fallback and REG_SZ format are preserved.

Registry values: CheckForUpdates, DidRunOnce, UpdateInterval, LastCheckTime,
SkipThisVersion and UpdateTempDir. Default path:
HKCU\\Software\\<company>\\<application>\\WinSparkle (company omitted if empty).
Only a validated WinSparkle-NG temp directory can be removed via UpdateTempDir.

`tools/abi-smoke.ps1` compiles an actual C executable against the distributed
header and runs it with the DLL. Pass `-HeaderDirectory <path>` to test an external
WinSparkle header. `tools/package.ps1` compares every header export to
the PE export table and audits imports/Cargo dependencies. x86 and x64 executables
were run; the ARM64 DLL and C consumer are cross-linked, with native execution
still requiring Windows ARM64.
'''
(root/'docs/api-compatibility.md').write_text(text,encoding='utf-8',newline='\n')
print(f'Inventoried {len(signatures)} C exports')
