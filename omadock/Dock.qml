import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Commons
import qs.Ui

// omadock: an always-visible bar of your own shortcuts down one side of every
// screen, drawn with the top bar's own buttons, colors and size so the two
// read as one piece of chrome.
//
// The shortcuts are ~/.config/omadock/omadock.toml (a copy of the omadock.toml
// that ships in this folder is made the first time the dock loads). The dock
// follows edits to it as they are saved.
//
//   left click    focus the app's window, or start it (click = "new": always start one)
//   middle click  start a new one
//   right click   open the TOML file in your editor
//
// It reserves its width like the top bar does, so windows tile beside it.
Item {
  id: root

  property var shell: null
  property var manifest: null

  readonly property string pluginDir: String(Qt.resolvedUrl(".")).replace(/^file:\/\//, "").replace(/\/$/, "")
  readonly property string configDir: (Quickshell.env("XDG_CONFIG_HOME") || (Quickshell.env("HOME") + "/.config")) + "/omadock"
  readonly property string configPath: root.configDir + "/omadock.toml"

  property string position: "left"
  // Buttons before the first [[spacer]] sit at the top, the rest at the bottom.
  property var startItems: []
  property var endItems: []
  property string configError: ""

  readonly property int barSize: Style.bar.sizeVertical

  // Every open window, so buttons can mark and focus their app. Touching
  // `values` in bindings keeps them live as windows come and go.
  readonly property var toplevels: ToplevelManager.toplevels.values

  // ---- config -------------------------------------------------------------

  // Just enough TOML for this file: comments, top-level keys, [[app]] /
  // [[separator]] / [[spacer]] tables, and key = "basic" | 'literal' strings.
  function parseConfig(source) {
    var out = { position: "left", items: [] }
    var current = null
    var lines = String(source || "").split("\n")
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i].trim()
      if (!line || line.charAt(0) === "#") continue
      var table = line.match(/^\[\[\s*(app|separator|spacer)\s*\]\]$/)
      if (table) {
        current = { kind: table[1] }
        out.items.push(current)
        continue
      }
      var kv = line.match(/^([A-Za-z_][A-Za-z0-9_-]*)\s*=\s*(.*)$/)
      if (!kv) throw new Error("line " + (i + 1) + ": " + line)
      var value = parseString(kv[2], i + 1)
      if (current) current[kv[1]] = value
      else out[kv[1]] = value
    }
    return out
  }

  function parseString(raw, lineNo) {
    var s = raw.trim()
    if (s.charAt(0) === "'") {
      var end = s.indexOf("'", 1)
      if (end < 0) throw new Error("line " + lineNo + ": unclosed '")
      return s.substring(1, end)
    }
    if (s.charAt(0) === "\"") {
      var value = ""
      for (var i = 1; i < s.length; i++) {
        var c = s.charAt(i)
        if (c === "\"") return value
        if (c === "\\" && i + 1 < s.length) {
          var n = s.charAt(++i)
          value += n === "n" ? "\n" : n === "t" ? "\t" : n
        } else value += c
      }
      throw new Error("line " + lineNo + ": unclosed \"")
    }
    throw new Error("line " + lineNo + ": values must be quoted strings")
  }

  function loadConfig(source) {
    try {
      var cfg = parseConfig(source)
      var start = [], end = [], target = start
      for (var i = 0; i < cfg.items.length; i++) {
        var item = cfg.items[i]
        if (item.kind === "spacer") { target = end; continue }
        if (item.kind === "app") {
          if (!item.run) throw new Error("app \"" + (item.name || "?") + "\" has no run command")
          item.matcher = makeMatcher(item["class"])
        }
        target.push(item)
      }
      root.position = cfg.position === "right" ? "right" : "left"
      root.startItems = start
      root.endItems = end
      root.configError = ""
    } catch (e) {
      root.configError = e.message
      root.startItems = []
      root.endItems = []
    }
  }

  function makeMatcher(pattern) {
    if (!pattern) return null
    try {
      var re = new RegExp(pattern, "i")
      return function(appId) { return re.test(appId) }
    } catch (e) {
      throw new Error("class \"" + pattern + "\": " + e.message)
    }
  }

  // ---- actions ------------------------------------------------------------

  function windowsFor(item, all) {
    if (!item.matcher) return []
    var found = []
    for (var i = 0; i < all.length; i++) {
      if (item.matcher(String(all[i].appId || ""))) found.push(all[i])
    }
    return found
  }

  // Focus the app if it has a window; when one is already focused, move on
  // to its next window, so repeated clicks cycle through them.
  function activate(item) {
    var open = windowsFor(item, ToplevelManager.toplevels.values)
    if (open.length === 0) {
      launch(item)
      return
    }
    var next = 0
    for (var i = 0; i < open.length; i++) {
      if (open[i].activated) { next = (i + 1) % open.length; break }
    }
    open[next].activate()
  }

  function launch(item) {
    Util.execDetached(item.run)
  }

  function editConfig() {
    Util.execArgv(["omarchy-launch-editor", root.configPath])
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
    command: ["bash", "-c", "mkdir -p \"$1\" && cp -n \"$2\" \"$1/omadock.toml\"", "omadock", root.configDir, root.pluginDir + "/omadock.toml"]
    onExited: userConfig.reload()
  }

  // ---- surface ------------------------------------------------------------

  Variants {
    model: Quickshell.screens

    PanelWindow {
      id: dock

      required property var modelData
      screen: modelData

      // What the qs.Ui bar buttons expect to find on their `bar`: the same
      // colors, font and geometry as a vertical top bar, and a tooltip.
      property var tooltipTarget: null
      property string tooltipText: ""
      readonly property QtObject barApi: QtObject {
        readonly property color barForeground: Color.bar.text
        readonly property color foreground: Color.bar.text
        readonly property color urgent: Color.bar.active
        readonly property string fontFamily: Style.font.family
        readonly property bool vertical: true
        readonly property int barSize: root.barSize
        readonly property bool foregroundAnimationEnabled: true
        function showTooltip(target, text) {
          dock.tooltipText = String(text || "")
          dock.tooltipTarget = dock.tooltipText !== "" ? target : null
        }
        function hideTooltip(target) {
          if (dock.tooltipTarget === target) dock.tooltipTarget = null
        }
      }

      anchors {
        top: true
        bottom: true
        left: root.position === "left"
        right: root.position === "right"
      }
      implicitWidth: root.barSize
      exclusiveZone: root.barSize
      color: Color.bar.background
      surfaceFormat.opaque: false
      WlrLayershell.namespace: "omadock"
      WlrLayershell.layer: WlrLayer.Top
      WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

      Column {
        anchors.top: parent.top
        anchors.topMargin: Style.space(8)
        anchors.horizontalCenter: parent.horizontalCenter
        Repeater {
          model: root.configError !== "" ? [{ kind: "error" }] : root.startItems
          delegate: DockEntry { bar: dock.barApi }
        }
      }

      Column {
        anchors.bottom: parent.bottom
        anchors.bottomMargin: Style.space(8)
        anchors.horizontalCenter: parent.horizontalCenter
        Repeater {
          model: root.endItems
          delegate: DockEntry { bar: dock.barApi }
        }
      }

      PopupWindow {
        id: tooltipWindow

        visible: dock.tooltipTarget !== null
        color: "transparent"
        implicitWidth: Math.ceil(bubble.implicitWidth)
        implicitHeight: Math.ceil(bubble.implicitHeight)

        anchor {
          id: tooltipAnchor
          window: dock
          adjustment: PopupAdjustment.Slide
          edges: Edges.Top | Edges.Left
          gravity: Edges.Bottom | Edges.Right
          rect.width: 1
          rect.height: 1

          onAnchoring: {
            var target = dock.tooltipTarget
            if (!target) return
            var w = tooltipWindow.implicitWidth
            var x = root.position === "left" ? target.width + 6 : -w - 6
            var y = target.height / 2 - tooltipWindow.implicitHeight / 2
            var point = dock.contentItem.mapFromItem(target, x, y)
            tooltipAnchor.rect.x = Math.round(point.x)
            tooltipAnchor.rect.y = Math.round(point.y)
          }
        }

        BorderSurface {
          id: bubble
          implicitWidth: label.implicitWidth + 20
          implicitHeight: label.implicitHeight + 14
          color: Color.tooltip.background
          borderSpec: Border.surfaceSpec("tooltip", "border", Color.tooltip.border, 1)
          radius: Style.cornerRadius

          Text {
            id: label
            textFormat: Text.PlainText
            anchors.centerIn: parent
            text: dock.tooltipText
            color: Color.tooltip.text
            font.family: Style.font.family
            font.pixelSize: Style.font.body
          }
        }
      }
    }
  }

  // One row of the dock: an app button, a separator, or the config error.
  component DockEntry: Item {
    id: entryRoot

    // Filled in by the Repeater.
    required property var modelData
    property var bar: null
    readonly property var entry: modelData

    readonly property bool isApp: entry.kind === "app"
    readonly property var open: isApp ? root.windowsFor(entry, root.toplevels) : []
    readonly property bool focused: {
      for (var i = 0; i < open.length; i++) if (open[i].activated) return true
      return false
    }

    implicitWidth: root.barSize
    implicitHeight: entry.kind === "separator" ? Style.space(9) : button.implicitHeight

    Rectangle {
      visible: entryRoot.entry.kind === "separator"
      anchors.centerIn: parent
      width: Math.round(root.barSize * 0.45)
      height: 1
      color: Color.bar.text
      opacity: 0.25
    }

    BarIconButton {
      id: button
      visible: entryRoot.isApp || entryRoot.entry.kind === "error"
      anchors.horizontalCenter: parent.horizontalCenter
      bar: entryRoot.bar
      text: entryRoot.entry.kind === "error" ? "⚠" : (entryRoot.entry.image ? "" : (entryRoot.entry.icon || "?"))
      active: entryRoot.entry.kind === "error"
      tooltipText: entryRoot.entry.kind === "error"
        ? "omadock: " + root.configError + "\nRight-click to edit " + root.configPath
        : (entryRoot.entry.name || entryRoot.entry.run || "")
      iconComponent: entryRoot.entry.image ? imageIcon : null

      onPressed: function(b) {
        if (b === Qt.RightButton || entryRoot.entry.kind === "error") root.editConfig()
        else if (b === Qt.MiddleButton) root.launch(entryRoot.entry)
        else if (entryRoot.entry.click === "new") root.launch(entryRoot.entry)
        else root.activate(entryRoot.entry)
      }

      Component {
        id: imageIcon
        Image {
          readonly property string image: String(entryRoot.entry.image || "")
          source: image.indexOf("/") >= 0 ? "file://" + image.replace(/^~/, Quickshell.env("HOME")) : Quickshell.iconPath(image, true)
          sourceSize.width: Style.bar.iconCanvas * 2
          sourceSize.height: Style.bar.iconCanvas * 2
          fillMode: Image.PreserveAspectFit
          smooth: true
        }
      }
    }

    // Open-app marker on the screen-edge side, like a dock's running dot;
    // taller while one of its windows has focus.
    Rectangle {
      visible: entryRoot.open.length > 0
      width: 2
      radius: 1
      height: entryRoot.focused ? Math.round(button.height * 0.6) : Math.round(button.height * 0.25)
      anchors.verticalCenter: button.verticalCenter
      x: root.position === "left" ? 1 : entryRoot.width - width - 1
      color: entryRoot.focused ? Color.bar.active : Color.bar.text
      opacity: entryRoot.focused ? 1 : 0.6

      Behavior on height { NumberAnimation { duration: 140; easing.type: Easing.OutCubic } }
    }
  }
}
