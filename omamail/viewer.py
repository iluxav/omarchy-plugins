#!/usr/bin/python3
"""The omamail email viewer: one email, rendered by WebKitGTK.

  viewer.py ~/.cache/omamail/<uid>.json

A window of its own rather than part of the bar, because the Omarchy shell
cannot host a web view. The window's app id is "omamail", which a Hyprland
window rule floats in the top-right corner.

There is no GTK title bar: a slim header in the current Omarchy theme's
colours and font shows who it is from, with "Open in Gmail" and a close
button. The email runs with JavaScript off; images load, links open in the
normal browser. Esc closes it.
"""

import json
import os
import subprocess
import sys
import tomllib

# WebKitGTK's DMA-BUF renderer dies with a Wayland protocol error on NVIDIA.
# Must be set before WebKit loads.
os.environ.setdefault("WEBKIT_DISABLE_DMABUF_RENDERER", "1")

import gi  # noqa: E402

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("WebKit2", "4.1")
gi.require_version("Pango", "1.0")
from gi.repository import Gdk, GLib, Gtk, Pango, WebKit2  # noqa: E402

# Pictures, CSS and fonts from anywhere, but never scripts.
POLICY = "default-src 'none'; img-src data: https: http:; style-src 'unsafe-inline' https: http:; font-src data: https: http:"

# Emails are written for a white page, whatever the desktop theme is.
BASE_STYLE = "<style>html{background:#fff;color:#111}body{margin:12px}</style>"

THEME = os.path.join(os.environ.get("XDG_STATE_HOME") or os.path.expanduser("~/.local/state"),
                     "omarchy/current/theme/colors.toml")


def theme():
    colors = {"background": "#121212", "foreground": "#bebebe", "light_foreground": "#8a8a8d",
              "bright_foreground": "#eaeaea", "muted": "#333333", "accent": "#e68e0d",
              "selection": "#2a2a2a"}
    try:
        with open(THEME, "rb") as f:
            colors.update({k: v for k, v in tomllib.load(f).items() if isinstance(v, str)})
    except (OSError, tomllib.TOMLDecodeError):
        pass
    return colors


def font():
    try:
        out = subprocess.run(["omarchy-font-current"], capture_output=True, text=True, timeout=3)
        if out.returncode == 0 and out.stdout.strip():
            return out.stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        pass
    return "monospace"


def style(colors, family):
    c = colors
    return """
    .header {
      background: %(background)s;
      border-bottom: 1px solid %(muted)s;
      padding: 10px 8px 10px 16px;
      font-family: "%(font)s";
    }
    .subject { color: %(bright_foreground)s; font-weight: bold; font-size: 15px; }
    .sender { color: %(light_foreground)s; font-size: 12px; }
    .action {
      background: none;
      border: none;
      box-shadow: none;
      color: %(foreground)s;
      font-family: "%(font)s";
      font-size: 12px;
      padding: 4px 10px;
      border-radius: 4px;
    }
    .action:hover { background: %(selection)s; color: %(accent)s; }
    .close { font-size: 16px; }
    """ % dict(c, font=family)


def build_page(markup):
    meta = '<meta http-equiv="Content-Security-Policy" content="%s">' % POLICY
    return meta + '<meta charset="utf-8">' + BASE_STYLE + markup


def open_outside(uri):
    subprocess.Popen(["xdg-open", uri], stdin=subprocess.DEVNULL,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=True)


def label(text, css, ellipsize=True):
    widget = Gtk.Label(xalign=0)
    widget.set_text(text)
    widget.get_style_context().add_class(css)
    if ellipsize:
        widget.set_ellipsize(Pango.EllipsizeMode.END)
    return widget


def button(text, css, tooltip, on_click):
    widget = Gtk.Button(label=text)
    widget.set_relief(Gtk.ReliefStyle.NONE)
    widget.set_tooltip_text(tooltip)
    widget.set_valign(Gtk.Align.CENTER)
    for name in css:
        widget.get_style_context().add_class(name)
    widget.connect("clicked", lambda _b: on_click())
    return widget


def main(path):
    with open(path) as f:
        mail = json.load(f)
    # The email is in memory now; don't leave a copy of it lying in ~/.cache.
    try:
        os.remove(path)
    except OSError:
        pass

    GLib.set_prgname("omamail")
    GLib.set_application_name("omamail")

    provider = Gtk.CssProvider()
    provider.load_from_data(style(theme(), font()).encode())
    Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider,
                                             Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

    window = Gtk.Window(title=mail.get("subject", "omamail"))
    window.set_default_size(900, 1000)
    # Hyprland draws the border; the header below stands in for a title bar.
    window.set_decorated(False)

    # Subject over "Sender <address>  ·  date", then the buttons.
    header = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
    header.get_style_context().add_class("header")

    who = mail.get("from", "")
    if mail.get("address") and mail["address"] != who:
        who += " <%s>" % mail["address"]
    titles = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
    titles.pack_start(label(mail.get("subject", ""), "subject"), False, False, 0)
    titles.pack_start(label("  ·  ".join(x for x in (who, mail.get("date", "")) if x), "sender"), False, False, 0)
    header.pack_start(titles, True, True, 0)

    header.pack_end(button("×", ["action", "close"], "Close (Esc)", window.close), False, False, 0)
    if mail.get("link"):
        header.pack_end(button("Open in Gmail", ["action"], "Open this email in Gmail",
                               lambda: open_outside(mail["link"])), False, False, 0)

    settings = WebKit2.Settings()
    settings.set_enable_javascript(False)
    settings.set_javascript_can_open_windows_automatically(False)
    settings.set_enable_page_cache(False)
    view = WebKit2.WebView.new_with_settings(settings)

    def on_policy(_view, decision, kind):
        if kind == WebKit2.PolicyDecisionType.NAVIGATION_ACTION:
            action = decision.get_navigation_action()
            if action.get_navigation_type() == WebKit2.NavigationType.LINK_CLICKED:
                open_outside(action.get_request().get_uri())
                decision.ignore()
                return True
        elif kind == WebKit2.PolicyDecisionType.NEW_WINDOW_ACTION:
            open_outside(decision.get_navigation_action().get_request().get_uri())
            decision.ignore()
            return True
        return False

    view.connect("decide-policy", on_policy)
    view.load_html(build_page(mail.get("html", "")), None)

    def on_key(_w, event):
        if event.keyval == Gdk.KEY_Escape:
            window.close()
            return True
        return False

    window.connect("key-press-event", on_key)
    window.connect("destroy", Gtk.main_quit)

    body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
    body.pack_start(header, False, False, 0)
    body.pack_start(view, True, True, 0)
    window.add(body)
    window.show_all()
    Gtk.main()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__.strip(), file=sys.stderr)
        sys.exit(2)
    main(sys.argv[1])
