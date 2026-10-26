pragma ComponentBehavior: Bound
import QtQuick

// The OAuth dialog for Google Drive, OneDrive and Dropbox.
//
// The app ships no built-in client credentials: the user creates an OAuth app
// in the provider's console and enters its id here. The loopback listener and
// token exchange live in `dae_core::fs::cloud::oauth`; this file only collects
// the credentials and reports the browser round trip's outcome.
Item {
    id: root

    property bool dark: true
    property color accent: "#2f6fdf"
    property string provider: "google_drive"
    property bool authorizing: false
    property bool copied: false
    property string errorText: ""

    readonly property string redirectUri: "http://127.0.0.1:51888"

    signal authorizeRequested(string provider, string clientId, string clientSecret)
    signal canceled()

    visible: false
    z: 1005
    focus: visible

    readonly property color textColor: root.dark ? "#e7ebf2" : "#20252b"
    readonly property color mutedColor: root.dark ? "#8d94a2" : "#657080"
    readonly property color fieldColor: root.dark ? "#16181c" : "#f4f6f9"
    readonly property color hairline: root.dark ? "#26ffffff" : "#18000000"
    readonly property color dangerColor: root.dark ? "#f08a8a" : "#b42323"

    function open() {
        root.provider = "google_drive"
        clientIdField.value = ""
        secretField.value = ""
        root.authorizing = false
        root.copied = false
        root.errorText = ""
        root.visible = true
        Qt.callLater(function () {
            if (root.visible)
                clientIdField.inputItem.forceActiveFocus()
        })
    }

    function close() {
        root.visible = false
    }

    function dismiss() {
        root.visible = false
        root.canceled()
    }

    function setProvider(value) {
        if (root.provider === value)
            return
        root.provider = value
        root.errorText = ""
    }

    function setAuthorizing(value) {
        root.authorizing = value
        if (value)
            root.errorText = ""
    }

    function applyResult(ok, message) {
        root.authorizing = false
        if (!ok)
            root.errorText = message
    }

    function applyError(message) {
        root.authorizing = false
        root.errorText = message
    }

    function guideText() {
        switch (root.provider) {
        case "onedrive":
            return "在 Azure 门户的「应用注册」中创建应用（支持任何组织目录和个人账户），并在「移动和桌面应用程序」平台下添加重定向 URI。"
        case "dropbox":
            return "在 Dropbox App Console 中创建应用，并在 OAuth 2 设置中逐字添加下方重定向 URI。"
        default:
            return "在 Google Cloud 控制台中启用 Google Drive API，并在「API 和服务 → 凭据」中创建「桌面应用」类型的 OAuth 客户端。"
        }
    }

    function secretLabel() {
        return root.provider === "onedrive" ? "Client Secret" : "Client Secret（可选）"
    }

    function submit() {
        if (root.authorizing)
            return
        const clientId = clientIdField.value.trim()
        if (clientId === "") {
            root.errorText = "请输入 Client ID"
            return
        }
        root.setAuthorizing(true)
        root.authorizeRequested(root.provider, clientId, secretField.value.trim())
    }

    function copyRedirect() {
        clipboardShim.text = root.redirectUri
        clipboardShim.selectAll()
        clipboardShim.copy()
        root.copied = true
        copyReset.restart()
    }

    Keys.onEscapePressed: root.dismiss()

    Timer {
        id: copyReset
        interval: 1500
        onTriggered: root.copied = false
    }

    TextInput {
        id: clipboardShim
        width: 1
        height: 1
        opacity: 0
    }

    Rectangle {
        anchors.fill: parent
        color: "#73000000"

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: root.dismiss()
        }
    }

    Rectangle {
        id: card
        anchors.centerIn: parent
        width: Math.min(490, root.width - 48)
        height: 470
        radius: 10
        color: root.dark ? "#202329" : "#ffffff"
        border.width: 1
        border.color: root.dark ? "#33ffffff" : "#18000000"

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Text {
            x: 20
            y: 18
            text: "连接云存储"
            color: root.textColor
            font.pixelSize: 14
            font.weight: Font.DemiBold
        }

        Text {
            x: 20
            y: 42
            width: parent.width - 40
            text: "使用你自己创建的 OAuth 应用凭据，在浏览器中完成登录授权。"
            color: root.mutedColor
            font.pixelSize: 11
            elide: Text.ElideRight
        }

        Text {
            x: 20
            y: 76
            text: "网盘"
            color: root.dark ? "#aeb5c0" : "#565e68"
            font.pixelSize: 12
        }

        Row {
            x: 20
            y: 96
            spacing: 8

            ProviderCard {
                provider: "google_drive"
                label: "Google Drive"
                glyph: "G"
                tint: "#4285f4"
            }

            ProviderCard {
                provider: "onedrive"
                label: "OneDrive"
                glyph: "O"
                tint: "#0078d4"
            }

            ProviderCard {
                provider: "dropbox"
                label: "Dropbox"
                glyph: "D"
                tint: "#0061ff"
            }
        }

        Field {
            id: clientIdField
            x: 20
            y: 180
            width: card.width - 40
            label: "Client ID"
            placeholder: "来自服务商控制台的 OAuth 客户端 ID"
            enabled: !root.authorizing
            onEdited: root.errorText = ""
        }

        Field {
            id: secretField
            x: 20
            y: 242
            width: card.width - 40
            label: root.secretLabel()
            password: true
            enabled: !root.authorizing
            onEdited: root.errorText = ""
        }

        Rectangle {
            x: 20
            y: 306
            width: card.width - 40
            height: 86
            radius: 6
            color: root.dark ? "#1a1d22" : "#f4f6f9"
            border.width: 1
            border.color: root.hairline

            Text {
                x: 10
                y: 8
                width: parent.width - 20
                text: root.guideText()
                color: root.mutedColor
                font.pixelSize: 10
                lineHeight: 1.25
                wrapMode: Text.Wrap
                maximumLineCount: 3
                elide: Text.ElideRight
            }

            Item {
                x: 10
                y: 54
                width: parent.width - 20
                height: 22

                Text {
                    x: 0
                    anchors.verticalCenter: parent.verticalCenter
                    text: "重定向 URI"
                    color: root.mutedColor
                    font.pixelSize: 10
                }

                Rectangle {
                    id: redirectFrame
                    x: 68
                    width: parent.width - 68 - 26
                    height: 22
                    radius: 4
                    color: root.dark ? "#0f1114" : "#ffffff"
                    border.width: 1
                    border.color: root.hairline

                    TextInput {
                        anchors.fill: parent
                        anchors.leftMargin: 6
                        anchors.rightMargin: 6
                        verticalAlignment: TextInput.AlignVCenter
                        readOnly: true
                        selectByMouse: true
                        text: root.redirectUri
                        color: root.dark ? "#c9cfd9" : "#3a4048"
                        selectionColor: root.accent
                        selectedTextColor: "#ffffff"
                        font.pixelSize: 10
                        font.family: "Cascadia Mono"
                    }
                }

                Item {
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    width: 20
                    height: 20

                    Rectangle {
                        anchors.fill: parent
                        radius: 4
                        color: copyMouse.containsMouse
                            ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"

                        Text {
                            anchors.centerIn: parent
                            text: root.copied ? "✓" : "⎘"
                            color: root.copied ? root.accent : root.mutedColor
                            font.pixelSize: 12
                        }
                    }

                    MouseArea {
                        id: copyMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.copyRedirect()
                    }
                }
            }
        }

        Text {
            x: 20
            y: 398
            width: card.width - 40
            height: 24
            text: root.authorizing
                ? "请在刚刚打开的浏览器窗口中完成登录，成功后本窗口会自动关闭。"
                : root.errorText
            color: root.authorizing ? root.mutedColor : root.dangerColor
            font.pixelSize: 10
            lineHeight: 1.2
            wrapMode: Text.Wrap
            elide: Text.ElideRight
        }

        Rectangle {
            id: authorizeButton
            anchors.right: parent.right
            anchors.rightMargin: 14
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 14
            width: 146
            height: 26
            radius: 6
            color: root.authorizing
                ? (root.dark ? "#2b2f36" : "#e2e6eb")
                : (authorizeMouse.containsMouse ? "#3f7ce0" : root.accent)

            Text {
                anchors.centerIn: parent
                text: root.authorizing ? "等待授权中…" : "在浏览器中授权…"
                color: "#ffffff"
                font.pixelSize: 12
            }

            MouseArea {
                id: authorizeMouse
                anchors.fill: parent
                hoverEnabled: true
                enabled: !root.authorizing
                onClicked: root.submit()
            }
        }

        Rectangle {
            anchors.right: authorizeButton.left
            anchors.rightMargin: 8
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 14
            width: 72
            height: 26
            radius: 6
            color: cancelMouse.containsMouse
                ? (root.dark ? "#26ffffff" : "#12000000") : "transparent"
            border.width: 1
            border.color: root.hairline

            Text {
                anchors.centerIn: parent
                text: "取消"
                color: root.dark ? "#c9cfd9" : "#3a4048"
                font.pixelSize: 12
            }

            MouseArea {
                id: cancelMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.dismiss()
            }
        }
    }

    component ProviderCard: Rectangle {
        id: providerCard

        property string provider: ""
        property string label: ""
        property string glyph: ""
        property color tint: "#4285f4"

        readonly property bool selected: root.provider === providerCard.provider
        readonly property bool hovered: providerMouse.containsMouse

        width: 138
        height: 64
        radius: 7
        color: selected
            ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.14)
            : (hovered ? (root.dark ? "#26ffffff" : "#12000000") : "transparent")
        border.width: 1
        border.color: selected ? root.accent : root.hairline

        Accessible.role: Accessible.Button
        Accessible.name: providerCard.label
        Accessible.onPressAction: root.setProvider(providerCard.provider)

        Rectangle {
            anchors.top: parent.top
            anchors.topMargin: 9
            anchors.horizontalCenter: parent.horizontalCenter
            width: 26
            height: 26
            radius: 7
            color: providerCard.tint

            Text {
                anchors.centerIn: parent
                text: providerCard.glyph
                color: "#ffffff"
                font.pixelSize: 13
                font.weight: Font.DemiBold
            }
        }

        Text {
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 8
            anchors.horizontalCenter: parent.horizontalCenter
            text: providerCard.label
            color: providerCard.selected ? root.textColor : (root.dark ? "#c3cad5" : "#3a4048")
            font.pixelSize: 11
        }

        MouseArea {
            id: providerMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            enabled: !root.authorizing
            onClicked: root.setProvider(providerCard.provider)
        }
    }

    component Field: Item {
        id: field

        property string label: ""
        property string placeholder: ""
        property bool password: false
        property alias value: input.text
        property alias inputItem: input

        signal edited()

        height: 52

        Text {
            x: 0
            y: 0
            text: field.label
            color: root.dark ? "#aeb5c0" : "#565e68"
            font.pixelSize: 12
        }

        Rectangle {
            y: 20
            width: parent.width
            height: 30
            radius: 6
            color: root.fieldColor
            border.width: 1
            border.color: input.activeFocus ? root.accent : root.hairline

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 9
                anchors.verticalCenter: parent.verticalCenter
                visible: input.text.length === 0
                text: field.placeholder
                color: root.mutedColor
                font.pixelSize: 12
                elide: Text.ElideRight
                width: parent.width - 18
            }

            TextInput {
                id: input
                anchors.fill: parent
                anchors.leftMargin: 9
                anchors.rightMargin: 9
                verticalAlignment: TextInput.AlignVCenter
                color: root.textColor
                selectionColor: root.accent
                selectedTextColor: "#ffffff"
                font.pixelSize: 12
                echoMode: field.password ? TextInput.Password : TextInput.Normal
                selectByMouse: true
                clip: true

                onTextEdited: field.edited()
                onAccepted: root.submit()
            }
        }
    }
}
