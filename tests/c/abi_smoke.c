#include <stdio.h>
#include <wchar.h>
#include "winsparkle.h"

static int __cdecl read_config(const char *name, wchar_t *buf, size_t len, void *data) {
    (void)name; (void)data;
    if (len < 2) return 0;
    buf[0] = L'0'; buf[1] = 0; return 1;
}
static void __cdecl write_config(const char *name, const wchar_t *value, void *data) {(void)name;(void)value;(void)data;}
static void __cdecl delete_config(const char *name, void *data) {(void)name;(void)data;}
static void __cdecl callback(void) {}
static int __cdecl can_shutdown(void) {return 0;}
static int __cdecl installer(const wchar_t *path) {(void)path;return 1;}
int main(void) {
    win_sparkle_config_methods_t config = {read_config,write_config,delete_config,NULL};
    win_sparkle_set_config_methods(&config);
#ifdef WIN_SPARKLE_THEME_AUTO
    if (win_sparkle_set_theme(WIN_SPARKLE_THEME_AUTO) != 1) return 3;
    if (win_sparkle_set_theme(WIN_SPARKLE_THEME_LIGHT) != 1) return 4;
    if (win_sparkle_set_theme(WIN_SPARKLE_THEME_DARK) != 1) return 5;
    if (win_sparkle_set_theme(-1) != 0) return 6;
    if (win_sparkle_set_theme(WIN_SPARKLE_THEME_AUTO) != 1) return 7;
#endif
    win_sparkle_set_app_details(L"ABI test",L"Existing C consumer",L"1.0");
    win_sparkle_set_lang("it"); win_sparkle_set_langid(0x410);
    win_sparkle_set_app_build_version(L"100");
    win_sparkle_set_appcast_url("https://example.org/appcast.xml");
    win_sparkle_set_http_header("X-Test","C ABI"); win_sparkle_clear_http_headers();
    win_sparkle_set_registry_path("Software\\WinSparkle-NG\\Tests");
    win_sparkle_set_error_callback(callback);win_sparkle_set_can_shutdown_callback(can_shutdown);
    win_sparkle_set_shutdown_request_callback(callback);win_sparkle_set_did_find_update_callback(callback);
    win_sparkle_set_did_not_find_update_callback(callback);win_sparkle_set_update_cancelled_callback(callback);
    win_sparkle_set_update_skipped_callback(callback);win_sparkle_set_update_postponed_callback(callback);
    win_sparkle_set_update_dismissed_callback(callback);win_sparkle_set_user_run_installer_callback(installer);
    win_sparkle_set_automatic_check_for_updates(0);win_sparkle_set_update_check_interval(3600);
    if (win_sparkle_get_automatic_check_for_updates()!=0) return 2;
    (void)win_sparkle_get_update_check_interval();(void)win_sparkle_get_last_check_time();
    win_sparkle_init(); win_sparkle_init(); win_sparkle_cleanup(); win_sparkle_cleanup();
    puts("WinSparkle C header and import library: OK");return 0;
}
