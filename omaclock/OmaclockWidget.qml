import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui
import "Config.js" as Config

// Extra time zones in the bar, meant to sit right after omarchy.clock:
//
//   Sat 26 Sep 12:41   IL 19:41  ·  NYC 05:41
//
// Which zones, and how each is written, is ~/.config/omaclock/omaclock.yaml
// (a copy of the omaclock.yaml that ships in this folder is made the first
// time the widget loads). The bar follows edits to it as they are saved.
//
//   left click    refresh now
//   right click   open the YAML file in your editor
//   hover         the full date and time for each zone
//
// The times come from `date`, one call per minute for all the zones: QML has
// no time zone database of its own, and `date` already knows daylight saving.
BarWidget {
  id: root
  moduleName: "iluxav.omaclock"

  readonly property string pluginDir: Qt.resolvedUrl(".").toString().replace(/^file:\/\//, "").replace(/\/$/, "")
  readonly property string configDir: (Quickshell.env("XDG_CONFIG_HOME") || (Quickshell.env("HOME") + "/.config")) + "/omaclock"
  readonly property string configPath: root.configDir + "/omaclock.yaml"

  property var config: Config.defaults()
  property string configError: ""
  // One entry per zone, in file order: what the bar shows, and the tooltip.
  property var shown: []
  property var details: []

  readonly property string separator: root.vertical ? "\n" : root.config.separator
  readonly property string displayText: {
    if (root.configError !== "") return "omaclock ⚠"
    var parts = []
    for (var i = 0; i < root.shown.length; i++) parts.push(root.shown[i])
    return parts.join(root.separator)
  }
  readonly property string tooltip: root.configError !== ""
    ? "omaclock: " + root.configError + "\nRight-click to edit " + root.configPath
    : root.details.join("\n")

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  function loadConfig(source) {
    try {
      root.config = Config.parse(source)
      root.configError = ""
    } catch (e) {
      root.configError = e.message
    }
    root.refresh()
  }

  // Ask `date` for every zone in one go. Zones go in as positional arguments,
  // three per zone (tz, bar format, tooltip format), never as command text.
  function refresh() {
    if (root.configError !== "" || root.config.zones.length === 0) {
      root.shown = []
      root.details = []
      return
    }
    var args = ["omaclock"]
    for (var i = 0; i < root.config.zones.length; i++) {
      var z = root.config.zones[i]
      args.push(z.tz, z.format, z.tooltipFormat)
    }
    dateProc.command = ["bash", "-c", dateProc.script].concat(args)
    dateProc.running = false
    dateProc.running = true
  }

  function accept(text) {
    var rows = text.split("\n")
    var shown = [], details = []
    for (var i = 0; i < root.config.zones.length && i < rows.length; i++) {
      var z = root.config.zones[i]
      var cols = rows[i].split("\t")
      var time = cols[0]
      shown.push(z.label === "" ? time : z.label + " " + time)
      details.push((z.label === "" ? z.tz : z.label + " · " + z.tz) + "   " + (cols.length > 1 ? cols[1] : time))
    }
    root.shown = shown
    root.details = details
  }

  function editConfig() {
    if (root.bar) root.bar.run("omarchy-launch-editor '" + root.configPath + "'")
  }

  SystemClock {
    id: minute
    precision: SystemClock.Minutes
    onDateChanged: root.refresh()
  }

  Process {
    id: dateProc
    // A name with no zoneinfo file would make `date` fall back to UTC without
    // a word, so it is checked first and shown as "??" instead.
    readonly property string script: "while [ $# -ge 3 ]; do\n"
      + "  if [ -f \"/usr/share/zoneinfo/$1\" ]; then\n"
      + "    printf '%s\\t%s\\n' \"$(TZ=\"$1\" date \"+$2\")\" \"$(TZ=\"$1\" date \"+$3\")\"\n"
      + "  else\n"
      + "    printf '??\\tunknown time zone %s\\n' \"$1\"\n"
      + "  fi\n"
      + "  shift 3\n"
      + "done"
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.accept(text)
    }
  }

  FileView {
    id: userConfig
    path: root.configPath
    watchChanges: true
    printErrors: false
    onLoaded: root.loadConfig(text())
    onFileChanged: reload()
    // No file of their own yet: give them a copy of the default to edit.
    onLoadFailed: installConfig.running = true
  }

  Process {
    id: installConfig
    command: ["bash", "-c", "mkdir -p \"$1\" && cp -n \"$2\" \"$1/omaclock.yaml\"", "omaclock", root.configDir, root.pluginDir + "/omaclock.yaml"]
    onExited: userConfig.reload()
  }

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.displayText
    hasVisualContent: root.displayText !== ""
    horizontalMargin: 8.75
    verticalPadding: 8.75
    tooltipText: root.tooltip

    onPressed: function(b) {
      if (b === Qt.RightButton) root.editConfig()
      else root.refresh()
    }
  }
}
