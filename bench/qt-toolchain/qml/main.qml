import QtQuick
import QtQuick.Controls
import QtQuick.Window
import dae.smoke 1.0

ApplicationWindow {
    id: root
    width: 480
    height: 240
    visible: true
    title: qsTr("dae / cxx-qt smoke")
    color: palette.window

    readonly property SmokeObject model: SmokeObject { number: 1 }

    Column {
        anchors.fill: parent
        anchors.margins: 12
        spacing: 8
        Label { text: "Qt " + Qt.application.version; color: palette.text }
        Label { text: "Rust property: " + root.model.number; color: palette.text }
        Button { text: "increment"; onClicked: root.model.incrementNumber() }
    }

    Component.onCompleted: {
        console.log("qml: window up, Qt " + Qt.application.version)
        root.model.incrementNumber()
    }
}
