import QtQuick
import QtMultimedia

// In-pane playback for the selected audio or video file.
//
// Loaded through a `Loader` from PreviewPanel so a build without Qt
// Multimedia degrades to the metadata card instead of failing the whole pane.
// Nothing plays until the user asks: a selection change must not make noise.
Item {
    id: root

    property string mediaUrl: ""
    property string mediaKind: "video"
    property string posterSource: ""
    property bool dark: true
    property color accent: Theme.interactive
    property color textColor: Theme.ink
    property color mutedColor: "#8f98aa"
    property color sectionColor: "#1f2229"
    property color hairline: Theme.hairline

    property bool muted: false
    /// True once playback has started, so the poster steps aside for the
    /// video output (which stays black until the first frame arrives).
    property bool started: false

    readonly property bool video: mediaKind === "video"
    readonly property bool playing: player.playbackState === MediaPlayer.PlayingState
    readonly property bool failed: player.error !== MediaPlayer.NoError
    readonly property real progress: player.duration > 0
        ? Math.max(0, Math.min(1, player.position / player.duration)) : 0
    readonly property bool loading: player.mediaStatus === MediaPlayer.LoadingMedia
        || player.mediaStatus === MediaPlayer.StalledMedia
        || player.mediaStatus === MediaPlayer.BufferingMedia

    implicitHeight: layout.implicitHeight

    function durationLabel(milliseconds) {
        if (!(milliseconds > 0))
            return "0:00"
        const total = Math.floor(milliseconds / 1000)
        const hours = Math.floor(total / 3600)
        const minutes = Math.floor((total % 3600) / 60)
        const seconds = total % 60
        const pad = value => value < 10 ? "0" + value : "" + value
        return hours > 0
            ? hours + ":" + pad(minutes) + ":" + pad(seconds)
            : minutes + ":" + pad(seconds)
    }

    function togglePlayback() {
        if (failed)
            return
        if (playing) {
            player.pause()
            return
        }
        if (player.mediaStatus === MediaPlayer.EndOfMedia)
            player.position = 0
        player.play()
    }

    function seekTo(x, width) {
        if (!player.seekable || !(player.duration > 0) || width <= 0)
            return
        const ratio = Math.max(0, Math.min(1, x / width))
        player.position = Math.round(ratio * player.duration)
    }

    onMediaUrlChanged: {
        player.stop()
        started = false
    }

    Column {
        id: layout
        width: parent.width
        spacing: 6

        Rectangle {
            id: surface
            width: parent.width
            height: root.video ? Math.round(width * 9 / 16) : 76
            radius: 12
            color: root.sectionColor
            border.width: 1
            border.color: root.hairline
            clip: true

            VideoOutput {
                id: videoSink
                anchors.fill: parent
                visible: root.video
                fillMode: VideoOutput.PreserveAspectFit
            }

            Image {
                anchors.fill: parent
                anchors.margins: 1
                source: root.video ? root.posterSource : ""
                fillMode: Image.PreserveAspectFit
                asynchronous: true
                cache: false
                visible: root.video && status === Image.Ready && !root.started
            }

            ThemedIcon {
                visible: !root.video
                anchors.centerIn: parent
                category: "audio"
                size: 40
            }

            Item {
                visible: root.video && !root.started && !root.failed
                anchors.centerIn: parent
                width: 52
                height: 52

                Rectangle {
                    anchors.fill: parent
                    radius: width / 2
                    color: root.accent
                    opacity: 0.92
                }

                Canvas {
                    anchors.centerIn: parent
                    width: 16
                    height: 20

                    onPaint: {
                        const context = getContext("2d")
                        context.reset()
                        context.beginPath()
                        context.moveTo(0, 0)
                        context.lineTo(16, 10)
                        context.lineTo(0, 20)
                        context.closePath()
                        context.fillStyle = "#ffffff"
                        context.fill()
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.togglePlayback()
                }
            }

            MouseArea {
                anchors.fill: parent
                enabled: !root.video || (root.started && !root.failed)
                cursorShape: Qt.PointingHandCursor
                onClicked: root.togglePlayback()
            }

            Text {
                anchors.centerIn: parent
                width: parent.width - 24
                visible: root.failed
                text: qsTr("无法播放此文件")
                color: "#e06c75"
                font.pixelSize: 11
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
            }

            Text {
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.margins: 8
                visible: root.video && root.started && root.playing
                text: root.durationLabel(player.position)
                color: Theme.onInteractive
                font.pixelSize: 11
            }
        }

        Item {
            width: parent.width
            height: 14

            Rectangle {
                id: track
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width
                height: 4
                radius: 4
                color: root.dark ? "#3a414c" : "#d5dbe3"
            }

            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: Math.round(track.width * root.progress)
                height: 4
                radius: 4
                color: root.accent
            }

            Rectangle {
                x: Math.round(track.width * root.progress - width / 2)
                anchors.verticalCenter: parent.verticalCenter
                width: 10
                height: 10
                radius: 6
                color: root.accent
                visible: player.seekable
            }

            MouseArea {
                anchors.fill: parent
                enabled: player.seekable
                cursorShape: Qt.PointingHandCursor
                onPressed: function (mouse) {
                    root.seekTo(mouse.x, width)
                }
                onPositionChanged: function (mouse) {
                    if (pressed)
                        root.seekTo(mouse.x, width)
                }
            }
        }

        Item {
            width: parent.width
            height: 26

            Rectangle {
                id: playButton
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                width: 26
                height: 26
                radius: 12
                color: root.accent
                opacity: root.failed ? 0.4 : 1

                Canvas {
                    anchors.centerIn: parent
                    width: 9
                    height: 11
                    visible: !root.playing

                    onPaint: {
                        const context = getContext("2d")
                        context.reset()
                        context.beginPath()
                        context.moveTo(0, 0)
                        context.lineTo(9, 5.5)
                        context.lineTo(0, 11)
                        context.closePath()
                        context.fillStyle = "#ffffff"
                        context.fill()
                    }
                }

                Row {
                    anchors.centerIn: parent
                    spacing: 3
                    visible: root.playing

                    Rectangle {
                        width: 3
                        height: 11
                        radius: 4
                        color: Theme.onInteractive
                    }

                    Rectangle {
                        width: 3
                        height: 11
                        radius: 4
                        color: Theme.onInteractive
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    enabled: !root.failed
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.togglePlayback()
                }
            }

            Text {
                anchors.left: playButton.right
                anchors.leftMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                text: root.durationLabel(player.position) + " / "
                    + (player.duration > 0 ? root.durationLabel(player.duration) : "--:--")
                color: root.mutedColor
                font.pixelSize: 11
                font.family: Theme.monoFamily
            }

            Text {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                text: root.muted ? qsTr("取消静音") : qsTr("静音")
                color: root.mutedColor
                font.pixelSize: 11

                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.muted = !root.muted
                }
            }
        }
    }

    MediaPlayer {
        id: player
        source: root.mediaUrl

        audioOutput: AudioOutput {
            muted: root.muted
        }

        videoOutput: videoSink

        onPlayingChanged: {
            if (playing)
                root.started = true
        }
    }
}
