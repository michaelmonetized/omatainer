import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "omatainer"

  property bool popupOpen: false

  readonly property var svc: bar && bar.shell ? bar.shell.serviceFor("omatainer") : null
  readonly property bool playing: svc ? svc.playing : false
  readonly property bool recording: svc ? svc.recording : false
  readonly property bool running: svc ? svc.running : false
  readonly property real bpm: svc ? svc.bpm : 124
  readonly property string deckA: svc ? svc.deckA : ""
  readonly property string deckB: svc ? svc.deckB : ""
  readonly property string error: svc ? svc.error : ""

  readonly property string liveLabel: {
    if (root.vertical)
      return playing ? "󰐊" : "󰝚"
    if (!running)
      return "󰝚  omatainer"
    var bits = []
    if (recording)
      bits.push("REC")
    bits.push(playing ? "PLAY" : "STOP")
    bits.push(Math.round(bpm) + "bpm")
    return bits.join(" · ")
  }

  readonly property color liveColor: {
    if (recording)
      return Color.urgent
    if (playing)
      return root.bar ? root.bar.barForeground : Color.foreground
    return Color.muted
  }

  readonly property bool opened: popupOpen

  function open() { popupOpen = true }
  function close() { popupOpen = false }
  function togglePanel() { popupOpen = !popupOpen }
  function closeForPopoutSwitch() { popupOpen = false }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.liveLabel
    foreground: root.liveColor
    useActiveColor: false
    tooltipText: root.error ? root.error : root.running
      ? ("omatainer  " + Math.round(root.bpm) + " bpm" + (root.deckA ? "\nA  " + root.deckA : "") + (root.deckB ? "\nB  " + root.deckB : ""))
      : (root.error || "omatainer idle — click to launch")
    horizontalMargin: 8.75
    verticalPadding: 8.75

    onPressed: function(b) {
      if (b === Qt.RightButton) {
        if (root.svc)
          root.svc.togglePlay()
        else
          root.launchApp()
      } else if (b === Qt.MiddleButton) {
        if (root.svc)
          root.svc.tap()
      } else {
        root.togglePanel()
      }
    }
  }

  function launchApp() {
    if (root.svc && root.svc.launch)
      root.svc.launch()
    else if (root.bar)
      root.bar.run("omarchy-launch-or-focus org.omarchy.omatainer 'uwsm-app -- omatainer'")
  }

  PopupCard {
    id: popup
    anchorItem: button
    bar: root.bar
    owner: root
    open: root.popupOpen
    contentWidth: popup.fittedContentWidth(Style.space(280))
    contentHeight: popup.fittedContentHeight(column.implicitHeight)

    Column {
      id: column
      width: parent.width
      spacing: Style.space(8)

      Text {
        width: parent.width
        text: "omatainer"
        color: Color.popups.text
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
        font.bold: true
      }

      Text {
        width: parent.width
        text: root.running
          ? (Math.round(root.bpm) + " bpm  ·  " + (root.playing ? "playing" : "stopped"))
          : "engine idle"
        color: Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }

      Text {
        width: parent.width
        visible: root.deckA !== "" || root.deckB !== ""
        wrapMode: Text.WordWrap
        text: "A  " + (root.deckA || "empty") + "\nB  " + (root.deckB || "empty")
        color: Color.popups.text
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }

      Text {
        width: parent.width
        visible: root.error !== ""
        text: root.error
        wrapMode: Text.WordWrap
        color: Color.urgent
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }

      Row {
        spacing: Style.space(6)

        Button {
          text: root.playing ? "Stop" : "Play"
          active: root.playing
          foreground: Color.popups.text
          horizontalPadding: Style.spacing.controlPaddingX
          verticalPadding: Style.spacing.controlPaddingY
          onClicked: {
            if (root.svc)
              root.svc.togglePlay()
            else
              root.launchApp()
          }
        }

        Button {
          text: "Open"
          foreground: Color.popups.text
          horizontalPadding: Style.spacing.controlPaddingX
          verticalPadding: Style.spacing.controlPaddingY
          onClicked: root.launchApp()
        }

        Button {
          text: "Tap"
          foreground: Color.popups.text
          horizontalPadding: Style.spacing.controlPaddingX
          verticalPadding: Style.spacing.controlPaddingY
          onClicked: if (root.svc) root.svc.tap()
        }
      }
    }
  }
}
