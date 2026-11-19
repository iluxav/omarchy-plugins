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

      WidgetButton {
        id: button

        required property int modelData

        readonly property var workspace: root.workspaceById(modelData)
        readonly property bool occupied: workspace !== null && workspace.toplevels.values.length > 0
        readonly property bool focused: Hyprland.focusedWorkspace !== null && Hyprland.focusedWorkspace.id === button.modelData
        readonly property string label: root.labelOf(button.modelData)
        // A vertical bar has no room for words, so it keeps the numbers.
        readonly property bool labelled: label !== "" && !root.vertical
        readonly property color tint: focused ? Color.bar.active : foreground

        bar: root.bar
        // Unlabelled, it is the stock widget: the number, or a dot when focused.
        // Labelled, it draws its own number-in-a-circle and name (below), and
        // the focused one takes the bar's active colour, so the name stays readable.
        text: labelled
          ? root.numberOf(button.modelData) + " " + root.shortened(label)
          : (focused ? "󱓻" : root.numberOf(button.modelData))
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
          if (mouseButton === Qt.RightButton) root.rename(button.modelData)
          else root.focusWorkspace(button.modelData)
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

            // Centred on the digit's ink, not its line box: a line box has
            // room for descenders below and accents above, which puts a
            // digit high and to one side of the ring.
            TextMetrics {
              id: inkMetrics
              font: number.font
              text: number.text
            }

            Text {
              id: number
              readonly property rect ink: inkMetrics.tightBoundingRect
              x: Math.round((parent.width - ink.width) / 2 - ink.x)
              y: Math.round((parent.height - ink.height) / 2 - ink.y - baselineOffset)
              textFormat: Text.PlainText
              text: root.numberOf(button.modelData)
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
    }
  }
}
