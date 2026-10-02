import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import QtQuick
import qs.Commons
import qs.Ui

// The box a right click on a workspace opens: type a label, Enter saves it,
// Esc leaves it as it was. Saving an empty label puts the plain number back.
//
//   omarchy-shell shell summon iluxav.omadesklabel '{"workspace":2,"number":"2","label":"","path":"..."}'
//
// The bar widget sends the workspace, its current label and where the labels
// file is; this writes the file and the bar follows it.
Item {
  id: root

  property var shell: null
  property var manifest: null

  property bool opened: false
  property int workspace: 0
  property string number: ""
  property string path: ""
  property string text: ""
  // The label as it was: the first key typed replaces it rather than adding to it.
  property bool replacing: false

  property color background: Color.menu.background
  property color foreground: Color.menu.text
  property color border: Color.menu.border
  property var borderSpec: Border.surfaceSpec("menu", "border", border, Math.max(1, Style.space(2)))
  property color scrim: Color.menu.scrim
  property color selectedBackground: Color.menu.selectedBackground
  property color selectedText: Color.menu.selectedText
  property int contentMargin: Style.spacing.panelPadding
  property int headerHeight: Math.max(Style.space(34), Style.font.title + Style.spacing.controlPaddingY * 2)
  property int cardWidth: Math.min(Style.space(440), panel.width - Style.gapsOut * 2)

  function open(payloadJson) {
    var payload = ({})
    try { payload = JSON.parse(payloadJson || "{}") } catch (e) { payload = ({}) }
    if (!payload.workspace || !payload.path) return
    root.workspace = payload.workspace
    root.number = payload.number || String(payload.workspace)
    root.path = payload.path
    root.text = payload.label || ""
    root.replacing = root.text !== ""
    root.opened = true
    Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  }

  function close() {
    root.opened = false
  }

  function dismiss() {
    root.opened = false
    if (root.shell && typeof root.shell.hide === "function")
      root.shell.hide((root.manifest && root.manifest.id) || "iluxav.omadesklabel")
  }

  function type(next) {
    root.text = next
    root.replacing = false
  }

  // Read the file fresh, change one key, write it back: another label may
  // have been saved since the bar last looked.
  function save() {
    var key = String(root.workspace)
    var label = root.text.trim()
    root.dismiss()
    saveProc.command = ["python3", "-c",
      "import json, sys\n" +
      "path, key, label = sys.argv[1:4]\n" +
      "try:\n" +
      "    labels = json.load(open(path))\n" +
      "    if not isinstance(labels, dict): labels = {}\n" +
      "except Exception:\n" +
      "    labels = {}\n" +
      "if label: labels[key] = label\n" +
      "else: labels.pop(key, None)\n" +
      "data = json.dumps(labels, indent=2, ensure_ascii=False, sort_keys=True) + '\\n'\n" +
      "open(path, 'w').write(data)\n",
      root.path, key, label]
    saveProc.running = true
  }

  Process {
    id: saveProc
    onExited: function(exitCode) {
      if (exitCode !== 0) console.warn("omadesklabel", "Could not save the label to", root.path)
    }
  }

  PanelWindow {
    id: panel
    visible: root.opened
    anchors { top: true; bottom: true; left: true; right: true }
    color: "transparent"
    WlrLayershell.namespace: "omarchy-omadesklabel"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive
    exclusionMode: ExclusionMode.Ignore

    Rectangle {
      anchors.fill: parent
      color: root.scrim
    }

    MouseArea {
      anchors.fill: parent
      onClicked: root.dismiss()
    }

    BorderSurface {
      id: card
      width: root.cardWidth
      height: root.contentMargin * 2 + root.headerHeight
      radius: Style.cornerRadius
      anchors.horizontalCenter: parent.horizontalCenter
      y: Math.max(Style.gapsOut, Math.round(parent.height * 0.26))
      color: root.background
      borderSpec: root.borderSpec
      padding: root.contentMargin

      MouseArea { anchors.fill: parent; onClicked: {} }

      Item {
        id: keyCatcher
        anchors.fill: parent
        focus: true

        Keys.priority: Keys.BeforeItem
        Keys.onPressed: function(event) {
          if (event.key === Qt.Key_Escape) {
            root.dismiss()
            event.accepted = true
          } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            root.save()
            event.accepted = true
          } else if (root.replacing && (event.key === Qt.Key_Backspace || event.key === Qt.Key_Delete)) {
            root.type("")
            event.accepted = true
          } else if (Util.editsFilter(event, root.text)) {
            root.type(Util.editedFilter(event, root.text))
            event.accepted = true
          } else if (event.text && event.text.length === 1 && event.text.charCodeAt(0) >= 32 && event.text.charCodeAt(0) !== 127) {
            root.type((root.replacing ? "" : root.text) + event.text)
            event.accepted = true
          } else if (root.replacing && (event.key === Qt.Key_Right || event.key === Qt.Key_End)) {
            root.replacing = false
            event.accepted = true
          }
        }
      }

      Item {
        anchors.fill: parent
        anchors.topMargin: card.contentTopInset
        anchors.rightMargin: card.contentRightInset
        anchors.bottomMargin: card.contentBottomInset
        anchors.leftMargin: card.contentLeftInset

        // "3)" in front, as it will read on the bar.
        Text {
          id: prefix
          textFormat: Text.PlainText
          anchors.left: parent.left
          anchors.verticalCenter: parent.verticalCenter
          text: root.number + ")"
          color: root.foreground
          opacity: 0.58
          font.family: Style.font.menuFamily
          font.pixelSize: Style.font.heading
        }

        // The label being typed. The old one shows selected until a key replaces it.
        Rectangle {
          anchors.fill: input
          anchors.leftMargin: -Style.space(2)
          anchors.rightMargin: -Style.space(2)
          visible: root.replacing
          radius: Math.max(0, Style.cornerRadius - 2)
          color: root.selectedBackground
        }

        Text {
          id: input
          textFormat: Text.PlainText
          anchors.left: prefix.right
          anchors.leftMargin: Style.space(2)
          anchors.verticalCenter: parent.verticalCenter
          width: Math.min(implicitWidth, parent.width - prefix.width - Style.space(12))
          text: root.text || "Name this workspace..."
          color: root.replacing ? root.selectedText : root.foreground
          opacity: root.text ? 1 : 0.58
          font.family: Style.font.menuFamily
          font.pixelSize: Style.font.heading
          elide: Text.ElideLeft
        }

        Rectangle {
          id: caret
          visible: root.opened && !root.replacing
          anchors.left: root.text ? input.right : prefix.right
          anchors.leftMargin: root.text ? Style.space(1) : Style.space(2)
          anchors.verticalCenter: parent.verticalCenter
          width: Math.max(1, Style.space(2))
          height: Style.font.heading
          color: root.foreground

          SequentialAnimation on opacity {
            running: caret.visible
            loops: Animation.Infinite
            NumberAnimation { to: 0; duration: 530; easing.type: Easing.InOutQuad }
            NumberAnimation { to: 1; duration: 530; easing.type: Easing.InOutQuad }
          }
        }
      }
    }
  }
}
