pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The macOS/Linux "Open With" picker. The model owns the three groups and the
// filter, so headers disappear with their last row; this file only draws the
// rows, keeps one application selected, and emits the command.
//
// Windows never opens this dialog: its native SHOpenWithDialog can also change
// associations, which is why `Workspace.openWithPickerSupported` guards the
// context-menu branch.
FocusScope {
    id: root

    property bool dark: true
    property color accent: Theme.interactive
    property string path: ""
    property string selectedId: ""
    property bool setDefault: false
    property bool opening: false
    property string errorText: ""

    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color fieldColor: Theme.field
    readonly property color hairline: Theme.hairline
    readonly property color dangerColor: Theme.danger
    readonly property string fileName: root.path.split(/[\\/]/).pop()

    signal canceled()

    visible: false
    z: 1004
    focus: visible

    function open(targetPath) {
        root.path = targetPath
        root.selectedId = ""
        root.setDefault = false
        root.opening = false
        root.errorText = ""
        filterInput.text = ""
        apps.load(targetPath)
        root.visible = true
        Qt.callLater(function () {
            if (!root.visible)
                return
            if (filterField.visible)
                filterInput.forceActiveFocus()
            else
                appList.forceActiveFocus()
        })
    }

    function close() {
        root.visible = false
    }

    function dismiss() {
        if (root.opening)
            return
        root.close()
        root.canceled()
    }

    function showError(message) {
        root.opening = false
        root.errorText = message || qsTr("无法打开这个项目")
    }

    function choose(appId) {
        if (appId === "" || root.opening)
            return
        root.selectedId = appId
        root.errorText = ""
    }

    function ensureSelection() {
        for (let index = 0; index < appList.count; index++) {
            if (apps.appIdAt(index) === root.selectedId)
                return
        }
        root.selectedId = apps.suggestedAppId
        for (let index = 0; index < appList.count; index++) {
            if (apps.appIdAt(index) === root.selectedId)
                return
        }
        root.selectedId = ""
        for (let index = 0; index < appList.count; index++) {
            const appId = apps.appIdAt(index)
            if (appId !== "") {
                root.selectedId = appId
                return
            }
        }
    }

    function moveSelection(delta) {
        if (appList.count === 0)
            return
        let index = appList.currentIndex
        for (let step = 0; step < appList.count; step++) {
            index += delta
            if (index < 0)
                index = appList.count - 1
            else if (index >= appList.count)
                index = 0
            if (apps.appIdAt(index) !== "") {
                appList.currentIndex = index
                root.choose(apps.appIdAt(index))
                return
            }
        }
    }

    function confirm() {
        if (root.opening || root.selectedId === "")
            return
        root.opening = true
        root.errorText = ""
        Workspace.openWithApp(root.path, root.selectedId, root.setDefault)
    }

    Keys.onEscapePressed: root.dismiss()

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: root.dismiss()
        }
    }

    Rectangle {
        anchors.centerIn: parent
        width: Math.min(440, root.width - 40)
        height: Math.min(464, root.height - 48)
        radius: 10
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Column {
            id: contentColumn
            anchors.fill: parent
            anchors.margins: 18
            spacing: 0

            Text {
                width: parent.width
                text: qsTr("选择打开方式")
                color: root.textColor
                font.pixelSize: 14
                font.weight: Font.DemiBold
            }

            Item { width: 1; height: 5 }

            Text {
                width: parent.width
                text: qsTr("用哪个应用打开“%1”？").arg(root.fileName)
                color: root.mutedColor
                font.pixelSize: 12
                elide: Text.ElideMiddle
                wrapMode: Text.NoWrap
            }

            Item { width: 1; height: 10 }

            Rectangle {
                id: filterField
                width: parent.width
                height: visible ? 30 : 0
                radius: 6
                visible: apps.needsFilter
                color: root.fieldColor
                border.width: 1
                border.color: filterInput.activeFocus
                    ? Theme.focusRing
                    : root.hairline

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 9
                    anchors.verticalCenter: parent.verticalCenter
                    visible: filterInput.text.length === 0
                    text: qsTr("筛选应用…")
                    color: root.mutedColor
                    font.pixelSize: 12
                }

                TextInput {
                    id: filterInput
                    anchors.fill: parent
                    anchors.leftMargin: 9
                    anchors.rightMargin: 9
                    verticalAlignment: TextInput.AlignVCenter
                    color: root.textColor
                    selectionColor: root.accent
                    selectedTextColor: Theme.onInteractive
                    font.pixelSize: 12
                    selectByMouse: true
                    clip: true

                    onTextChanged: {
                        apps.setFilter(text)
                        Qt.callLater(root.ensureSelection)
                    }
                    onAccepted: {
                        if (root.selectedId === "" && appList.count > 0)
                            root.moveSelection(1)
                        else
                            root.confirm()
                    }
                }
            }

            Item {
                width: 1
                height: filterField.visible ? 8 : 0
            }

            Item {
                id: listArea
                width: parent.width
                // The footer stays at the bottom of the panel; the list takes
                // whatever the title, filter and error line leave over.
                height: Math.max(80, parent.height - 90
                    - (filterField.visible ? 38 : 0)
                    - (root.errorText !== "" ? errorText.height + 6 : 0))

                ListView {
                    id: appList
                    anchors.fill: parent
                    visible: apps.loaded && apps.error === ""
                    model: apps
                    clip: true
                    spacing: 1
                    boundsBehavior: Flickable.StopAtBounds
                    focus: true

                    Keys.onUpPressed: root.moveSelection(-1)
                    Keys.onDownPressed: root.moveSelection(1)
                    Keys.onReturnPressed: root.confirm()
                    Keys.onEnterPressed: root.confirm()

                    onCurrentIndexChanged: {
                        if (currentIndex >= 0)
                            positionViewAtIndex(currentIndex, ListView.Contain)
                    }

                    delegate: Item {
                        required property string kind
                        required property string label
                        required property string appId
                        required property string iconName
                        required property bool isDefault

                        width: appList.width
                        height: kind === "header" ? 24 : 32
                        opacity: kind === "header" || appId !== "" ? 1 : 0

                        Text {
                            visible: kind === "header"
                            anchors.left: parent.left
                            anchors.leftMargin: 4
                            anchors.verticalCenter: parent.verticalCenter
                            text: kind === "header" ? label : ""
                            color: root.mutedColor
                            font.pixelSize: 11
                            font.weight: Font.DemiBold
                        }

                        Rectangle {
                            id: rowBackground
                            visible: kind !== "header"
                            anchors.fill: parent
                            anchors.leftMargin: 2
                            anchors.rightMargin: 2
                            radius: 5
                            color: root.selectedId === appId
                                ? Theme.selection
                                : (rowMouse.containsMouse
                                    ? Theme.hover
                                    : "transparent")
                        }

                        Image {
                            id: appIcon
                            visible: kind !== "header" && iconName !== ""
                                && status === Image.Ready
                            x: 12
                            width: 18
                            height: 18
                            anchors.verticalCenter: parent.verticalCenter
                            source: iconName === "" ? ""
                                : "image://fileicon/name/" + encodeURIComponent(iconName) + "?size=36"
                            fillMode: Image.PreserveAspectFit
                            asynchronous: true
                            cache: true
                        }

                        Rectangle {
                            visible: kind !== "header" && !appIcon.visible
                            x: 12
                            width: 18
                            height: 18
                            radius: 4
                            anchors.verticalCenter: parent.verticalCenter
                            color: Theme.hairlineStrong

                            Text {
                                anchors.centerIn: parent
                                text: label.length > 0 ? label.charAt(0).toUpperCase() : "?"
                                color: root.mutedColor
                                font.pixelSize: 10
                            }
                        }

                        Text {
                            visible: kind !== "header"
                            anchors.left: parent.left
                            anchors.leftMargin: 40
                            anchors.right: defaultBadge.left
                            anchors.rightMargin: 8
                            anchors.verticalCenter: parent.verticalCenter
                            text: kind === "header" ? "" : label
                            color: root.textColor
                            font.pixelSize: 13
                            elide: Text.ElideRight
                        }

                        Text {
                            id: defaultBadge
                            visible: kind !== "header" && isDefault
                            anchors.right: parent.right
                            anchors.rightMargin: 10
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("默认")
                            color: root.mutedColor
                            font.pixelSize: 10
                        }

                        MouseArea {
                            id: rowMouse
                            visible: kind !== "header"
                            anchors.fill: parent
                            hoverEnabled: true
                            onClicked: {
                                appList.currentIndex = index
                                root.choose(appId)
                            }
                            onDoubleClicked: {
                                root.choose(appId)
                                root.confirm()
                            }
                        }
                    }
                }

                Column {
                    anchors.centerIn: parent
                    width: parent.width
                    visible: !apps.loaded
                    spacing: 8

                    Text {
                        width: parent.width
                        text: qsTr("正在查找可用的应用…")
                        color: root.mutedColor
                        font.pixelSize: 12
                        horizontalAlignment: Text.AlignHCenter
                    }
                }

                Column {
                    anchors.centerIn: parent
                    width: parent.width
                    spacing: 10
                    visible: apps.loaded && apps.error !== ""

                    Text {
                        width: parent.width
                        text: apps.error
                        color: root.dangerColor
                        font.pixelSize: 12
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                    }

                    Text {
                        width: parent.width
                        text: qsTr("重试")
                        color: root.accent
                        font.pixelSize: 12
                        font.underline: true
                        horizontalAlignment: Text.AlignHCenter

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: apps.retry()
                        }
                    }
                }

                Text {
                    anchors.centerIn: parent
                    width: parent.width
                    visible: apps.loaded && apps.error === "" && appList.count === 0
                    text: qsTr("没有匹配的应用")
                    color: root.mutedColor
                    font.pixelSize: 12
                    horizontalAlignment: Text.AlignHCenter
                }
            }

            Item {
                width: 1
                height: 10
            }

            Text {
                id: errorText
                width: parent.width
                visible: root.errorText !== ""
                text: root.errorText
                color: root.dangerColor
                font.pixelSize: 11
                wrapMode: Text.Wrap
            }

            Item {
                width: 1
                height: root.errorText !== "" ? 6 : 0
            }

            Item {
                width: parent.width
                height: 30

                Item {
                    id: defaultBox
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    width: defaultRow.width
                    height: 20

                    Row {
                        id: defaultRow
                        spacing: 6

                        Rectangle {
                            width: 14
                            height: 14
                            radius: 3
                            anchors.verticalCenter: parent.verticalCenter
                            color: root.setDefault ? root.accent : "transparent"
                            border.width: 1
                            border.color: root.setDefault ? root.accent
                                : (root.dark ? "#55ffffff" : Theme.scrim)

                            Text {
                                anchors.centerIn: parent
                                visible: root.setDefault
                                text: "✓"
                                color: Theme.onInteractive
                                font.pixelSize: 10
                                font.weight: Font.Bold
                            }
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("设为默认打开方式")
                            color: root.mutedColor
                            font.pixelSize: 11
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.setDefault = !root.setDefault
                    }
                }

                Row {
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8

                    Rectangle {
                        width: 66
                        height: 26
                        radius: 6
                        color: cancelMouse.containsMouse
                            ? Theme.fill
                            : "transparent"
                        border.width: 1
                        border.color: root.hairline

                        Text {
                            anchors.centerIn: parent
                            text: qsTr("取消")
                            color: root.mutedColor
                            font.pixelSize: 12
                        }

                        MouseArea {
                            id: cancelMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            onClicked: root.dismiss()
                        }
                    }

                    Rectangle {
                        width: 76
                        height: 26
                        radius: 6
                        opacity: root.selectedId === "" || root.opening ? 0.45 : 1
                        color: root.accent

                        Text {
                            anchors.centerIn: parent
                            text: root.opening ? qsTr("正在打开…") : qsTr("打开")
                            color: Theme.onInteractive
                            font.pixelSize: 12
                            font.weight: Font.DemiBold
                        }

                        MouseArea {
                            anchors.fill: parent
                            enabled: root.selectedId !== "" && !root.opening
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.confirm()
                        }
                    }
                }
            }
        }
    }

    OpenWithModel {
        id: apps

        onLoadedChanged: {
            if (!apps.loaded || apps.error !== "")
                return
            root.selectedId = apps.suggestedAppId
            root.moveSelectionTo(root.selectedId)
        }
    }

    function moveSelectionTo(appId) {
        if (appId === "")
            return
        for (let index = 0; index < appList.count; index++) {
            if (apps.appIdAt(index) === appId) {
                appList.currentIndex = index
                return
            }
        }
    }
}
