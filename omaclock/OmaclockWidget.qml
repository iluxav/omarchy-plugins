import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons as Commons
import qs.Ui
import "Config.js" as Config
import "Stats.js" as Stats

// Extra time zones and optional system usage, beside omarchy.clock:
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
  property int configRevision: 0
  // One entry per zone, in file order: what the bar shows, and the tooltip.
  property var shown: []
  property var details: []
  property var statsSample: Stats.parse("")
  property var previousCpu: null
  property var cpuUsage: null
  readonly property bool statsEnabled: root.configError === "" && (root.config.showCpu || root.config.showMemory || root.config.showDisk)
  readonly property var statsContent: root.formatStats()

  readonly property string separator: root.vertical ? "\n" : root.config.separator
  readonly property string displayText: {
    if (root.configError !== "") return "omaclock ⚠"
    return root.shown.concat(root.statsContent.shown).join(root.separator)
  }
  readonly property string tooltip: root.configError !== ""
    ? "omaclock: " + root.configError + "\nRight-click to edit " + root.configPath
    : root.details.concat(root.statsContent.details).join("\n")

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  function loadConfig(source) {
    root.configRevision++
    root.shown = []
    root.details = []
    root.statsSample = Stats.parse("")
    root.previousCpu = null
    root.cpuUsage = null
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
    root.refreshClock()
    root.refreshStats()
  }

  function refreshClock() {
    if (root.configError !== "" || root.config.zones.length === 0) {
      root.shown = []
      root.details = []
      return
    }
    if (dateProc.running) return
    var args = ["omaclock"]
    for (var i = 0; i < root.config.zones.length; i++) {
      var z = root.config.zones[i]
      args.push(z.tz, z.format, z.tooltipFormat)
    }
    dateProc.command = ["bash", "-c", dateProc.script].concat(args)
    dateProc.revision = root.configRevision
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
    if (root.bar) root.bar.run("omarchy-launch-editor '" + root.configPath.replace(/'/g, "'\\''") + "'")
  }

  function refreshStats() {
    if (!root.statsEnabled || statsProc.running) return
    statsProc.revision = root.configRevision
    statsProc.command = ["bash", root.pluginDir + "/stats.sh",
      String(root.config.showCpu), String(root.config.showMemory), String(root.config.showDisk), root.config.diskPath]
    statsProc.running = true
  }

  function acceptStats(text) {
    var sample = Stats.parse(text)
    root.cpuUsage = Stats.cpuPercent(root.previousCpu, sample.cpu)
    root.previousCpu = sample.cpu
    root.statsSample = sample
  }

  function percent(value) {
    return value === null ? "--" : Commons.I18n.formatNumber(Math.round(value)) + "%"
  }

  function size(kib) {
    var result = Stats.size(kib)
    return Commons.I18n.formatNumber(result.value, 1) + " " + result.unit
  }

  function usage(sample, format) {
    if (!sample) return "--"
    if (format === "percent") return root.percent(sample.percent)
    // The collector reports KiB; GB is decimal (1,000,000,000 bytes).
    var used = Commons.I18n.formatNumber(sample.used * 1024 / 1000000000, 1)
    var total = Commons.I18n.formatNumber(sample.total * 1024 / 1000000000, 1)
    var text = used + "/" + total + " GB"
    return format === "both" ? text + " (" + root.percent(sample.percent) + ")" : text
  }

  function formatStats() {
    var shown = [], details = []
    if (root.config.showCpu) {
      shown.push("CPU " + root.percent(root.cpuUsage))
      details.push(strings.t("stats.cpu").arg(root.cpuUsage === null ? strings.t("stats.unavailable") : root.percent(root.cpuUsage)))
    }
    if (root.config.showMemory) {
      var mem = root.statsSample.memory
      shown.push("RAM " + root.usage(mem, root.config.memoryFormat))
      details.push(mem ? strings.t("stats.memory").arg(root.size(mem.used)).arg(root.size(mem.total)).arg(root.percent(mem.percent))
        : strings.t("stats.memoryUnavailable"))
    }
    if (root.config.showDisk) {
      var disk = root.statsSample.disk
      shown.push("DISK " + root.usage(disk, root.config.diskFormat))
      details.push(disk ? strings.t("stats.disk").arg(root.config.diskPath).arg(root.size(disk.used)).arg(root.size(disk.total))
        .arg(root.percent(disk.percent)).arg(root.size(disk.available))
        : strings.t("stats.diskUnavailable").arg(root.config.diskPath))
    }
    return { shown: shown, details: details }
  }

  Commons.Translations {
    id: strings
    directory: Qt.resolvedUrl("i18n")
  }

  Timer {
    interval: root.config.statsInterval * 1000
    running: root.statsEnabled
    repeat: true
    onTriggered: root.refreshStats()
  }

  Process {
    id: statsProc
    property int revision: -1
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: if (statsProc.revision === root.configRevision) root.acceptStats(text)
    }
    onExited: if (statsProc.revision !== root.configRevision) Qt.callLater(root.refreshStats)
  }

  SystemClock {
    id: minute
    precision: SystemClock.Minutes
    onDateChanged: root.refreshClock()
  }

  Process {
    id: dateProc
    property int revision: -1
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
      onStreamFinished: if (dateProc.revision === root.configRevision) root.accept(text)
    }
    onExited: if (dateProc.revision !== root.configRevision) Qt.callLater(root.refreshClock)
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
    anchors.fill: root
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
