import QtQuick
import QtQuick.Window
import dae 1.0

// A window that shares the main window's model, the shape gate 4 measured.
// It gets its own HWND entry in the native map; the tab's data is not copied.
Window {
    id: root

    property var model: null
    property string token: ""
    property bool dark: true
    property string material: "none"
    property var ownerList: []

    objectName: token
    width: 900
    height: 620
    visible: false
    color: "transparent"
    title: "dae"
    flags: Qt.Window | Qt.FramelessWindowHint
        | Qt.WindowMinimizeButtonHint | Qt.WindowMaximizeButtonHint
        | Qt.WindowCloseButtonHint

    readonly property color chromeTint: dark
        ? Qt.rgba(0.055, 0.063, 0.075, 0.38)
        : Qt.rgba(0.98, 0.985, 0.99, 0.46)
    readonly property color solidChrome: dark ? "#14161a" : "#f4f6f8"
    readonly property color chromeColor: material === "none" ? solidChrome : chromeTint

    WindowController {
        id: chrome
        token: root.objectName
    }

    Component.onCompleted: {
        if (!chrome.attach())
            console.warn("dae: native window attach failed")
        chrome.applyMaterial(root.material, root.dark)
        visible = true
    }

    Component.onDestruction: {
        const index = root.ownerList ? root.ownerList.indexOf(root) : -1
        if (index >= 0)
            root.ownerList.splice(index, 1)
    }

    onClosing: Qt.callLater(root.destroy)

    Column {
        anchors.fill: parent

        TitleBar {
            width: parent.width
            window: root
            dark: root.dark
            material: root.material
            title: root.title
        }

        FileList {
            width: parent.width
            height: parent.height - 40
            model: root.model
            dark: root.dark
            headerColor: root.chromeColor
        }
    }
}
