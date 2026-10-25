import QtQuick
import QtQuick.Window
import dae.imespike 1.0

Window {
    id: root
    x: Screen.width - width - 56
    y: Screen.height - height - 12
    width: 1024
    height: 880
    visible: true
    color: "#101216"
    title: "dae · 中文 IME spike（门槛 2）"

    readonly property var args: Qt.application.arguments
    readonly property bool manualMode: args.indexOf("--manual") >= 0
    readonly property bool keepOpen: args.indexOf("--keep-open") >= 0
    readonly property string outputPath: argument("--out", "results/win11-ime.json")
    readonly property string shotsDir: argument("--shots", "results/shots")

    property string statusLine: "starting"
    property string liveText: ""
    property var imeStatus: ({})
    property var checks: []
    property var notes: []
    property var timeline: []
    property var snapshots: []
    property var scenarioStart: ({})
    property var steps: []
    property int stepIndex: 0
    property int failures: 0
    property double startedAt: 0
    property bool running: false
    property bool preflightRetry: false
    property bool preflightOk: false
    property string activeScene: ""
    property var activeInput: null
    property var lastSnapshot: null
    property string echoA: ""
    property string echoB: ""
    property string echoC: ""

    function argument(name, fallback) {
        for (let i = 0; i + 1 < args.length; ++i)
            if (args[i] === name)
                return args[i + 1]
        return fallback
    }

    function elapsed() {
        return Date.now() - startedAt
    }

    function logLine(text) {
        probe.appendLog(outputPath + ".log", String(text))
    }

    function lettersOnly(value) {
        return String(value || "").replace(/[^A-Za-z']/g, "").replace(/'/g, "").toLowerCase()
    }

    function check(name, ok, detail) {
        const entry = {
            name: name,
            ok: !!ok,
            detail: detail === undefined ? "" : String(detail),
            t_ms: elapsed()
        }
        checks.push(entry)
        if (!ok)
            failures += 1
        const line = (ok ? "PASS " : "FAIL ") + name + (entry.detail ? " :: " + entry.detail : "")
        console.log(line)
        logLine(line)
    }

    function note(name, detail) {
        notes.push({ name: name, detail: String(detail), t_ms: elapsed() })
        const line = "NOTE " + name + " :: " + detail
        console.log(line)
        logLine(line)
    }

    function record(kind, extra) {
        const input = activeInput
        const entry = {
            t_ms: elapsed(),
            kind: kind,
            scene: activeScene,
            focus: input ? input.objectName : "",
            text: input ? input.text : "",
            preedit: input ? input.preeditText : "",
            display: input ? input.displayText : "",
            composing: input ? input.inputMethodComposing : false,
            caret: input ? [input.cursorRectangle.x, input.cursorRectangle.y,
                            input.cursorRectangle.width, input.cursorRectangle.height] : null
        }
        if (extra)
            for (const key in extra)
                entry[key] = extra[key]
        timeline.push(entry)
        statusLine = kind + (entry.scene ? " · " + entry.scene : "")
        return entry
    }

    function slim(snap) {
        return {
            tag: snap.tag,
            t_ms: snap.t_ms,
            qt_version: snap.qt_version,
            os: snap.os,
            dpr: snap.dpr,
            screen: snap.screen,
            focus_class: snap.focus_class,
            focus_name: snap.focus_name,
            window_position: snap.window_position,
            window_size: snap.window_size,
            cursor_rect: snap.cursor_rect,
            cursor_rect_native: snap.cursor_rect_native,
            item_cursor_rect: snap.item_cursor_rect,
            item_cursor_global: snap.item_cursor_global,
            client_origin: snap.client_origin,
            caret_native: snap.caret_native,
            ime: snap.ime,
            last_composition: snap.last_composition,
            last_result: snap.last_result,
            committed: snap.committed,
            ime_windows: snap.ime_windows,
            near_windows: (snap.near_windows || []).slice(0, 16)
        }
    }

    function takeSnapshot(tag) {
        const raw = probe.snapshot()
        let snap
        try {
            snap = JSON.parse(raw)
        } catch (error) {
            snap = { parse_error: String(error), raw: raw }
        }
        snap.tag = tag
        snap.t_ms = elapsed()
        lastSnapshot = snap

        const ime = snap.ime || {}
        const forms = ime.candidate_forms || []
        let first = null
        for (let index = 0; index < forms.length; ++index)
            if (forms[index].style_name.indexOf("CFS_EXCLUDE") >= 0) {
                first = forms[index]
                break
            }
        timeline.push({
            t_ms: elapsed(),
            kind: "native",
            scene: activeScene,
            tag: tag,
            dpr: snap.dpr,
            focus_name: snap.focus_name,
            foreground: snap.foreground,
            window_active: snap.window_active,
            gui_focus_hwnd: snap.gui_focus_hwnd,
            send: snap.send,
            ime_context: ime.context,
            ime_open: ime.open,
            conversion: ime.conversion,
            composition: ime.composition,
            expected_candidate_pt: ime.expected_candidate_pt,
            candidate_pt: first ? first.pt : null,
            candidate_style: first ? first.style_name : null,
            anchor_dx: ime.anchor_dx,
            anchor_dy: ime.anchor_dy,
            candidate_count: ime.candidate_list ? ime.candidate_list.count : -1,
            cursor_rect: snap.cursor_rect,
            caret_native: snap.caret_native,
            client_origin: snap.client_origin,
            window_position: snap.window_position
        })
        snapshots.push(slim(snap))
        return snap
    }

    function checkPreedit(scene, expected, input) {
        const snap = takeSnapshot(scene + ":preedit:" + expected)
        const ime = snap.ime || {}
        const nativeComposition = (ime.composition !== undefined && ime.composition !== "")
                ? ime.composition : (snap.last_composition || "")
        const qmlLetters = lettersOnly(input.preeditText)
        const nativeLetters = lettersOnly(nativeComposition)
        const displayLetters = lettersOnly(input.displayText)

        check(scene + ":preedit-qml", qmlLetters === expected,
              "expected=" + expected + " qml=" + JSON.stringify(input.preeditText)
              + " native=" + JSON.stringify(nativeComposition))
        check(scene + ":preedit-native", nativeLetters === expected,
              "expected=" + expected + " native=" + JSON.stringify(nativeComposition))
        check(scene + ":preedit-visible",
              input.inputMethodComposing && displayLetters.indexOf(expected) >= 0,
              "display=" + JSON.stringify(input.displayText) + " composing=" + input.inputMethodComposing)
        record("preedit-ok", {
            expected: expected,
            qml: input.preeditText,
            native: nativeComposition
        })
    }

    function locateCandidate(snap) {
        const ime = snap.ime || {}
        const forms = ime.candidate_forms || []
        let form = null
        for (let index = 0; index < forms.length; ++index)
            if (forms[index].style_name.indexOf("CFS_EXCLUDE") >= 0) {
                form = forms[index]
                break
            }
        if (!form || !snap.client_origin)
            return { window: null, distance: -1, ime_windows: (snap.ime_windows || []).length }

        const anchor = {
            x: snap.client_origin[0] + form.pt[0],
            y: snap.client_origin[1] + form.pt[1]
        }
        let best = null
        let bestDistance = 1e9
        const windows = snap.ime_windows || []
        for (let index = 0; index < windows.length; ++index) {
            const candidate = windows[index]
            if (candidate.own || !candidate.visible)
                continue
            const rect = candidate.rect
            if (rect[2] < 24 || rect[3] < 16)
                continue
            const dx = Math.max(rect[0] - anchor.x, 0, anchor.x - (rect[0] + rect[2]))
            const dy = Math.max(rect[1] - anchor.y, 0, anchor.y - (rect[1] + rect[3]))
            const distance = Math.max(dx, dy)
            if (distance < bestDistance) {
                bestDistance = distance
                best = candidate
            }
        }
        return { window: best, distance: bestDistance, anchor: [anchor.x, anchor.y], ime_windows: windows.length }
    }

    function checkCandidate(scene, expected, snap) {
        const ime = snap.ime || {}
        check(scene + ":ime-context", ime.context === true, JSON.stringify(ime).substring(0, 320))
        check(scene + ":composition-native", lettersOnly(ime.composition) === expected,
              "expected=" + expected + " native=" + JSON.stringify(ime.composition))
        check(scene + ":candidate-anchor-set", ime.saw_exclude === true,
              "forms=" + JSON.stringify(ime.candidate_forms))
        check(scene + ":candidate-anchor",
              ime.saw_exclude === true && ime.anchor_dx >= 0 && ime.anchor_dx <= 2
              && ime.anchor_dy >= 0 && ime.anchor_dy <= 2,
              "dx=" + ime.anchor_dx + " dy=" + ime.anchor_dy
              + " expected=" + JSON.stringify(ime.expected_candidate_pt)
              + " forms=" + JSON.stringify(ime.candidate_forms))
        check(scene + ":dpr-is-150", Math.abs((snap.dpr || 0) - 1.5) < 0.01, "dpr=" + snap.dpr)

        if (ime.candidate_list_available) {
            check(scene + ":candidate-list", ime.candidate_list.count > 0,
                  "count=" + ime.candidate_list.count
                  + " strings=" + JSON.stringify((ime.candidate_list.strings || []).slice(0, 8)))
        } else {
            note(scene + ":candidate-list",
                 "ImmGetCandidateList returned nothing; the TextInputHost IMM bridge does not expose it")
        }

        const located = locateCandidate(snap)
        note(scene + ":candidate-window", JSON.stringify(located))
        if (located.window) {
            const rect = located.window.rect
            const primary = (snap.screen && snap.screen.primary) ? snap.screen.primary : [0, 0, 2560, 1440]
            check(scene + ":candidate-on-screen",
                  rect[0] >= primary[0] && rect[1] >= primary[1]
                  && rect[0] + rect[2] <= primary[0] + primary[2]
                  && rect[1] + rect[3] <= primary[1] + primary[3],
                  "rect=" + JSON.stringify(rect) + " screen=" + JSON.stringify(primary))
        }
    }

    function checkCommit(scene, input, before, expected) {
        const snap = takeSnapshot(scene + ":committed")
        const appended = input.text.substring(before.length)
        const nativeResult = snap.committed || ""
        check(scene + ":commit-nonempty", appended.length > 0,
              "appended=" + JSON.stringify(appended))
        check(scene + ":commit-cjk",
              /[\u3400-\u9fff]/.test(appended) && appended.indexOf("\uFFFD") < 0,
              "appended=" + JSON.stringify(appended))
        check(scene + ":commit-transport", appended === nativeResult,
              "qml=" + JSON.stringify(appended) + " native=" + JSON.stringify(nativeResult))
        check(scene + ":commit-no-pinyin-left", lettersOnly(appended) === "",
              "appended=" + JSON.stringify(appended))
        check(scene + ":commit-preedit-cleared",
              input.preeditText === "" && !input.inputMethodComposing,
              "preedit=" + JSON.stringify(input.preeditText)
              + " composing=" + input.inputMethodComposing)
        record("commit-ok", {
            appended: appended,
            native: nativeResult,
            expected_pinyin: expected
        })
    }

    function checkCancel(scene, input, before, reason) {
        const snap = takeSnapshot(scene + ":cancelled")
        const appended = input.text.substring(before.length)
        check(scene + ":cancel-preedit-cleared",
              input.preeditText === "" && !input.inputMethodComposing,
              "preedit=" + JSON.stringify(input.preeditText)
              + " composing=" + input.inputMethodComposing)
        check(scene + ":cancel-text-unchanged", input.text === before,
              "before=" + JSON.stringify(before) + " after=" + JSON.stringify(input.text))
        check(scene + ":cancel-no-commit",
              appended === "" && (snap.committed || "") === "",
              "appended=" + JSON.stringify(appended) + " committed=" + JSON.stringify(snap.committed || ""))
        let ended = false
        const events = snap.events || []
        for (let index = 0; index < events.length; ++index)
            if (events[index].name === "WM_IME_ENDCOMPOSITION")
                ended = true
        note(scene + ":cancel-ended", reason + " · WM_IME_ENDCOMPOSITION=" + ended
             + " · events=" + events.length)
    }

    function add(delay, run) {
        steps.push({ delay: delay, run: run })
    }

    function addComposeScenario(scene, input, pinyin, options) {
        options = options || {}

        add(250, function() {
            activeScene = scene
            activeInput = input
            input.forceActiveFocus()
            root.requestActivate()
            probe.focusWindow()
            probe.clickItem(input.objectName)
            input.forceActiveFocus()
            // A previous composition is cancelled explicitly: assigning text
            // does not end an IMM composition, and the leftover segments would
            // be prepended to the next scene's pinyin.
            probe.sendKey(0x1B)
        })

        add(200, function() {
            input.text = options.initialText || ""
            scenarioStart[scene] = input.text
            probe.clearEvents()
            record("scene-start", {
                pinyin: pinyin,
                options: JSON.stringify({
                    backspace: options.backspace || 0,
                    retype: options.retype || "",
                    fast: !!options.fast,
                    cancel: options.cancel || "",
                    initialText: options.initialText || "",
                    screenshot: !!options.screenshot
                })
            })
        })

        add(350, function() {
            check(scene + ":foreground", probe.isForeground(), "foreground=" + probe.isForeground())
            check(scene + ":focus", input.activeFocus, "focus=" + input.activeFocus)
        })

        let typed = ""
        const perLetter = !options.fast
        for (let index = 0; index < pinyin.length; ++index) {
            const letter = pinyin.charAt(index)
            typed += letter
            const expected = typed
            add(perLetter ? 90 : 35, function() {
                if (!probe.isForeground()) {
                    check(scene + ":foreground", false, "lost foreground before " + letter)
                    return
                }
                probe.sendKey(letter.toUpperCase().charCodeAt(0))
            })
            if (perLetter)
                add(130, function() { checkPreedit(scene, expected, input) })
        }

        if (options.backspace) {
            for (let count = 0; count < options.backspace; ++count) {
                const expected = typed.substring(0, typed.length - 1)
                add(120, function() { probe.sendKey(0x08) })
                add(180, function() { checkPreedit(scene, expected, input) })
                typed = expected
            }
            if (options.retype) {
                for (let index = 0; index < options.retype.length; ++index) {
                    const letter = options.retype.charAt(index)
                    typed += letter
                    const expected = typed
                    add(90, function() { probe.sendKey(letter.toUpperCase().charCodeAt(0)) })
                    add(130, function() { checkPreedit(scene, expected, input) })
                }
            }
        }

        add(options.fast ? 450 : 650, function() {
            const snap = takeSnapshot(scene + ":composing")
            checkCandidate(scene, typed, snap)
            if (options.screenshot) {
                const path = shotsDir + "/" + scene + "-composing.png"
                const saved = probe.grab(path)
                note(scene + ":screenshot", path + " ok=" + saved)
            }
        })

        if (options.cancel === "escape") {
            add(150, function() { probe.sendKey(0x1B) })
            add(500, function() { checkCancel(scene, input, scenarioStart[scene], "escape") })
        } else if (options.cancel === "focus") {
            const target = options.cancelTarget
            add(150, function() {
                target.text = ""
                // A real press: the focus-change path the shell will actually
                // take, not a programmatic forceActiveFocus() that skips the
                // window event filter.
                const clicked = probe.clickItem(target.objectName)
                record("focus-move", {
                    target: target.objectName,
                    clicked: clicked,
                    sourceFocus: input.activeFocus
                })
            })
            add(200, function() {
                if (!target.activeFocus) {
                    target.forceActiveFocus()
                    note(scene + ":focus-click", "click did not move focus; forced it")
                }
            })
            add(300, function() {
                check(scene + ":cancel-focus-moved",
                      !input.activeFocus && target.activeFocus,
                      "source=" + input.activeFocus + " target=" + target.activeFocus)
                checkCancel(scene, input, scenarioStart[scene], "focus-change")
            })
        } else {
            add(120, function() { probe.sendKey(0x20) })
            add(650, function() { checkCommit(scene, input, scenarioStart[scene], typed) })
        }
    }

    function buildScript() {
        add(900, function() {
            const attached = probe.attach()
            check("setup:probe-attached", attached, "hwnd")
            root.requestActivate()
            const focused = probe.focusWindow()
            note("setup:focus", "focused=" + focused + " foreground=" + probe.isForeground())
        })

        add(400, function() {
            if (probe.isForeground())
                return
            root.requestActivate()
            probe.focusWindow()
        })

        add(300, function() {
            check("setup:foreground", probe.isForeground(),
                  "foreground=" + probe.isForeground())
            inputA.forceActiveFocus()
            const clicked = probe.clickItem("imeInputA")
            inputA.forceActiveFocus()
            note("setup:click", "clicked=" + clicked + " focus=" + inputA.activeFocus)
        })

        // The IMM context exists only after Qt has a focus object with input
        // enabled, so this must run after the click above.
        add(250, function() {
            const status = JSON.parse(probe.ensureNativeMode())
            imeStatus = status
            check("setup:ime-open", status.ok === true && status.open_after === true,
                  JSON.stringify(status))
            check("setup:ime-native", status.native === true, JSON.stringify(status))
        })

        add(400, function() {
            inputA.text = ""
            echoA = ""
            probe.clearEvents()
            probe.sendKey(0x4E) // N
        })

        add(400, function() {
            preflightOk = lettersOnly(inputA.preeditText) === "n"
            check("preflight:ime-composing", preflightOk,
                  "preedit=" + JSON.stringify(inputA.preeditText)
                  + " text=" + JSON.stringify(inputA.text))
            if (!preflightOk) {
                probe.sendKey(0x1B)
                inputA.text = ""
                probe.clearEvents()
                preflightRetry = true
            }
        })

        add(250, function() {
            if (!preflightRetry)
                return
            probe.sendKey(0x10) // VK_SHIFT toggles Chinese/English in Microsoft Pinyin
            probe.clearEvents()
            inputA.text = ""
            probe.sendKey(0x4E)
        })

        add(400, function() {
            if (!preflightRetry)
                return
            preflightOk = lettersOnly(inputA.preeditText) === "n"
            check("preflight:ime-composing-after-shift", preflightOk,
                  "preedit=" + JSON.stringify(inputA.preeditText)
                  + " text=" + JSON.stringify(inputA.text))
            probe.sendKey(0x1B)
        })

        add(150, function() { probe.sendKey(0x1B) })

        add(300, function() {
            check("preflight:cancelled",
                  inputA.preeditText === "" && !inputA.inputMethodComposing,
                  "preedit=" + JSON.stringify(inputA.preeditText)
                  + " composing=" + inputA.inputMethodComposing)
            inputA.text = ""
            probe.clearEvents()
        })

        addComposeScenario("A-basic-commit", inputA, "nihao", { screenshot: true })
        addComposeScenario("A-backspace", inputA, "nihao",
                           { backspace: 2, retype: "ao" })
        addComposeScenario("A-escape-cancel", inputA, "zhongwen",
                           { initialText: "keep", cancel: "escape" })
        addComposeScenario("A-rapid", inputA, "zhongwenshuru", { fast: true })
        addComposeScenario("B-edge-commit", inputB, "nihao",
                           { screenshot: true })
        addComposeScenario("B-focus-cancel", inputB, "shijie",
                           { initialText: "keep", cancel: "focus", cancelTarget: inputA })
        addComposeScenario("C-scrolled-commit", inputC, "wenjian",
                           { screenshot: true })

        add(250, function() { finish() })
    }

    function runScript() {
        startedAt = Date.now()
        checks = []
        notes = []
        timeline = []
        snapshots = []
        scenarioStart = {}
        failures = 0
        steps = []
        stepIndex = 0
        running = true
        logLine("script: start · args=" + JSON.stringify(args))
        buildScript()
        tickTimer.interval = 50
        tickTimer.start()
    }

    function runNext() {
        if (stepIndex >= steps.length) {
            if (running)
                finish()
            return
        }
        const step = steps[stepIndex++]
        try {
            step.run()
        } catch (error) {
            const line = "EXCEPTION in step " + (stepIndex - 1) + " :: " + String(error)
            console.log(line)
            logLine(line)
            check("step-exception", false, String(error))
        }
        tickTimer.interval = step.delay
        tickTimer.start()
    }

    function allEventLog() {
        try {
            return JSON.parse(probe.eventLog()).events || []
        } catch (error) {
            return []
        }
    }

    function finish() {
        if (!running)
            return
        running = false
        tickTimer.stop()
        manualTimer.stop()
        logLine("finish: start · checks=" + checks.length + " failures=" + failures)

        const passed = failures === 0
        let wrote = false
        let writeError = ""
        try {
            logLine("finish: checks json bytes=" + JSON.stringify(checks).length)
            logLine("finish: notes json bytes=" + JSON.stringify(notes).length)
            logLine("finish: timeline json bytes=" + JSON.stringify(timeline).length)
            logLine("finish: snapshots json bytes=" + JSON.stringify(snapshots).length)
            const events = allEventLog()
            logLine("finish: events json bytes=" + JSON.stringify(events).length)

            const summary = {
                gate: "engine-qt-quick §5 gate 2 — Chinese IME in a custom-styled QML TextInput",
                finished_at: new Date().toISOString(),
                qt: (lastSnapshot && lastSnapshot.qt_version)
                    ? lastSnapshot.qt_version : Qt.application.version,
                os: lastSnapshot ? lastSnapshot.os : "",
                manual: manualMode,
                passed: passed,
                failures: failures,
                check_count: checks.length,
                host: lastSnapshot ? {
                    dpr: lastSnapshot.dpr,
                    screen: lastSnapshot.screen,
                    focus_name: lastSnapshot.focus_name,
                    ime: imeStatus
                } : { ime: imeStatus },
                checks: checks,
                notes: notes,
                scenarios: Object.keys(scenarioStart),
                snapshots: snapshots,
                timeline: timeline,
                events: events
            }
            logLine("finish: summary built")
            const json = JSON.stringify(summary, null, 2)
            logLine("finish: json bytes=" + json.length)
            wrote = probe.writeResults(outputPath, json)
            logLine("finish: wrote=" + wrote)
        } catch (error) {
            writeError = String(error)
            logLine("finish: exception :: " + writeError
                    + " :: " + (error && error.stack ? error.stack : ""))
            try {
                probe.writeResults(outputPath, JSON.stringify({
                    gate: "engine-qt-quick §5 gate 2 — Chinese IME",
                    passed: false,
                    error: writeError,
                    failures: failures,
                    checks: checks,
                    notes: notes,
                    timeline: timeline,
                    partial_snapshots: snapshots.length
                }, null, 2))
            } catch (nested) {
                logLine("finish: fallback failed :: " + String(nested))
            }
        }

        statusLine = (passed ? "PASS" : "FAIL") + " · " + checks.length + " checks · "
                + outputPath + (wrote ? "" : " (write failed: " + writeError + ")")
        console.log(statusLine)
        logLine(statusLine)
        if (!keepOpen)
            Qt.exit(passed ? 0 : 1)
    }

    function updateLive() {
        const snap = takeSnapshot("manual")
        const ime = snap.ime || {}
        liveText = "focus=" + snap.focus_name
                + "  dpr=" + snap.dpr
                + "\npreedit=" + JSON.stringify(activeInput ? activeInput.preeditText : "")
                + "  IME=" + JSON.stringify(ime.composition)
                + "\nopen=" + ime.open + "  conv=" + ime.conversion
                + "  anchorΔ=" + ime.anchor_dx + "," + ime.anchor_dy
                + "\ncaret=" + JSON.stringify(snap.cursor_rect)
                + "\nIME windows=" + (snap.ime_windows || []).length
    }

    ImeProbe {
        id: probe
    }

    Timer {
        id: tickTimer
        repeat: false
        onTriggered: root.runNext()
    }

    Timer {
        id: manualTimer
        interval: 500
        repeat: true
        running: false
        onTriggered: root.updateLive()
    }

    Shortcut {
        sequence: "F12"
        onActivated: {
            const path = root.shotsDir + "/manual-" + Date.now() + ".png"
            root.note("manual-screenshot", path + " ok=" + probe.grab(path))
        }
    }

    Rectangle {
        id: topBar
        width: parent.width
        height: 44
        color: "#171a20"

        Text {
            x: 20
            anchors.verticalCenter: parent.verticalCenter
            text: "dae · 中文 IME · 自定义样式 TextInput · 门槛 2"
            color: "#cfd6e4"
            font.pixelSize: 14
        }

        Text {
            anchors.right: parent.right
            anchors.rightMargin: 20
            anchors.verticalCenter: parent.verticalCenter
            width: 520
            horizontalAlignment: Text.AlignRight
            text: root.statusLine
            color: root.failures > 0 ? "#ff8b8b" : "#8ab4ff"
            font.pixelSize: 12
            elide: Text.ElideRight
        }
    }

    Text {
        x: 48
        y: 66
        text: "A · 路径栏 · 自定义样式 TextInput（textChanged 实时回写镜像）"
        color: "#8f98aa"
        font.pixelSize: 12
    }

    Rectangle {
        id: frameA
        x: 48
        y: 88
        width: 600
        height: 44
        radius: 8
        color: "#171a20"
        border.width: inputA.activeFocus ? 2 : 1
        border.color: inputA.activeFocus ? "#5b8def" : "#2a2f39"

        TextInput {
            id: inputA
            objectName: "imeInputA"
            anchors.fill: parent
            anchors.leftMargin: 14
            anchors.rightMargin: 14
            verticalAlignment: TextInput.AlignVCenter
            color: "#e6e9ef"
            font.pixelSize: 15
            selectionColor: "#3a5fcd"
            selectedTextColor: "#ffffff"
            selectByMouse: true
            cursorDelegate: Rectangle {
                width: 2
                height: 20
                color: "#8ab4ff"
            }
            onTextChanged: root.echoA = text
        }

        Text {
            x: 14
            anchors.verticalCenter: parent.verticalCenter
            visible: inputA.text === "" && !inputA.inputMethodComposing
            text: "点击后输入拼音，例如 nihao…"
            color: "#5c6472"
            font.pixelSize: 15
        }
    }

    Text {
        x: 48
        y: 140
        text: "mirror: " + JSON.stringify(root.echoA)
        color: "#6f7889"
        font.pixelSize: 12
    }

    Text {
        x: 48
        y: 268
        text: "C · 滚动容器内（contentY = 140，候选窗必须跟随内容偏移）"
        color: "#8f98aa"
        font.pixelSize: 12
    }

    Flickable {
        id: scroller
        x: 48
        y: 290
        width: 560
        height: 150
        contentWidth: width
        contentHeight: 420
        contentY: 140
        clip: true
        interactive: false

        Item {
            width: scroller.width
            height: 420

            Text {
                x: 8
                y: 16
                text: "form row 01 · 滚动内容"
                color: "#4c5361"
                font.pixelSize: 12
            }
            Text {
                x: 8
                y: 76
                text: "form row 02 · 滚动内容"
                color: "#4c5361"
                font.pixelSize: 12
            }

            Rectangle {
                x: 0
                y: 170
                width: 480
                height: 44
                radius: 8
                color: "#171a20"
                border.width: inputC.activeFocus ? 2 : 1
                border.color: inputC.activeFocus ? "#5b8def" : "#2a2f39"

                TextInput {
                    id: inputC
                    objectName: "imeInputC"
                    anchors.fill: parent
                    anchors.leftMargin: 14
                    anchors.rightMargin: 14
                    verticalAlignment: TextInput.AlignVCenter
                    color: "#e6e9ef"
                    font.pixelSize: 15
                    selectionColor: "#3a5fcd"
                    selectedTextColor: "#ffffff"
                    cursorDelegate: Rectangle {
                        width: 2
                        height: 20
                        color: "#8ab4ff"
                    }
                    onTextChanged: root.echoC = text
                }

                Text {
                    x: 14
                    anchors.verticalCenter: parent.verticalCenter
                    visible: inputC.text === "" && !inputC.inputMethodComposing
                    text: "滚动容器内的输入框…"
                    color: "#5c6472"
                    font.pixelSize: 15
                }
            }

            Text {
                x: 8
                y: 236
                text: "form row 03 · 滚动内容"
                color: "#4c5361"
                font.pixelSize: 12
            }
        }
    }

    Rectangle {
        x: 48
        y: 456
        width: 560
        height: 1
        color: "#22262e"
    }

    Text {
        x: 48
        y: 476
        width: 560
        text: "脚本会自动输入拼音。每个字母同时对照两条通道：QML TextInput.preeditText 与 IMM32 的 GCS_COMPSTR；提交后对照 GCS_RESULTSTR。候选窗位置由 Qt 交给 IME 的 CFS_EXCLUDE 坐标与 QInputMethod.cursorRectangle × DPR 对比，并抓屏留证。"
        color: "#6f7889"
        font.pixelSize: 12
        wrapMode: Text.Wrap
        lineHeight: 1.35
    }

    Text {
        x: 48
        y: 556
        text: root.manualMode
              ? "手动模式：切到微软拼音，在任一输入框里打拼音；F12 抓屏。关闭窗口结束。"
              : "自动模式：窗口获得焦点后开始注入按键；不要切换窗口。"
        color: "#8ab4ff"
        font.pixelSize: 12
    }

    Rectangle {
        anchors.right: parent.right
        anchors.rightMargin: 24
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 12
        width: 420
        height: 88
        radius: 10
        color: "#171a20"
        border.width: inputB.activeFocus ? 2 : 1
        border.color: inputB.activeFocus ? "#5b8def" : "#2a2f39"

        Text {
            x: 14
            y: 9
            text: "B · 命令栏 · 屏幕右下角（候选窗需要翻转）"
            color: "#6f7889"
            font.pixelSize: 11
        }

        TextInput {
            id: inputB
            objectName: "imeInputB"
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.leftMargin: 14
            anchors.rightMargin: 14
            anchors.bottomMargin: 12
            height: 40
            verticalAlignment: TextInput.AlignVCenter
            color: "#e6e9ef"
            font.pixelSize: 15
            selectionColor: "#3a5fcd"
            selectedTextColor: "#ffffff"
            cursorDelegate: Rectangle {
                width: 2
                height: 20
                color: "#8ab4ff"
            }
            onTextChanged: root.echoB = text
        }

        Text {
            x: 14
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 20
            visible: inputB.text === "" && !inputB.inputMethodComposing
            text: "命令栏输入…"
            color: "#5c6472"
            font.pixelSize: 15
        }
    }

    Rectangle {
        anchors.top: parent.top
        anchors.topMargin: 60
        anchors.right: parent.right
        anchors.rightMargin: 24
        width: 420
        height: 150
        radius: 8
        color: "#14161b"
        border.width: 1
        border.color: "#22262e"

        Text {
            id: liveInfo
            anchors.fill: parent
            anchors.margins: 10
            text: root.liveText === "" ? "probe: idle" : root.liveText
            color: "#8f98aa"
            font.family: "Consolas"
            font.pixelSize: 11
            wrapMode: Text.WrapAnywhere
        }
    }

    Component.onCompleted: {
        startedAt = Date.now()
        if (manualMode) {
            statusLine = "manual mode · F12 抓屏"
            probe.attach()
            probe.focusWindow()
            manualTimer.start()
            inputA.forceActiveFocus()
            note("manual", "attach=" + probe.isForeground() + " dpr=" + (lastSnapshot ? lastSnapshot.dpr : ""))
        } else {
            runScript()
        }
    }
}
