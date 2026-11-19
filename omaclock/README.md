# omaclock

Extra time zones and optional CPU, memory, and disk usage in the Omarchy bar:

```
Sat 26 Sep 11:56   IL 21:56  ·  NYC 14:56  ·  CPU 12%  ·  RAM 38%  ·  DISK 61%
```

Configure the zones and usage readings in a small YAML file. Daylight saving
is handled for you. Usage readings are off by default.

## Install

```bash
ln -s ~/Projects/omarchy-plugins/omaclock ~/.config/omarchy/plugins/iluxav.omaclock
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
separator: "  ·  "     # between zones and usage readings
format: "%H:%M"        # strftime codes, as in `date +...`
showCpu: true
showMemory: true
showDisk: true
memoryFormat: both     # used/total GB and percentage
diskFormat: both       # percent, used-total or both
statsInterval: 5        # seconds between usage samples
diskPath: "/"          # filesystem containing this path

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
| `separator`     | top level | text between zones and usage readings                          |
| `tooltipFormat` | top level | what hover shows, default `%A %d %B %H:%M (%Z)`                |
| `showCpu`       | top level | show total CPU usage, `true` or `false` (default `false`)       |
| `showMemory`    | top level | show RAM usage, `true` or `false` (default `false`)             |
| `showDisk`      | top level | show filesystem space used, `true` or `false` (default `false`) |
| `memoryFormat`  | top level | RAM display: `percent`, `used-total` or `both` (default `percent`) |
| `diskFormat`    | top level | disk display: `percent`, `used-total` or `both` (default `percent`) |
| `statsInterval` | top level | seconds between usage samples, integer 1–3600 (default `5`)    |
| `diskPath`      | top level | absolute path on the filesystem to measure (default `/`)       |

Usage readings appear after the time zones, in CPU, RAM, disk order. Set
`zones: []` to show only system usage. Hover for memory and disk used/total
amounts and available disk space. `diskPath: "/home"` measures the filesystem
containing `/home`, not the size of that directory. Disk usage is capacity,
not read/write activity.

Set `memoryFormat: both` and `diskFormat: both` for readings such as
`RAM 16.0/64.0 GB (25%)` and `DISK 250.0/1000.0 GB (25%)`. The first number
is used space, the second is total capacity. `used-total` omits the percentage.
Bar amounts use decimal GB (1 GB = 1,000,000,000 bytes), with one decimal place;
hover amounts use automatically scaled binary units such as GiB and TiB.

CPU usage is averaged across all cores between samples; it shows `--` until
the second sample arrives. Memory usage is total minus available memory,
so reclaimable cache is not counted as consumed RAM. Disk percentages match
`df`, including filesystem reserved-space handling. Missing readings show
`--`; a bad disk path does not stop CPU or memory updates.

A typo in the file shows `omaclock ⚠` in the bar; hover it for the line number.

After updating the plugin's source code, run `omarchy restart shell` if a newly
added setting reports `unknown key`. Quickshell can retain an older JavaScript
config reader even after `omarchy-shell shell rescanPlugins`. Restarting the
shell loads the updated reader. Ordinary YAML changes still reload on save.

## Interaction

- left click: refresh now
- right click: edit the config
- hover: full date and time for each zone, plus detailed system usage

## How it works

`OmaclockWidget.qml` asks `date` for every zone once a minute (QML has no time zone
database of its own). Zones are passed to the shell as arguments, never as command
text, and each name is checked against `/usr/share/zoneinfo` first. `Config.js` is a
reader for the small slice of YAML the file uses, and reports errors with line numbers.

When enabled, `stats.sh` reads `/proc/stat` and `/proc/meminfo`, and runs `df`
for the selected filesystem at `statsInterval`. The path is passed as an
argument. Disk queries time out after two seconds, and polls never overlap.
No usage polling runs when all three readings are disabled. This uses Bash
and GNU coreutils, already included with Omarchy.

## Check changes

Run `node --test omaclock/tests/test.cjs` from the repository root to check
configuration validation, usage calculations, and the collector on Linux.
