# omaclock

Extra time zones in the Omarchy bar, right next to the clock:

```
Sat 26 Sep 11:56   IL 21:56  ·  NYC 14:56
```

The zones are listed in a small YAML file. Daylight saving is handled for you.

## Install

```bash
ln -s ~/Projects/omaclock ~/.config/omarchy/plugins/iluxav.omaclock
omarchy-shell shell rescanPlugins
omarchy plugin enable iluxav.omaclock
omarchy bar move iluxav.omaclock --section center --index 2   # right after omarchy.clock
```

(`--index 2` assumes the clock is at index 1 of the center section; adjust if yours differs.)

## Configure

The first time the widget loads it copies `omaclock.yaml` to
`~/.config/omaclock/omaclock.yaml`. Edit that copy; the bar follows as soon as you
save. Right-click the widget to open it in your editor.

```yaml
separator: "  ·  "     # between zones
format: "%H:%M"        # strftime codes, as in `date +...`

zones:
  - label: IL
    tz: Asia/Jerusalem
  - label: NYC
    tz: America/New_York
    format: "%I:%M %p"
  - { label: LON, tz: Europe/London }
```

| key             | where     | meaning                                                        |
| --------------- | --------- | -------------------------------------------------------------- |
| `tz`            | zone      | required. IANA name (`timedatectl list-timezones`)             |
| `label`         | zone      | shown before the time; defaults to the city in `tz`            |
| `format`        | both      | strftime format; a zone's own overrides the top-level one      |
| `separator`     | top level | text between zones                                             |
| `tooltipFormat` | top level | what hover shows, default `%A %d %B %H:%M (%Z)`                |

A typo in the file shows `omaclock ⚠` in the bar; hover it for the line number.

## Interaction

- left click: refresh now
- right click: edit the config
- hover: full date and time for each zone

## How it works

`OmaclockWidget.qml` asks `date` for every zone once a minute (QML has no time zone
database of its own). Zones are passed to the shell as arguments, never as command
text, and each name is checked against `/usr/share/zoneinfo` first. `Config.js` is a
reader for the small slice of YAML the file uses, and reports errors with line numbers.
