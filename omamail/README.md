# Omamail

Gmail in the Omarchy bar: an envelope with your unread count, a dropdown of the
unread emails, and a viewer that renders them, HTML and all.

- **Left click** the envelope for the unread emails. Click one to read it; that
  marks it read, as Gmail does. Middle-click one to open it in Gmail instead.
- **Right click** to check now (it checks by itself every two minutes).
- **Middle click** to open Gmail in the browser.

## Set up

1. Turn on 2-Step Verification for your Google account, then make an app
   password at <https://myaccount.google.com/apppasswords>.
2. Click the envelope (it is dimmed until set up) and follow the prompts, or run
   `./omamail setup` yourself.

The password is kept in the system keyring (`secret-tool`, under service
`omamail`), never in a file. The address goes in `~/.config/omamail/config.json`.

## Install

```bash
ln -s ~/Projects/omamail ~/.config/omarchy/plugins/iluxav.omamail
```

Then add `{ "id": "iluxav.omamail" }` to a section of the bar in
`~/.config/omarchy/shell.json`, and float the viewer with a Hyprland rule in
`~/.config/hypr/hyprland.lua`:

```lua
o.window("^omamail$", {
  tag = "-default-opacity",
  float = true,
  size = { 900, 1000 },
  opacity = "1 1",
  move = { "(monitor_w-window_w-40)", "(monitor_h*0.04)" },
})
```

Needs `python-gobject`, `webkit2gtk-4.1` and `libsecret` (all on Omarchy).

## Settings

| Setting    | Default | Meaning                                  |
|------------|---------|------------------------------------------|
| `interval` | `120`   | Seconds between checks (at least 30).    |
| `limit`    | `15`    | How many unread emails the dropdown lists. |

```json
{ "id": "iluxav.omamail", "interval": 60, "limit": 10 }
```

## How it fits together

- `MailWidget.qml`: the bar icon and the dropdown.
- `omamail`: talks to Gmail over IMAP (`setup`, `list`, `open UID`), answers in JSON.
- `viewer.py`: one email in a WebKitGTK window. It is a separate window because
  the Omarchy shell cannot host a web view.

The viewer has no GTK title bar: a slim header in the current Omarchy theme's
colours and font shows the subject and sender, with **Open in Gmail** and a
close button. Images load; JavaScript is off, and links open in your normal
browser. **Esc** closes it.
