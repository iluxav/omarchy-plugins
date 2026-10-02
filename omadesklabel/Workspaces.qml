import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import Quickshell.Hyprland
import qs.Commons
import qs.Ui

// The Omarchy workspace numbers, with an optional label on each.
//   left click    go to that workspace
//   right click   name it (an empty name puts the plain number back)
//
// After each number come the icons of the apps open on it, one per app.
// Clicking an icon focuses that app's window there.
//
// A labelled workspace reads "omanote" after its number in a circle. The labels live in
// ~/.config/omadesklabel/labels.json, keyed by workspace number, so they
// survive logouts and every monitor's bar shows the same ones.
BarWidget {
  id: root
  moduleName: "iluxav.omadesklabel"

  readonly property string configDir: Quickshell.env("HOME") + "/.config/omadesklabel"
  readonly property string labelsPath: root.configDir + "/labels.json"
  // Longer labels are cut short with an ellipsis. Set "maxLength" in shell.json.
  readonly property int maxLength: Number(setting("maxLength", 24)) || 24
  // At most this many app icons per workspace; more show as "+N". 0 hides them.
  readonly property int maxIcons: Math.max(0, Number(setting("maxIcons", 5)))
  readonly property int iconSize: Math.round(Style.bar.iconCanvas * 0.7)

  property var labels: ({})

  function workspaceById(id) {
    var values = Hyprland.workspaces.values
    for (var i = 0; i < values.length; i++) {
      if (values[i].id === id) return values[i]
    }

    return null
  }

  function workspaceIds() {
    var ids = [1, 2, 3, 4, 5]
    var values = Hyprland.workspaces.values

    for (var i = 0; i < values.length; i++) {
      var id = values[i].id
      if (id > 0 && id <= 10 && ids.indexOf(id) === -1) ids.push(id)
    }

    ids.sort(function(left, right) { return left - right })
    return ids
  }

  function focusWorkspace(id) {
    if (!root.bar) return
    root.bar.run("hyprctl dispatch " + Util.shellQuote("hl.dsp.focus({ workspace = \"" + id + "\" })"))
  }

  function labelOf(id) {
    var label = root.labels[String(id)]
    return typeof label === "string" ? label.trim() : ""
  }

  function numberOf(id) {
    return id === 10 ? "0" : String(id)
  }

  function shortened(label) {
    return label.length > root.maxLength ? label.slice(0, root.maxLength - 1) + "…" : label
  }

  // The apps open on a workspace, one entry per app id, in window order.
  // Touching toplevels.values and each window's `wayland` keeps it live.
  function appsOn(workspace) {
    if (!workspace) return []
    var apps = [], seen = ({})
    var windows = workspace.toplevels.values
    for (var i = 0; i < windows.length; i++) {
      var window = windows[i]
      var ipc = window.lastIpcObject
      var appId = String((window.wayland && window.wayland.appId) || (ipc && ipc["class"]) || "")
      if (appId === "" || seen[appId]) continue
      seen[appId] = true
      apps.push({ appId: appId, window: window })
    }
    return apps
  }

  function iconFor(appId) {
    var entry = DesktopEntries.heuristicLookup(appId)
    var icon = entry && entry.icon ? String(entry.icon) : appId
    if (icon.charAt(0) === "/") return "file://" + icon
    return Quickshell.iconPath(icon, "application-x-executable")
  }

  function nameFor(app) {
    var entry = DesktopEntries.heuristicLookup(app.appId)
    return entry && entry.name ? entry.name : app.appId
  }

  function focusApp(app, workspaceId) {
    if (app.window.wayland) app.window.wayland.activate()
    else root.focusWorkspace(workspaceId)
  }

  function rename(id) {
    var payload = JSON.stringify({ workspace: id, number: root.numberOf(id), label: root.labelOf(id), path: root.labelsPath })
    Quickshell.execDetached(["omarchy-shell", "shell", "summon", "iluxav.omadesklabel", payload])
  }

  function parse(content) {
    try {
      var parsed = JSON.parse(String(content || "{}"))
      root.labels = parsed && typeof parsed === "object" ? parsed : ({})
    } catch (e) {
      console.warn("omadesklabel", "Ignoring bad labels file", root.labelsPath, e)
      root.labels = ({})
    }
  }

  // The file has to be there before it can be watched.
  Process {
    running: true
    command: ["bash", "-c", "mkdir -p \"$1\" && { [ -e \"$2\" ] || printf '{}\\n' > \"$2\"; }", "omadesklabel", root.configDir, root.labelsPath]
    onExited: labelsFile.path = root.labelsPath
  }

  FileView {
    id: labelsFile
    watchChanges: true
    printErrors: false
    onFileChanged: reload()
    onLoaded: root.parse(text())
    onLoadFailed: root.labels = ({})
  }

  readonly property real trailingGap: root.vertical ? 0 : Style.spaceReal(1.5)

  implicitWidth: grid.implicitWidth + trailingGap
  implicitHeight: grid.implicitHeight

  GridLayout {
    id: grid
    anchors.fill: parent
    anchors.rightMargin: root.trailingGap
    columns: root.vertical ? 1 : root.workspaceIds().length
    columnSpacing: root.vertical ? 0 : Style.space(15)
    rowSpacing: root.vertical ? Style.space(2) : 0

    Repeater {
      model: root.workspaceIds()

      Row {
        id: cell

        required property int modelData

        readonly property var workspace: root.workspaceById(modelData)
        readonly property var apps: root.vertical || root.maxIcons === 0 ? [] : root.appsOn(workspace)
        readonly property var shownApps: apps.slice(0, root.maxIcons)

        WidgetButton {
          id: button

          readonly property bool occupied: cell.workspace !== null && cell.workspace.toplevels.values.length > 0
          readonly property bool focused: Hyprland.focusedWorkspace !== null && Hyprland.focusedWorkspace.id === cell.modelData
          readonly property string label: root.labelOf(cell.modelData)
          // A vertical bar has no room for words, so it keeps the numbers.
          readonly property bool labelled: label !== "" && !root.vertical
          readonly property color tint: focused ? Color.bar.active : foreground

          bar: root.bar
          // Unlabelled, it is the stock widget: the number, or a dot when focused.
          // Labelled, it draws its own number-in-a-circle and name (below), and
          // the focused one takes the bar's active colour, so the name stays readable.
          text: labelled
            ? root.numberOf(cell.modelData) + " " + root.shortened(label)
            : (focused ? "󱓻" : root.numberOf(cell.modelData))
          labelVisible: !labelled
          opacity: occupied || focused ? 1 : 0.5
          horizontalMargin: 6
          verticalPadding: 6
          fixedWidth: root.vertical
            ? root.barSize
            : (labelled ? Math.ceil(badge.implicitWidth + scaledHorizontalMargin * 2) : Style.space(20))
          fixedHeight: root.barSize
          tooltipText: labelled && label.length > root.maxLength ? label : ""
          onPressed: function(mouseButton) {
            if (mouseButton === Qt.RightButton) root.rename(cell.modelData)
            else root.focusWorkspace(cell.modelData)
          }

          Row {
            id: badge
            visible: button.labelled
            anchors.centerIn: parent
            spacing: Style.space(5)

            // The number in a ring; the focused workspace fills it.
            Rectangle {
              readonly property int diameter: Math.round(button.fontSize * 1.2)

              anchors.verticalCenter: parent.verticalCenter
              width: diameter
              height: diameter
              radius: diameter / 2
              color: button.focused ? button.tint : "transparent"
              border.width: Math.max(1, Style.space(1))
              border.color: button.tint

              Text {
                anchors.centerIn: parent
                textFormat: Text.PlainText
                text: root.numberOf(cell.modelData)
                color: button.focused ? (root.bar ? root.bar.background : Color.background) : button.tint
                font.family: button.fontFamily
                font.pixelSize: Math.round(button.fontSize * 0.65)
                font.bold: true
                renderType: Text.NativeRendering
              }
            }

            Text {
              anchors.verticalCenter: parent.verticalCenter
              textFormat: Text.PlainText
              text: root.shortened(button.label)
              color: button.tint
              font.family: button.fontFamily
              font.pixelSize: button.fontSize
              renderType: Text.NativeRendering
            }
          }
        }

        Row {
          visible: cell.apps.length > 0
          height: root.barSize
          spacing: Style.space(4)
          rightPadding: Style.space(2)

          Repeater {
            model: cell.shownApps

            Item {
              id: appIcon

              required property var modelData

              width: icon.width
              height: root.barSize

              // Fixed height; the width follows the icon's own shape, so a
              // wide or tall icon is not squeezed into a square.
              Image {
                id: icon
                anchors.centerIn: parent
                height: root.iconSize
                width: implicitHeight > 0 ? Math.round(implicitWidth * root.iconSize / implicitHeight) : root.iconSize
                source: root.iconFor(appIcon.modelData.appId)
                // Load at full resolution; the loader keeps the image's proportions.
                sourceSize.width: Math.round(root.iconSize * Screen.devicePixelRatio)
                sourceSize.height: Math.round(root.iconSize * Screen.devicePixelRatio)
                fillMode: Image.PreserveAspectFit
                smooth: true
              }

              MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton | Qt.RightButton
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onEntered: if (root.bar) root.bar.showTooltip(appIcon, root.nameFor(appIcon.modelData))
                onExited: if (root.bar) root.bar.hideTooltip(appIcon)
                onClicked: function(mouse) {
                  if (root.bar) root.bar.hideTooltip(appIcon)
                  if (mouse.button === Qt.RightButton) root.rename(cell.modelData)
                  else root.focusApp(appIcon.modelData, cell.modelData)
                }
              }
            }
          }

          Text {
            visible: cell.apps.length > cell.shownApps.length
            height: root.barSize
            verticalAlignment: Text.AlignVCenter
            text: "+" + (cell.apps.length - cell.shownApps.length)
            color: root.bar ? root.bar.barForeground : Color.foreground
            opacity: 0.7
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
          }
        }
      }
    }
  }
}
