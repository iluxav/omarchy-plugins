import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// Gmail in the bar: an envelope and the unread count.
//   left click    the unread emails; click one to read it (that marks it read)
//   right click   check now
//   middle click  open Gmail in the browser
//
// The Gmail side is the `omamail` script next to this file (IMAP with an app
// password kept in the keyring); the email itself opens in `viewer.py`, a
// WebKitGTK window, since the shell cannot render HTML email.
BarWidget {
  id: root
  moduleName: "iluxav.omamail"

  readonly property string pluginDir: Qt.resolvedUrl(".").toString().replace(/^file:\/\//, "").replace(/\/$/, "")
  readonly property string script: root.pluginDir + "/omamail"
  // Seconds between checks, and how many emails the dropdown lists.
  readonly property int interval: Math.max(30, Number(setting("interval", 120)) || 120)
  readonly property int limit: Math.max(1, Number(setting("limit", 15)) || 15)

  // "loading" until the first answer, then "ok", or the error kind:
  // "setup" (no account yet), "auth" (Gmail said no), "network".
  property string status: "loading"
  property string message: ""
  property string account: ""
  property string inbox: "https://mail.google.com/"
  property int count: 0
  property var messages: []
  property bool popupOpen: false

  function close() { popupOpen = false }

  function refresh() {
    if (!listProc.running) listProc.running = true
  }

  function setup() {
    root.popupOpen = false
    if (root.bar) root.bar.run("omarchy-launch-floating-terminal-with-presentation " + Util.shellQuote(root.script) + " setup")
    // Look again soon: setup may well have finished by then.
    setupWatch.restart()
  }

  function openGmail(link) {
    root.popupOpen = false
    Quickshell.execDetached(["xdg-open", link || root.inbox])
  }

  function read(uid) {
    root.popupOpen = false
    // Drop it from the list straight away; the check after opening confirms it.
    var left = []
    for (var i = 0; i < root.messages.length; i++)
      if (root.messages[i].uid !== uid) left.push(root.messages[i])
    if (left.length !== root.messages.length) {
      root.messages = left
      root.count = Math.max(0, root.count - 1)
    }
    openProc.command = [root.script, "open", uid]
    openProc.running = true
  }

  function take(text) {
    var answer = null
    try { answer = JSON.parse(String(text || "").trim().split("\n").pop()) } catch (e) { answer = null }
    if (!answer) {
      root.status = "network"
      root.message = "omamail gave no answer"
      return
    }
    if (answer.error) {
      root.status = answer.error
      root.message = answer.message || ""
      return
    }
    root.status = "ok"
    root.message = ""
    root.account = answer.email || ""
    root.inbox = answer.inbox || root.inbox
    root.count = answer.count || 0
    root.messages = answer.messages || []
  }

  Process {
    id: listProc
    command: [root.script, "list", "--limit", String(root.limit)]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.take(text)
    }
  }

  Process {
    id: openProc
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var answer = null
        try { answer = JSON.parse(String(text || "").trim().split("\n").pop()) } catch (e) { answer = null }
        if (!answer || answer.error)
          Quickshell.execDetached(["notify-send", "omamail", (answer && answer.message) || "Could not open that email"])
      }
    }
    onExited: root.refresh()
  }

  Timer {
    interval: root.interval * 1000
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  Timer {
    id: setupWatch
    interval: 4000
    repeat: true
    running: false
    property int tries: 0
    onRunningChanged: if (running) tries = 0
    onTriggered: {
      tries += 1
      if (root.status === "ok" || tries > 45) stop()
      else root.refresh()
    }
  }

  // Envelope glyphs: filled when there is something unread, outlined otherwise.
  readonly property string glyph: root.status === "ok" && root.count > 0 ? "󰇮" : "󰇰"
  readonly property bool problem: root.status !== "ok" && root.status !== "loading"

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.glyph + (root.status === "ok" && root.count > 0 && !root.vertical ? " " + (root.count > 999 ? "999+" : root.count) : "")
    dimmed: root.problem || (root.status === "ok" && root.count === 0)
    tooltipText: root.status === "ok"
      ? (root.count === 1 ? "1 unread email" : root.count + " unread emails") + "  ·  right-click: check now  ·  middle-click: Gmail"
      : (root.status === "setup" ? "Gmail is not set up — click to set it up" : (root.message || "Checking Gmail..."))
    onPressed: function(b) {
      if (b === Qt.RightButton) root.refresh()
      else if (b === Qt.MiddleButton) root.openGmail("")
      else if (root.status === "setup") root.setup()
      else {
        root.popupOpen = !root.popupOpen
        if (root.popupOpen) root.refresh()
      }
    }
  }

  PopupCard {
    id: popup
    anchorItem: root
    bar: root.bar
    owner: root
    open: root.popupOpen
    contentWidth: popup.fittedContentWidth(Style.space(400))
    contentHeight: popup.fittedContentHeight(column.implicitHeight, Style.space(640))

    readonly property color fg: Color.popups.text
    readonly property color hoverFill: Qt.rgba(fg.r, fg.g, fg.b, 0.08)

    Column {
      id: column
      anchors.fill: parent
      spacing: Style.space(8)

      // Inbox · 3 unread                                         Open Gmail
      Item {
        width: parent.width
        height: Math.max(heading.implicitHeight, gmailLink.implicitHeight)

        Text {
          id: heading
          textFormat: Text.PlainText
          anchors.left: parent.left
          anchors.verticalCenter: parent.verticalCenter
          text: root.status === "ok"
            ? (root.count > 0 ? "Inbox  ·  " + root.count + " unread" : "Inbox")
            : "Gmail"
          color: popup.fg
          font.family: Style.font.family
          font.pixelSize: Style.font.body
          font.bold: true
        }

        Text {
          id: gmailLink
          textFormat: Text.PlainText
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          text: "Open Gmail"
          color: popup.fg
          opacity: gmailArea.containsMouse ? 1 : 0.6
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall

          MouseArea {
            id: gmailArea
            anchors.fill: parent
            anchors.margins: -Style.space(4)
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: root.openGmail("")
          }
        }
      }

      Rectangle {
        width: parent.width
        height: Style.spacing.hairline
        color: popup.fg
        opacity: 0.12
      }

      // Nothing to list: say why.
      Text {
        visible: root.status !== "ok" || root.messages.length === 0
        width: parent.width
        textFormat: Text.PlainText
        wrapMode: Text.Wrap
        topPadding: Style.space(6)
        bottomPadding: Style.space(6)
        text: root.status === "ok" ? "No unread email."
          : root.status === "loading" ? "Checking Gmail..."
          : root.status === "auth" ? "Gmail refused the app password. Set it up again with a new one."
          : root.status === "setup" ? "Gmail is not set up yet."
          : (root.message || "Could not reach Gmail.")
        color: popup.fg
        opacity: 0.7
        font.family: Style.font.family
        font.pixelSize: Style.font.body
      }

      Button {
        visible: root.status === "setup" || root.status === "auth"
        text: root.status === "auth" ? "Set up again" : "Set up Gmail"
        onClicked: root.setup()
      }

      ListView {
        id: list
        visible: root.status === "ok" && root.messages.length > 0
        width: parent.width
        height: Math.min(contentHeight, Style.space(560))
        clip: true
        spacing: Style.space(2)
        boundsBehavior: Flickable.StopAtBounds
        model: root.messages

        delegate: Rectangle {
          id: row
          required property var modelData
          width: list.width
          height: lines.implicitHeight + Style.space(12)
          radius: Math.max(0, Style.cornerRadius - 2)
          color: rowArea.containsMouse ? popup.hoverFill : "transparent"

          MouseArea {
            id: rowArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            acceptedButtons: Qt.LeftButton | Qt.MiddleButton
            onClicked: function(mouse) {
              if (mouse.button === Qt.MiddleButton) root.openGmail(row.modelData.link)
              else root.read(row.modelData.uid)
            }
          }

          Column {
            id: lines
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            anchors.leftMargin: Style.space(8)
            anchors.rightMargin: Style.space(8)
            spacing: Style.space(1)

            // Sender, and when, on the right.
            Item {
              width: parent.width
              height: from.implicitHeight

              Text {
                id: from
                textFormat: Text.PlainText
                anchors.left: parent.left
                anchors.right: when.left
                anchors.rightMargin: Style.space(8)
                text: row.modelData.from
                elide: Text.ElideRight
                color: popup.fg
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                font.bold: true
              }

              Text {
                id: when
                textFormat: Text.PlainText
                anchors.right: parent.right
                anchors.verticalCenter: from.verticalCenter
                text: row.modelData.date
                color: popup.fg
                opacity: 0.5
                font.family: Style.font.family
                font.pixelSize: Style.font.bodySmall
              }
            }

            Text {
              width: parent.width
              textFormat: Text.PlainText
              text: row.modelData.subject
              elide: Text.ElideRight
              color: popup.fg
              font.family: Style.font.family
              font.pixelSize: Style.font.body
            }

            Text {
              visible: text !== ""
              width: parent.width
              textFormat: Text.PlainText
              text: row.modelData.snippet
              elide: Text.ElideRight
              maximumLineCount: 1
              color: popup.fg
              opacity: 0.55
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
            }
          }
        }
      }

      Text {
        visible: root.status === "ok" && root.count > root.messages.length && root.messages.length > 0
        width: parent.width
        textFormat: Text.PlainText
        horizontalAlignment: Text.AlignHCenter
        text: "and " + (root.count - root.messages.length) + " more in Gmail"
        color: popup.fg
        opacity: 0.5
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
    }
  }
}
