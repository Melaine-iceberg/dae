import QtQuick
import dae.listspike 1.0

Window {
    id: root
    width: 1180
    height: 820
    visible: true
    color: "#111317"
    title: "dae · Qt list spike"

    readonly property var args: Qt.application.arguments
    readonly property int rowHeight: 26
    readonly property string datasetPath: argument("--path", "K:/dae-listing-bench/huge")
    readonly property int measureSeconds: parseInt(argument("--seconds", "12"))
    readonly property int settleMs: parseInt(argument("--settle", "800"))
    readonly property string outputPath: argument("--out", "results/qt-list-spike.json")
    readonly property string grabPath: argument("--grab", "")
    readonly property bool patchesEnabled: argument("--patches", "on") !== "off"
    readonly property bool jumpsEnabled: argument("--jumps", "on") !== "off"

    property bool scrollActive: false
    property real scrollStep: rowHeight * 3
    property int jumps: 0
    property int patched: 0
    property int delegatesCreated: 0

    function argument(name, fallback) {
        for (var i = 0; i + 1 < args.length; ++i)
            if (args[i] === name)
                return args[i + 1]
        return fallback
    }

    DirectoryListModel {
        id: dirModel
    }

    FrameProbe {
        id: probe
    }

    Column {
        anchors.fill: parent

        Rectangle {
            width: parent.width
            height: 34
            color: "#191c22"

            Text {
                x: 12
                anchors.verticalCenter: parent.verticalCenter
                text: "dae / QAbstractListModel + ListView.reuseItems"
                color: "#cfd6e4"
                font.pixelSize: 13
            }

            Text {
                anchors.right: parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                text: dirModel.finished
                      ? dirModel.status
                      : (dirModel.loaded + " rows · reading…")
                color: dirModel.finished ? "#7d8698" : "#8ab4ff"
                font.pixelSize: 12
            }
        }

        ListView {
            id: list
            width: parent.width
            height: parent.height - 34
            model: dirModel
            reuseItems: true
            cacheBuffer: 2600
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            snapMode: ListView.NoSnap
            synchronousDrag: false
            spacing: 0

            delegate: Item {
                id: delegateRoot
                required property int index
                required property string fileName
                required property string sizeText
                required property string modifiedText
                required property bool isDir
                required property bool hidden

                width: ListView.view.width
                height: root.rowHeight

                Component.onCompleted: root.delegatesCreated += 1

                Rectangle {
                    anchors.fill: parent
                    color: (delegateRoot.index % 2) === 0 ? "#111317" : "#15181e"
                }

                Rectangle {
                    x: 12
                    y: 6
                    width: 14
                    height: 14
                    radius: 3
                    color: delegateRoot.isDir ? "#5b8def" : "#3b4250"
                }

                Text {
                    x: 36
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.max(0, delegateRoot.width - 36 - 340)
                    text: delegateRoot.fileName
                    color: delegateRoot.hidden ? "#6b7280" : "#e6e9ef"
                    font.pixelSize: 13
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                }

                Text {
                    anchors.right: modifiedTextItem.left
                    anchors.rightMargin: 24
                    anchors.verticalCenter: parent.verticalCenter
                    width: 90
                    horizontalAlignment: Text.AlignRight
                    text: delegateRoot.sizeText
                    color: "#8f98aa"
                    font.pixelSize: 12
                    textFormat: Text.PlainText
                }

                Text {
                    id: modifiedTextItem
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    width: 140
                    horizontalAlignment: Text.AlignRight
                    text: delegateRoot.modifiedText
                    color: "#8f98aa"
                    font.pixelSize: 12
                    textFormat: Text.PlainText
                }
            }
        }
    }

    // The scroll loop is driven by presented frames, not by a timer: each
    // frame moves the offset, which requests the next frame. With
    // QSG_NO_VSYNC=1 the resulting interval is how long a frame takes; with
    // vsync on it is the display cadence.
    Connections {
        target: root

        function onFrameSwapped() {
            probe.onFrame()
            if (!root.scrollActive)
                return
            var maxY = list.contentHeight - list.height
            if (maxY <= 0)
                return
            var next = list.contentY + root.scrollStep
            if (next >= maxY) {
                next = maxY
                root.scrollStep = -Math.abs(root.scrollStep)
            } else if (next <= 0) {
                next = 0
                root.scrollStep = Math.abs(root.scrollStep)
            }
            list.contentY = next
        }
    }

    // A scrollbar jump every second. The smooth scroll only visits a few
    // thousand rows in twelve seconds; the jumps make the model serve rows
    // from every part of the 120k range.
    Timer {
        interval: 1000
        repeat: true
        running: root.scrollActive && root.jumpsEnabled

        onTriggered: {
            var maxY = Math.max(0, list.contentHeight - list.height)
            if (maxY > 0)
                list.contentY = Math.random() * maxY
            root.jumps += 1
        }
    }

    // A file change in the visible band: one dataChanged over 24 rows every
    // 250 ms, queued from the timer so it lands while the view is scrolling.
    Timer {
        interval: 250
        repeat: true
        running: root.scrollActive && root.patchesEnabled

        onTriggered: {
            var first = Math.max(0, Math.floor(list.contentY / root.rowHeight) + 8)
            dirModel.applyPatches(first, 24)
            probe.notePatch(dirModel.last_patch_us)
            root.patched += 1
        }
    }

    // A PSS reading every five seconds, so a soak shows whether memory
    // plateaus or keeps growing. One sample is a few hundred microseconds.
    Timer {
        interval: 5000
        repeat: true
        running: root.scrollActive
        onTriggered: probe.sampleMemory("scroll")
    }

    // Optional one-shot capture for inspection; off unless --grab is passed.
    Timer {
        id: grabTimer
        interval: 500
        running: root.scrollActive && root.grabPath !== ""
        onTriggered: root.contentItem.grabToImage(function(result) {
            result.saveToFile(root.grabPath)
        })
    }

    Timer {
        id: settleTimer
        interval: root.settleMs
        onTriggered: {
            probe.begin()
            root.scrollActive = true
            measureTimer.start()
        }
    }

    Timer {
        id: measureTimer
        interval: root.measureSeconds * 1000
        onTriggered: {
            root.scrollActive = false
            var summary = probe.finish(root.outputPath, dirModel.loaded,
                                       dirModel.load_ms, dirModel.batches,
                                       root.delegatesCreated,
                                       root.patchesEnabled, root.jumpsEnabled)
            root.title = summary
            Qt.quit()
        }
    }

    Component.onCompleted: {
        probe.sampleMemory("process-start")
        dirModel.start(root.datasetPath, "spike-" + Date.now())
    }

    Connections {
        target: dirModel

        function onFinishedChanged() {
            if (!dirModel.finished)
                return
            if (dirModel.loaded === 0) {
                console.error("listing failed: " + dirModel.status)
                Qt.quit()
                return
            }
            probe.sampleMemory("loaded")
            settleTimer.start()
        }
    }
}
