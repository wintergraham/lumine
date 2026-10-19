// Genuine CEF deployment diagnostic; this does not exercise Ask production.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <limits.h>
#include <X11/Xlib.h>
#include "include/capi/cef_app_capi.h"
#include "include/capi/cef_browser_capi.h"
#include "include/capi/cef_client_capi.h"
#include "include/internal/cef_string.h"
#include "include/cef_version_info.h"
static int refs = 1;
static void add_ref(cef_base_ref_counted_t *self) { ++refs; }
static int release(cef_base_ref_counted_t *self) { return --refs == 0; }
static int one_ref(cef_base_ref_counted_t *self) { return refs == 1; }
static int any_ref(cef_base_ref_counted_t *self) { return refs > 0; }
static void set_string(cef_string_t *out, const char *text) { cef_string_utf8_to_utf16(text, strlen(text), out); }
int main(int argc, char **argv) {
  const char *runtime = getenv("ASK_CEF_SMOKE_RUNTIME");
  const char *profile = getenv("ASK_CEF_SMOKE_PROFILE");
  if (!runtime || !profile) { fprintf(stderr, "Use scripts/check-cef-linux.py\n"); return 2; }
  cef_api_hash(15400, 0);
  cef_main_args_t args = {.argc=argc, .argv=argv};
  int child = cef_execute_process(&args, NULL, NULL);
  if (child >= 0) return child;
  printf("CEF_VERSION");
  for (int i=0; i<8; ++i) printf(" %d", cef_version_info(i));
  printf("\n"); fflush(stdout);
  char locales[PATH_MAX], log_file[PATH_MAX];
  if (snprintf(locales, sizeof(locales), "%s/locales", runtime) >= sizeof(locales) ||
      snprintf(log_file, sizeof(log_file), "%s/chromium.log", profile) >= sizeof(log_file)) return 2;
  cef_settings_t settings = {.size=sizeof(settings), .external_message_pump=1, .remote_debugging_port=9228};
  set_string(&settings.resources_dir_path, runtime);
  set_string(&settings.locales_dir_path, locales);
  set_string(&settings.root_cache_path, profile);
  set_string(&settings.log_file, log_file);
  if (!cef_initialize(&args, &settings, NULL, NULL)) { fprintf(stderr, "CEF_INITIALIZE_FAILED\n"); return 2; }
  printf("CEF_INITIALIZED sandbox=enabled api=15400\n"); fflush(stdout);
  Display *display = XOpenDisplay(NULL);
  if (!display) return 3;
  Window parent = XCreateSimpleWindow(display, DefaultRootWindow(display), 0, 0, 640, 480, 0, 0, 0);
  XMapWindow(display, parent); XSync(display, False);
  cef_window_info_t info = {.size=sizeof(info), .parent_window=parent, .bounds={0, 0, 640, 480}, .runtime_style=CEF_RUNTIME_STYLE_ALLOY};
  cef_browser_settings_t options = {.size=sizeof(options)};
  static cef_client_t client;
  client.base = (cef_base_ref_counted_t) {.size=sizeof(client), .add_ref=add_ref, .release=release, .has_one_ref=one_ref, .has_at_least_one_ref=any_ref};
  cef_string_t url = {};
  set_string(&url, "data:text/html,<html><title>Ask Linux CEF smoke</title><body>Native X11 child</body></html>");
  cef_browser_t *browser = cef_browser_host_create_browser_sync(&info, &client, &url, &options, NULL, NULL);
  if (!browser) { fprintf(stderr, "CEF_BROWSER_CREATE_FAILED\n"); return 4; }
  cef_browser_host_t *host = browser->get_host(browser);
  for (int i=0; i<1500; ++i) { cef_do_message_loop_work(); usleep(10000); }
  Window child_window = host->get_window_handle(host);
  Window root_return, parent_return, *children;
  unsigned int count;
  if (!XQueryTree(display, child_window, &root_return, &parent_return, &children, &count)) return 5;
  if (children) XFree(children);
  printf("CEF_NATIVE_CHILD xid=%lu parent=%lu expected_parent=%lu loading=%d\n", child_window, parent_return, parent, browser->is_loading(browser)); fflush(stdout);
  int valid = child_window && parent_return == parent && !browser->is_loading(browser);
  host->close_browser(host, 1);
  host->base.release(&host->base);
  browser->base.release(&browser->base);
  for (int i=0; i<100; ++i) { cef_do_message_loop_work(); usleep(10000); }
  cef_shutdown();
  XCloseDisplay(display);
  return valid ? 0 : 6;
}
