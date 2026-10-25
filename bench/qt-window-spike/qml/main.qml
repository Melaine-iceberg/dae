import QtQuick
import QtQuick.Window
import dae.windowspike 1.0

Window {
    id: root
    width: 1100
    height: 720
    x: 320
    y: 120
    visible: true
    title: gate4 ? "dae · 门槛 4 · 撕出" : "dae · 门槛 3 · 无边框 + Mica"
    flags: Qt.Window | Qt.FramelessWindowHint
        | Qt.WindowMinimizeButtonHint | Qt.WindowMaximizeButtonHint
        | Qt.WindowCloseButtonHint
    color: "transparent"

    readonly property var args: Qt.application.arguments
    readonly property bool manualMode: args.indexOf("--manual") >= 0
    readonly property bool probeOnly: args.indexOf("--probe-only") >= 0
    readonly property bool flyoutProbe: args.indexOf("--flyout-probe") >= 0
    readonly property bool snapProbe: args.indexOf("--snap-probe") >= 0
    readonly property bool idleProbe: args.indexOf("--idle-probe") >= 0
    readonly property bool maxProbe: args.indexOf("--max-probe") >= 0
    readonly property bool gate4: args.indexOf("--gate4") >= 0
    property var idleState: ({"tick": 0, "framesStart": 0, "framesIdle": 0,
                              "framesInvokable": 0})
    readonly property bool autoRun: !manualMode && !probeOnly && !flyoutProbe && !snapProbe
        && !idleProbe && !maxProbe && !gate4
    readonly property string outputPath: argument("--out",
        gate4 ? "results/win11-gate4.json" : "results/win11-gate3.json")
    readonly property string shotsDir: argument("--shots",
        gate4 ? "results" : "results/shots")
    readonly property string datasetPath: argument("--path", "K:/dae-listing-bench/huge")
    readonly property int cycles: parseInt(argument("--cycles", "6"))
    readonly property int settleMs: parseInt(argument("--settle", "800"))

    property bool dark: true
    property string material: "mica"
    property string statusLine: "starting"
    property var info: ({})
    property var materialInfo: ({})

    property var checks: []
    property var notes: []
    property int failures: 0
    property var plan: []
    property int planIndex: 0
    property bool running: false
    property bool quiet: false
    property bool hold: false
    property double startedAt: 0
    property var regions: ({})
    property var rectBeforeDrag: null
    property var rectBeforeResize: null
    property var rectNormal: null
    property var samples: []
    property var initialWindow: ({})
    property var dwmSummary: ({})
    property var dwmStatsBefore: ({})
    property var dwmState: ({})

    // Gate 4 state.
    property var secondaryWindow: null
    property int frameCycle: -1
    property var tearOffCycles: []
    property var tearOffLatencies: []
    property var tearOffOpenIncrements: []
    property var tearOffCloseResiduals: []
    property var tearOffClosePrivateResiduals: []
    property var secondaryRectBeforeDrag: null
    property bool secondaryClosing: false
    property bool finished: false

    readonly property bool maximized: visibility === Window.Maximized
    readonly property color chromeTint: dark
        ? Qt.rgba(0.055, 0.063, 0.075, 0.38)
        : Qt.rgba(0.98, 0.985, 0.99, 0.46)
    readonly property color solidChrome: dark ? "#14161a" : "#f4f6f8"
    readonly property color chromeColor: material === "none" ? solidChrome : chromeTint
    readonly property color contentColor: dark ? "#191b1f" : "#ffffff"
    readonly property color textColor: dark ? "#e7e9ee" : "#1c1f24"
    readonly property color mutedColor: dark ? "#8d94a2" : "#657080"
    readonly property color hairline: dark ? "#26ffffff" : "#16000000"

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
        const line = (ok ? "PASS " : "FAIL ") + name
            + (entry.detail ? " :: " + entry.detail : "")
        console.log(line)
        logLine(line)
    }

    function note(name, detail) {
        notes.push({ name: name, detail: String(detail), t_ms: elapsed() })
        const line = "NOTE " + name + " :: " + detail
        console.log(line)
        logLine(line)
    }

    function refresh() {
        try {
            info = JSON.parse(probe.windowInfo())
            materialInfo = JSON.parse(probe.materialReport())
            statusLine = (materialInfo.material || "none") + " · build "
                + (materialInfo.build || "?") + " · backdrop "
                + (info.backdrop ? info.backdrop.read_backdrop : "?")
                + " · " + (maximized ? "maximized" : "normal")
        } catch (error) {
            statusLine = "probe error: " + error
        }
    }

    function readInfo() {
        info = JSON.parse(probe.windowInfo())
        return info
    }

    function rect() {
        return readInfo().window_rect
    }

    function count(map, key) {
        return map && map[key] !== undefined ? Number(map[key]) : 0
    }

    function moveCursor(point) {
        return probe.moveCursor(point[0], point[1])
    }

    function schedule(label, delay, fn) {
        plan.push({ label: label, delay: delay, fn: fn })
    }

    function pump() {
        if (!running)
            return
        if (planIndex >= plan.length) {
            finishRun()
            return
        }
        const step = plan[planIndex++]
        if (!quiet)
            statusLine = step.label
        try {
            step.fn()
        } catch (error) {
            check("plan:" + step.label + ":threw", false, String(error))
        }
        if (!running)
            return
        if (planIndex >= plan.length) {
            if (!hold)
                finishRun()
            return
        }
        planTimer.interval = Math.max(1, plan[planIndex].delay)
        planTimer.restart()
    }

    function startRun() {
        if (!probe.attach()) {
            statusLine = "attach failed"
            console.log("probe: attach failed")
            Qt.exit(2)
            return
        }
        startedAt = Date.now()
        running = true
        material = "mica"
        probe.applyMaterial(material, true)
        planIndex = 0
        planTimer.interval = 1
        planTimer.start()
        ensureForeground(0)
    }

    function ensureForeground(attempt) {
        const ok = probe.setForeground()
        if (ok) {
            schedule("settle", 800, preflight)
            return
        }
        if (attempt >= 20) {
            check("attach:foreground", false, "could not take foreground after retries")
            finishRun()
            return
        }
        schedule("focus-retry", 100, function() { ensureForeground(attempt + 1) })
    }

    // ------------------------------------------------------------------
    // 1. Preflight: decision, application and two visual states

    function preflight() {
        refresh()
        initialWindow = JSON.parse(JSON.stringify(info))
        const backdrop = info.backdrop || {}
        check("attach:window-handle", info.hwnd !== undefined && info.hwnd !== "0",
              "hwnd=" + info.hwnd)
        check("attach:foreground", info.foreground === true)
        check("material:build-read", materialInfo.build >= 10000,
              "build=" + materialInfo.build)
        check("material:transparency-on", materialInfo.transparency !== false)
        check("material:mica-supported", materialInfo.build >= materialInfo.mica_min_build,
              "mica from " + materialInfo.mica_min_build)
        note("material:tauri-decision", materialInfo.material
             + " (the Tauri backend picks this on build " + materialInfo.build + ")")
        check("backdrop:mica-accepted", backdrop.read_backdrop === 2,
              "read_backdrop=" + backdrop.read_backdrop)
        check("backdrop:dark-accepted", backdrop.read_dark === true)
        check("backdrop:transparent-path", backdrop.layered === true
              && backdrop.blur_behind === true,
              "layered=" + backdrop.layered + " blur_behind=" + backdrop.blur_behind)
        const style = Number("0x" + info.style)
        check("chrome:native-style",
              (style & 0x00040000) !== 0 && (style & 0x00010000) !== 0,
              "WS_THICKFRAME/WS_MAXIMIZEBOX style=0x" + info.style)
        probe.captureWindow(shotsDir + "/mica-dark.png")

        schedule("capture:dark", 250, function() {})
        schedule("theme:light", 0, function() {
            root.dark = false
            probe.applyMaterial("mica", false)
        })
        schedule("capture:light", 450, function() {
            refresh()
            check("backdrop:mica-light-accepted", info.backdrop.read_backdrop === 2
                  && info.backdrop.read_dark === false,
                  "read=" + info.backdrop.read_backdrop + " dark=" + info.backdrop.read_dark)
            probe.captureWindow(shotsDir + "/mica-light.png")
        })
        schedule("theme:dark", 0, function() {
            root.dark = true
            probe.applyMaterial("mica", true)
        })
        schedule("hit-tests", 350, hitTests)
    }

    // ------------------------------------------------------------------
    // 2. Native hit test map

    function hitTests() {
        regions = JSON.parse(probe.chromeRegions())
        probe.clearStats()
        const titlebar = regions.titlebar_px
        const button = regions.button_px
        const dpr = info.dpr
        check("regions:titlebar-height", titlebar === Math.round(40 * dpr),
              "px=" + titlebar + " dpr=" + dpr)
        check("regions:button-width", button === Math.round(46 * dpr), "px=" + button)

        // The maximize button is the one native non-client zone, because
        // returning HTMAXBUTTON on hover is what opens Snap Layouts. While it
        // is pressed it is HTCLIENT so QML receives the click; minimize and
        // close are always client areas.
        const expected = {
            caption: 2, min: 1, max: 9, close: 1,
            border_left: 10, border_right: 11, border_top: 12, border_bottom: 15,
            corner_tl: 13, corner_br: 17, content: 1
        }
        for (const name in expected) {
            const point = regions[name]
            const code = probe.hitTest(point[0], point[1])
            check("hit:" + name, code === expected[name],
                  "at " + point + " got " + code + " want " + expected[name])
        }
        const stats = JSON.parse(probe.chromeStats())
        check("hit:max-observed-by-messages", count(stats.hit_test, "9") >= 1)
        check("hit:caption-observed-by-messages", count(stats.hit_test, "2") >= 1)
        schedule("drag", 100, startDrag)
    }

    // ------------------------------------------------------------------
    // 3. Drag, resize

    function startDrag() {
        rectBeforeDrag = rect()
        probe.clearStats()
        probe.setForeground()
        const point = regions.caption
        const dx = 80
        const dy = 55
        schedule("drag:down", 0, function() {
            moveCursor(point)
        })
        schedule("drag:press", 80, function() {
            check("drag:mouse-down", probe.mouseDown())
        })
        for (let step = 1; step <= 10; ++step) {
            schedule("drag:move" + step, 18, function() {
                probe.moveCursor(point[0] + Math.round(dx * step / 10),
                                 point[1] + Math.round(dy * step / 10))
            })
        }
        schedule("drag:up", 80, function() {
            probe.mouseUp()
        })
        schedule("drag:check", 350, checkDrag)
    }

    function checkDrag() {
        const after = rect()
        const movedX = after[0] - rectBeforeDrag[0]
        const movedY = after[1] - rectBeforeDrag[1]
        check("drag:moved-by-pointer", Math.abs(movedX - 80) <= 3 && Math.abs(movedY - 55) <= 3,
              "moved " + movedX + "," + movedY)
        check("drag:size-unchanged",
              (after[2] - after[0]) === (rectBeforeDrag[2] - rectBeforeDrag[0])
              && (after[3] - after[1]) === (rectBeforeDrag[3] - rectBeforeDrag[1]),
              JSON.stringify(after))
        const stats = JSON.parse(probe.chromeStats())
        check("drag:hit-caption", count(stats.hit_test, "2") >= 1,
              "hits=" + count(stats.hit_test, "2"))
        check("drag:nc-down-caption", count(stats.nc_down, "2") >= 1,
              "nc_down=" + JSON.stringify(stats.nc_down))
        // Qt's native event filter skips WM_MOVING/WM_SIZING (they are in
        // QWindowsContext::isInputMessage), so those counters stay zero by
        // design; WM_ENTERSIZEMOVE/EXITSIZEMOVE are the visible proof that the
        // native modal loop ran.
        check("drag:native-move-loop",
              stats.entersizemove >= 1 && stats.exitsizemove >= 1,
              "enter=" + stats.entersizemove + " exit=" + stats.exitsizemove)
        schedule("resize-left", 100, startResizeLeft)
    }

    function startResizeLeft() {
        rectBeforeResize = rect()
        probe.clearStats()
        probe.setForeground()
        const point = JSON.parse(probe.chromeRegions()).border_left
        schedule("resize:down", 0, function() {
            moveCursor(point)
        })
        schedule("resize:press", 80, function() {
            check("resize:mouse-down", probe.mouseDown())
        })
        for (let step = 1; step <= 10; ++step) {
            schedule("resize:move" + step, 18, function() {
                probe.moveCursor(point[0] - 12 * step, point[1])
            })
        }
        schedule("resize:up", 80, function() {
            probe.mouseUp()
        })
        schedule("resize:check", 350, checkResizeLeft)
    }

    function checkResizeLeft() {
        const after = rect()
        const widthDelta = (after[2] - after[0]) - (rectBeforeResize[2] - rectBeforeResize[0])
        const leftDelta = after[0] - rectBeforeResize[0]
        check("resize:width-grew", Math.abs(widthDelta - 120) <= 3, "width +" + widthDelta)
        check("resize:left-edge-moved", Math.abs(leftDelta + 120) <= 3, "left " + leftDelta)
        check("resize:height-unchanged",
              (after[3] - after[1]) === (rectBeforeResize[3] - rectBeforeResize[1]))
        const stats = JSON.parse(probe.chromeStats())
        check("resize:hit-left", count(stats.hit_test, "10") >= 1)
        check("resize:nc-down-left", count(stats.nc_down, "10") >= 1)
        check("resize:native-size-loop",
              stats.entersizemove >= 1 && stats.exitsizemove >= 1,
              "enter=" + stats.entersizemove + " exit=" + stats.exitsizemove)
        schedule("resize-corner", 100, startResizeCorner)
    }

    function startResizeCorner() {
        rectBeforeResize = rect()
        probe.clearStats()
        probe.setForeground()
        const point = JSON.parse(probe.chromeRegions()).corner_br
        schedule("corner:down", 0, function() {
            moveCursor(point)
        })
        schedule("corner:press", 80, function() {
            check("corner:mouse-down", probe.mouseDown())
        })
        for (let step = 1; step <= 10; ++step) {
            schedule("corner:move" + step, 18, function() {
                probe.moveCursor(point[0] + 6 * step, point[1] + 4 * step)
            })
        }
        schedule("corner:up", 80, function() {
            probe.mouseUp()
        })
        schedule("corner:check", 350, checkResizeCorner)
    }

    function checkResizeCorner() {
        const after = rect()
        const widthDelta = (after[2] - after[0]) - (rectBeforeResize[2] - rectBeforeResize[0])
        const heightDelta = (after[3] - after[1]) - (rectBeforeResize[3] - rectBeforeResize[1])
        check("corner:width-grew", Math.abs(widthDelta - 60) <= 3, "width +" + widthDelta)
        check("corner:height-grew", Math.abs(heightDelta - 40) <= 3, "height +" + heightDelta)
        const stats = JSON.parse(probe.chromeStats())
        check("corner:hit-bottom-right", count(stats.hit_test, "17") >= 1)
        schedule("maximize", 100, startMaximize)
    }

    // ------------------------------------------------------------------
    // 4. Maximize / restore through the custom button

    function startMaximize() {
        rectNormal = rect()
        const point = JSON.parse(probe.chromeRegions()).max
        probe.setForeground()
        probe.clearStats()
        schedule("maximize:click", 0, function() {
            check("maximize:click", probe.click(point[0], point[1]),
                  "at " + JSON.stringify(point))
        })
        schedule("maximize:check", 650, checkMaximized)
    }

    function checkMaximized() {
        const current = readInfo()
        const maxStats = JSON.parse(probe.chromeStats())
        note("maximize:after", "nc_up=" + maxStats.nc_up
             + " actions=" + maxStats.chrome_actions + " zoomed=" + current.zoomed)
        check("maximize:zoomed", current.zoomed === true)
        const work = current.work_area
        const origin = current.client_origin
        const client = current.client_rect
        check("maximize:client-is-work-area",
              client[2] === work[2] - work[0] && client[3] === work[3] - work[1]
              && origin[0] === work[0] && origin[1] === work[1],
              "client " + client + " origin " + origin + " work " + work)
        const zoomed = JSON.parse(probe.chromeRegions())
        check("maximize:no-resize-border",
              probe.hitTest(zoomed.border_left[0], zoomed.border_left[1]) === 1)
        check("maximize:max-button-still-max",
              probe.hitTest(zoomed.max[0], zoomed.max[1]) === 9)
        probe.captureWindow(shotsDir + "/maximized.png")

        schedule("restore:click", 150, function() {
            probe.click(zoomed.max[0], zoomed.max[1])
        })
        schedule("restore:check", 650, checkRestored)
    }

    function checkRestored() {
        const current = readInfo()
        check("restore:not-zoomed", current.zoomed === false)
        const restored = current.window_rect
        check("restore:same-rect",
              Math.abs(restored[0] - rectNormal[0]) <= 2
              && Math.abs(restored[1] - rectNormal[1]) <= 2
              && Math.abs((restored[2] - restored[0]) - (rectNormal[2] - rectNormal[0])) <= 2
              && Math.abs((restored[3] - restored[1]) - (rectNormal[3] - rectNormal[1])) <= 2,
              JSON.stringify(restored) + " vs " + JSON.stringify(rectNormal))
        // The cursor is still on the maximize button; moving away lets any
        // hover flyout close before the tint samples start.
        schedule("leave-max", 0, function() {
            probe.moveCursor(current.client_origin[0] + 400,
                             current.client_origin[1] + 400)
        })
        schedule("dwm-switch", 700, startDwmSwitch)
    }

    // ------------------------------------------------------------------
    // 5. DWM tint switch without a repaint

    function startDwmSwitch() {
        const current = readInfo()
        const windowRect = current.window_rect
        const x = windowRect[0] + 450
        const y = windowRect[1]
        const width = (windowRect[2] - windowRect[0]) - 900
        const height = current.titlebar_px
        quiet = true
        probe.captureWindow(shotsDir + "/dwm-before.png")
        probe.beginSamples(x, y, width, height)
        probe.clearStats()
        dwmStatsBefore = JSON.parse(probe.chromeStats())
        dwmState = { tick: 0 }
        hold = true
        dwmTimer.start()
    }

    // The samples run from one repeating timer rather than one plan step each.
    // The plan's per-step property writes were enough to make Qt present a
    // frame per sample in the first version of this run, which hid whether the
    // DWM switch itself caused a repaint.
    function dwmTick() {
        dwmState.tick += 1
        const tick = dwmState.tick
        if (tick <= 8) {
            probe.sample("dark", "")
        } else if (tick === 9) {
            probe.applyMaterial("mica", false)
        } else if (tick <= 27) {
            probe.sample("light", "")
            if (tick === 19)
                probe.captureWindow(shotsDir + "/dwm-light-settled.png")
        } else if (tick === 28) {
            probe.applyMaterial("mica", true)
        } else if (tick <= 36) {
            probe.sample("dark", "")
            if (tick === 36)
                probe.captureWindow(shotsDir + "/dwm-dark-settled.png")
        } else {
            dwmTimer.stop()
            analyzeDwmSwitch(dwmStatsBefore)
        }
    }

    function average(values) {
        if (!values.length)
            return 0
        let sum = 0
        for (let index = 0; index < values.length; ++index)
            sum += values[index]
        return sum / values.length
    }

    function analyzeDwmSwitch(statsBefore) {
        quiet = false
        samples = JSON.parse(probe.sampleReport())
        const darkSamples = samples.filter(function(sample) { return sample.tag === "dark" })
        const lightSamples = samples.filter(function(sample) { return sample.tag === "light" })
        let blackFrames = 0
        let whiteFrames = 0
        for (let index = 0; index < samples.length; ++index) {
            if (samples[index].mean < 8)
                blackFrames += 1
            if (samples[index].mean > 247)
                whiteFrames += 1
        }

        // The endpoints are the stable plateaus, not the averages. DWM
        // cross-fades the tint (about 150 ms at 150%); the middle of that ramp
        // is legitimate, and averaging it into the light endpoint would make
        // the plateau look like an overshoot. The first five dark samples are
        // before the switch; the last five light samples are after the ramp.
        const firstDark = darkSamples.slice(0, Math.min(5, darkSamples.length))
        const stableDark = average(firstDark.map(function(sample) { return sample.mean }))
        const stableLight = average(lightSamples.slice(Math.max(0, lightSamples.length - 5))
                                    .map(function(sample) { return sample.mean }))
        const pad = 6.0
        const low = Math.min(stableDark, stableLight) - pad
        const high = Math.max(stableDark, stableLight) + pad
        let outside = 0
        let intermediates = 0
        let worstJump = 0
        for (let index = 0; index < samples.length; ++index) {
            const mean = samples[index].mean
            outside = Math.max(outside, Math.max(low - mean, mean - high, 0))
            if (mean > Math.min(stableDark, stableLight) + pad
                    && mean < Math.max(stableDark, stableLight) - pad)
                intermediates += 1
            if (index > 0)
                worstJump = Math.max(worstJump, Math.abs(mean - samples[index - 1].mean))
        }
        const statsAfter = JSON.parse(probe.chromeStats())
        const framesDrawn = statsAfter.frames - statsBefore.frames
        const erases = statsAfter.erasebkgnd - statsBefore.erasebkgnd
        dwmSummary = {
            stable_dark: stableDark,
            stable_light: stableLight,
            samples: samples.length,
            in_between: intermediates,
            worst_jump: worstJump,
            qt_frames: framesDrawn,
            erasebkgnd: erases
        }

        check("dwm:two-stable-levels", Math.abs(stableLight - stableDark) >= 2.0,
              "dark=" + stableDark.toFixed(2) + " light=" + stableLight.toFixed(2))
        check("dwm:no-black-frame", blackFrames === 0, "samples=" + blackFrames)
        check("dwm:no-white-frame", whiteFrames === 0, "samples=" + whiteFrames)
        check("dwm:no-out-of-envelope-frame", outside === 0,
              "worst outside=" + outside.toFixed(2)
              + " envelope " + low.toFixed(1) + ".." + high.toFixed(1))
        check("dwm:transition-sampled", intermediates >= 3,
              "in-between samples=" + intermediates)
        check("dwm:no-background-erase", erases === 0, "WM_ERASEBKGND=" + erases)
        note("dwm:samples", samples.length + " samples at ~20 ms, worst adjacent jump "
             + worstJump.toFixed(2))
        note("dwm:levels", "stable dark " + stableDark.toFixed(2) + " / light "
             + stableLight.toFixed(2) + ", cross-fade over "
             + intermediates + " samples")
        note("dwm:qt-frames", "Qt presented " + framesDrawn
             + " frames during the capture sequence")
        quiet = false
        hold = false
        schedule("snap", 200, startSnapLayouts)
        planTimer.interval = 200
        planTimer.restart()
    }

    Timer {
        interval: 800
        running: root.maxProbe
        repeat: false
        onTriggered: {
            console.log("probe: max before=" + probe.windowInfo())
            console.log("probe: maximize via qt=" + probe.maximizeWindow(true))
            maxTimer.start()
        }
    }

    Timer {
        id: maxTimer
        interval: 900
        repeat: false
        onTriggered: {
            console.log("probe: max after=" + probe.windowInfo())
            Qt.quit()
        }
    }

    Timer {
        id: dwmTimer
        interval: 20
        repeat: true
        onTriggered: dwmTick()
    }

    // ------------------------------------------------------------------
    // 6. Snap Layouts: hover the maximize button, then click a zone

    function startSnapLayouts() {
        const current = readInfo()
        const point = JSON.parse(probe.chromeRegions()).max
        probe.setForeground()
        probe.moveCursor(current.client_origin[0] + 400, current.client_origin[1] + 400)
        schedule("snap:baseline", 250, function() {
            probe.flyoutBegin(point[0] - 800, point[1] - 30, 1000, 650)
        })
        schedule("snap:hover", 60, function() {
            probe.moveCursor(point[0], point[1])
        })
        schedule("snap:detect", 1500, function() {
            const report = JSON.parse(probe.flyoutReport())
            const bbox = report.bbox || [0, 0, 0, 0]
            check("snap:flyout-appeared", report.fraction >= 0.02,
                  "changed=" + report.changed + " fraction=" + report.fraction)
            check("snap:flyout-size", (bbox[2] - bbox[0]) >= 200 && (bbox[3] - bbox[1]) >= 100,
                  "bbox=" + bbox)
            check("snap:hit-max-button", probe.hitTest(point[0], point[1]) === 9)
            const currentInfo = readInfo()
            const r = currentInfo.window_rect
            probe.capture(shotsDir + "/snap-flyout.png",
                          r[0] - 60, r[1] - 30,
                          r[2] - r[0] + 120, r[3] - r[1] + 120)
            // The first layout tile is two columns; its left zone sits this
            // far from the maximize button at 150%. Measured from the capture.
            probe.click(point[0] - 218, point[1] + 86)
        })
        schedule("snap:check", 1000, function() {
            const currentInfo = readInfo()
            const work = currentInfo.work_area
            const snapped = currentInfo.window_rect
            const half = Math.round((work[2] - work[0]) / 2)
            check("snap:window-left-half",
                  Math.abs(snapped[0] - work[0]) <= 2
                  && Math.abs(snapped[1] - work[1]) <= 2
                  && Math.abs((snapped[2] - snapped[0]) - half) <= 2
                  && Math.abs((snapped[3] - snapped[1]) - (work[3] - work[1])) <= 2,
                  JSON.stringify(snapped) + " work " + JSON.stringify(work))
            check("snap:not-zoomed", currentInfo.zoomed === false)
            probe.captureWindow(shotsDir + "/snap-left-half.png")
            schedule("materials", 200, startMaterialMatrix)
        })
    }

    // ------------------------------------------------------------------
    // 7. Material read-back matrix and fallback

    function startMaterialMatrix() {
        schedule("material:acrylic", 0, function() {
            probe.applyMaterial("acrylic", true)
        })
        schedule("material:acrylic-check", 450, function() {
            const current = readInfo()
            check("material:acrylic-accepted", current.backdrop.read_backdrop === 3,
                  "read=" + current.backdrop.read_backdrop)
            probe.captureWindow(shotsDir + "/acrylic-dark.png")
        })
        schedule("material:none", 0, function() {
            root.material = "none"
            probe.applyMaterial("none", true)
        })
        schedule("material:none-check", 450, function() {
            const current = readInfo()
            check("material:none-accepted", current.backdrop.read_backdrop === 1,
                  "read=" + current.backdrop.read_backdrop)
            check("material:none-not-blurred", current.backdrop.blur_behind === false)
            probe.captureWindow(shotsDir + "/none-dark.png")
        })
        schedule("material:restore", 0, function() {
            root.material = "mica"
            probe.applyMaterial("mica", true)
        })
        schedule("final", 400, function() {})
    }

    // ------------------------------------------------------------------
    // Gate 4: tear a tab out into a second window, then close it

    function tearMetrics() {
        return JSON.parse(tearProbe.metricsJson())
    }

    function memorySample(cycle, label) {
        const samples = tearMetrics().memory
        for (let index = samples.length - 1; index >= 0; --index) {
            if (samples[index].cycle === cycle && samples[index].label === label)
                return samples[index]
        }
        return null
    }

    function cycleLatency(cycle) {
        const cycles = tearMetrics().cycles
        for (let index = 0; index < cycles.length; ++index) {
            if (cycles[index].label === cycle)
                return cycles[index].latency_ms
        }
        return null
    }

    function startGate4() {
        startedAt = Date.now()
        running = true
        material = "mica"
        probe.applyMaterial(material, true)
        // Allocate the PSS query buffers once, before the baseline, so the
        // sampler itself does not show up as a window cost.
        tearProbe.sampleMemory("warmup", "warmup")
        statusLine = "listing " + datasetPath
        dirModel.start(datasetPath, "gate4-" + Date.now())
    }

    function gate4Begin() {
        if (dirModel.loaded === 0) {
            check("listing:loaded", false, dirModel.status)
            finishGate4()
            return
        }
        check("listing:loaded", dirModel.loaded > 0, dirModel.status)
        note("listing", dirModel.loaded + " rows / " + dirModel.batches
             + " batches / " + dirModel.load_ms.toFixed(1) + " ms")
        plan = []
        planIndex = 0
        schedule("baseline", settleMs, function() {
            tearProbe.sampleMemory("baseline", "baseline")
        })
        schedule("cycles", 0, function() { scheduleCycle(1) })
        planTimer.interval = 1
        planTimer.start()
    }

    function scheduleCycle(number) {
        if (number > cycles) {
            schedule("finish", 200, finishGate4)
            return
        }
        const cycle = String(number)
        schedule("cycle" + cycle + ":baseline", settleMs, function() {
            tearProbe.sampleMemory(cycle, "baseline")
            schedule("cycle" + cycle + ":create", 0, function() { runTearOffCycle(number) })
        })
    }

    function runTearOffCycle(number) {
        const cycle = String(number)
        const token = "dae-tearoff-" + cycle + "-" + Date.now()
        frameCycle = -1
        secondaryClosing = false

        tearProbe.beginCycle(cycle)
        tearProbe.mark("request")
        try {
            secondaryWindow = tearOffComponent.createObject(null, {
                "model": dirModel,
                "token": token,
                "dark": root.dark,
                "material": root.material
            })
        } catch (error) {
            check("cycle" + cycle + ":created", false, String(error))
            finishGate4()
            return
        }
        tearProbe.mark("created")
        check("cycle" + cycle + ":created", secondaryWindow !== null)
        check("cycle" + cycle + ":shared-model", secondaryWindow.model === dirModel,
              "the torn-off view binds the source model, not a copy")
        check("cycle" + cycle + ":attach", tearProbe.attachSecondary(token), "token=" + token)
        tearProbe.applySecondaryMaterial("mica", root.dark)
        tearProbe.clearSecondaryStats()

        // The first presented frame is the latency this gate is about. The
        // handler is installed before `visible` so the show cannot race it.
        secondaryWindow.frameSwapped.connect(function() {
            if (root.frameCycle === number)
                return
            root.frameCycle = number
            tearProbe.mark("frame")
        })
        secondaryWindow.closing.connect(function() { root.secondaryClosing = true })

        secondaryWindow.visible = true
        tearProbe.mark("shown")
        schedule("cycle" + cycle + ":open-check", settleMs, function() {
            checkTearOffOpen(number)
        })
    }

    function checkTearOffOpen(number) {
        const cycle = String(number)
        tearProbe.mark("settled")
        tearProbe.sampleMemory(cycle, "open")

        const info = JSON.parse(tearProbe.secondaryInfo())
        const primary = readInfo()
        const backdrop = info.backdrop || {}
        const regions = JSON.parse(tearProbe.secondaryRegions())
        const stats = JSON.parse(tearProbe.secondaryStats())
        const latency = cycleLatency(cycle)
        const baseline = memorySample(cycle, "baseline")
        const open = memorySample(cycle, "open")
        const increment = (baseline && open) ? open.pss_mb - baseline.pss_mb : NaN

        const style = info.style !== undefined ? Number("0x" + info.style) : 0
        check("cycle" + cycle + ":hwnd", info.hwnd !== undefined && info.hwnd !== "0"
              && info.hwnd !== primary.hwnd, info.hwnd + " vs primary " + primary.hwnd)
        check("cycle" + cycle + ":visible", info.visible === true)
        check("cycle" + cycle + ":native-style",
              (style & 0x00040000) !== 0 && (style & 0x00010000) !== 0,
              "style=0x" + info.style)
        check("cycle" + cycle + ":client-is-window",
              info.client_rect !== undefined && info.window_rect !== undefined
              && info.client_rect[2] === info.window_rect[2] - info.window_rect[0]
              && info.client_rect[3] === info.window_rect[3] - info.window_rect[1],
              "client " + info.client_rect + " window " + info.window_rect)
        check("cycle" + cycle + ":mica", backdrop.read_backdrop === 2
              && backdrop.read_dark === true,
              "read_backdrop=" + backdrop.read_backdrop + " dark=" + backdrop.read_dark)
        check("cycle" + cycle + ":transparent-path",
              backdrop.layered === true && backdrop.blur_behind === true,
              "layered=" + backdrop.layered + " blur=" + backdrop.blur_behind)
        check("cycle" + cycle + ":hit-caption",
              tearProbe.secondaryHitTest(regions.caption[0], regions.caption[1]) === 2)
        check("cycle" + cycle + ":hit-close",
              tearProbe.secondaryHitTest(regions.close[0], regions.close[1]) === 1)
        check("cycle" + cycle + ":hit-max",
              tearProbe.secondaryHitTest(regions.max[0], regions.max[1]) === 9)
        check("cycle" + cycle + ":hit-content",
              tearProbe.secondaryHitTest(regions.content[0], regions.content[1]) === 1)
        check("cycle" + cycle + ":hit-border-left",
              tearProbe.secondaryHitTest(regions.border_left[0], regions.border_left[1]) === 10)
        check("cycle" + cycle + ":frames", stats.frames >= 1, "frames=" + stats.frames)
        check("cycle" + cycle + ":latency", latency !== null && latency <= 150,
              "tear-off " + latency + " ms")
        check("cycle" + cycle + ":pss-increment", increment <= 20,
              baseline.pss_mb + " -> " + open.pss_mb + " = +" + increment.toFixed(2)
              + " MB (baseline " + baseline.pss_mb + ")")

        tearOffLatencies.push(latency)
        tearOffOpenIncrements.push(increment)
        tearOffCycles.push({
            cycle: cycle,
            latency_ms: latency,
            pss_baseline_mb: baseline.pss_mb,
            pss_open_mb: open.pss_mb,
            pss_increment_mb: Math.round(increment * 100) / 100,
            marks: tearMetrics().cycles[tearMetrics().cycles.length - 1].marks,
            window_rect: info.window_rect,
            client_rect: info.client_rect,
            backdrop: backdrop
        })

        tearProbe.captureSecondaryWindow(shotsDir + "/gate4-window-" + cycle + ".png")
        schedule("cycle" + cycle + ":drag", 150, function() { startSecondaryDrag(number) })
    }

    function startSecondaryDrag(number) {
        const cycle = String(number)
        const info = JSON.parse(tearProbe.secondaryInfo())
        secondaryRectBeforeDrag = info.window_rect
        tearProbe.clearSecondaryStats()
        check("cycle" + cycle + ":foreground", tearProbe.secondarySetForeground())
        const point = JSON.parse(tearProbe.secondaryRegions()).caption
        // Windows sometimes spends the first click on activation after a
        // window is shown; gate 3 waited 800 ms, this waits long enough for the
        // activation to settle while still inside the measurement's tail.
        schedule("cycle" + cycle + ":drag-down", 250, function() {
            tearProbe.secondarySetForeground()
            tearProbe.moveCursor(point[0], point[1])
        })
        schedule("cycle" + cycle + ":drag-press", 150, function() {
            check("cycle" + cycle + ":drag-mouse-down", tearProbe.secondaryMouseDown())
        })
        for (let step = 1; step <= 8; ++step) {
            schedule("cycle" + cycle + ":drag-move" + step, 18, function() {
                tearProbe.moveCursor(point[0] + 60 * step / 8, point[1] + 40 * step / 8)
            })
        }
        schedule("cycle" + cycle + ":drag-up", 80, function() { tearProbe.mouseUp() })
        schedule("cycle" + cycle + ":drag-check", 350, function() { checkSecondaryDrag(number) })
    }

    function checkSecondaryDrag(number) {
        const cycle = String(number)
        const info = JSON.parse(tearProbe.secondaryInfo())
        const after = info.window_rect
        const movedX = after[0] - secondaryRectBeforeDrag[0]
        const movedY = after[1] - secondaryRectBeforeDrag[1]
        const stats = JSON.parse(tearProbe.secondaryStats())
        check("cycle" + cycle + ":drag-moved",
              Math.abs(movedX - 60) <= 3 && Math.abs(movedY - 40) <= 3,
              "moved " + movedX + "," + movedY + " from " + secondaryRectBeforeDrag
              + " to " + after + " cursor " + probe.cursorPosition())
        check("cycle" + cycle + ":drag-hit-caption", count(stats.hit_test, "2") >= 1)
        check("cycle" + cycle + ":drag-nc-down-caption", count(stats.nc_down, "2") >= 1,
              "nc_down=" + JSON.stringify(stats.nc_down))
        check("cycle" + cycle + ":drag-native-loop",
              stats.entersizemove >= 1 && stats.exitsizemove >= 1,
              "enter=" + stats.entersizemove + " exit=" + stats.exitsizemove)
        schedule("cycle" + cycle + ":close", 200, function() { closeTearOff(number) })
    }

    function closeTearOff(number) {
        const cycle = String(number)
        check("cycle" + cycle + ":close-posted", tearProbe.closeSecondary())
        schedule("cycle" + cycle + ":closed", 600, function() {
            check("cycle" + cycle + ":closed-hidden",
                  secondaryWindow !== null && secondaryWindow.visible === false,
                  "visible=" + (secondaryWindow ? secondaryWindow.visible : "gone")
                  + " closing=" + secondaryClosing)
            if (secondaryWindow !== null)
                secondaryWindow.destroy()
            secondaryWindow = null
            tearProbe.forgetSecondary()
            schedule("cycle" + cycle + ":closed-memory", settleMs, function() {
                checkTearOffClosed(number)
            })
        })
    }

    function checkTearOffClosed(number) {
        const cycle = String(number)
        tearProbe.sampleMemory(cycle, "closed")
        const baseline = memorySample(cycle, "baseline")
        const closed = memorySample(cycle, "closed")
        const residual = closed.pss_mb - baseline.pss_mb
        const privateResidual = closed.private_mb - baseline.private_mb
        tearOffCloseResiduals.push(residual)
        tearOffClosePrivateResiduals.push(privateResidual)
        check("cycle" + cycle + ":close-residual", residual <= 20,
              baseline.pss_mb + " -> " + closed.pss_mb + " = " + residual.toFixed(2) + " MB")
        // Private commit is recorded, not asserted per cycle: the first extra
        // window commits the render loop's one-time caches (~25 MB) and keeps
        // them. The no-leak check is the growth across cycles in finishGate4.
        check("cycle" + cycle + ":primary-alive", readInfo().hwnd !== "0")
        const entry = tearOffCycles[tearOffCycles.length - 1]
        entry.pss_closed_mb = closed.pss_mb
        entry.pss_residual_mb = Math.round(residual * 100) / 100
        entry.private_closed_mb = closed.private_mb
        entry.private_residual_mb = Math.round(privateResidual * 100) / 100

        if (number < cycles)
            scheduleCycle(number + 1)
        else
            schedule("finish", 200, finishGate4)
    }

    function finishGate4() {
        if (finished)
            return
        finished = true
        running = false
        const current = readInfo()
        const maxLatency = tearOffLatencies.length > 0 ? Math.max.apply(null, tearOffLatencies) : 0
        const maxIncrement = tearOffOpenIncrements.length > 0
            ? Math.max.apply(null, tearOffOpenIncrements) : 0
        const maxResidual = tearOffCloseResiduals.length > 0
            ? Math.max.apply(null, tearOffCloseResiduals) : 0

        check("gate:latency", maxLatency > 0 && maxLatency <= 150,
              "worst tear-off " + maxLatency.toFixed(2) + " ms")
        check("gate:pss", maxIncrement <= 20,
              "worst increment +" + maxIncrement.toFixed(2) + " MB")
        check("gate:close", maxResidual <= 20,
              "worst close residual " + maxResidual.toFixed(2) + " MB")
        // The first close keeps the render loop's one-time caches; the check
        // that matters is that a second and third open/close do not grow the
        // process. Private commit is the leak signal; PSS also counts shared
        // pages and drifts by a page-cache-sized amount.
        const coldClose = tearOffCycles.length > 0 ? tearOffCycles[0].pss_closed_mb : 0
        const lastClose = tearOffCycles.length > 1
            ? tearOffCycles[tearOffCycles.length - 1].pss_closed_mb : coldClose
        const closedGrowth = lastClose - coldClose
        const coldClosePrivate = tearOffCycles.length > 0
            ? tearOffCycles[0].private_closed_mb : 0
        const lastClosePrivate = tearOffCycles.length > 1
            ? tearOffCycles[tearOffCycles.length - 1].private_closed_mb : coldClosePrivate
        const privateGrowth = lastClosePrivate - coldClosePrivate
        check("gate:close-private-no-growth", tearOffCycles.length < 2 || privateGrowth <= 5,
              "private " + coldClosePrivate + " -> " + lastClosePrivate + " MB ("
              + privateGrowth.toFixed(2) + ")")
        note("tear-off:pss-close-growth", "closed PSS " + coldClose + " -> " + lastClose
             + " MB (" + closedGrowth.toFixed(2) + "); private commit "
             + coldClosePrivate + " -> " + lastClosePrivate + " MB ("
             + privateGrowth.toFixed(2) + ")");
        note("tear-off:phases", tearOffCycles.map(function(entry) {
            return entry.cycle + ": created " + entry.marks.created + " ms, shown "
                + entry.marks.shown + " ms, frame " + entry.marks.frame + " ms"
        }).join(" | "))

        const metrics = tearMetrics()
        const body = {
            gate: 4,
            title: "tear-off / close increment of a second QQuickWindow",
            qt_version: current.qt_version,
            os: current.os,
            dpr: current.dpr,
            screen: [Screen.width * Screen.devicePixelRatio,
                     Screen.height * Screen.devicePixelRatio],
            input: {
                path: datasetPath,
                rows: dirModel.loaded,
                batches: dirModel.batches,
                load_ms: dirModel.load_ms
            },
            thresholds: { tear_off_ms: 150, pss_mb: 20 },
            summary: {
                cycles: tearOffCycles.length,
                latency_ms: tearOffLatencies,
                max_latency_ms: Math.round(maxLatency * 100) / 100,
                pss_increment_mb: tearOffOpenIncrements.map(function(value) {
                    return Math.round(value * 100) / 100
                }),
                max_pss_increment_mb: Math.round(maxIncrement * 100) / 100,
                pss_close_residual_mb: tearOffCloseResiduals.map(function(value) {
                    return Math.round(value * 100) / 100
                }),
                max_pss_close_residual_mb: Math.round(maxResidual * 100) / 100,
                closed_pss_growth_mb: Math.round(closedGrowth * 100) / 100,
                private_close_residual_mb: tearOffClosePrivateResiduals.map(function(value) {
                    return Math.round(value * 100) / 100
                }),
                closed_private_growth_mb: Math.round(privateGrowth * 100) / 100
            },
            cycles: tearOffCycles,
            timings: metrics.cycles,
            memory: metrics.memory,
            checks: checks,
            notes: notes,
            failures: failures,
            check_count: checks.length,
            primary_window: current
        }
        tearProbe.writeResults(outputPath, JSON.stringify(body, null, 2))
        const summary = "SUMMARY failures=" + failures + " checks=" + checks.length
        console.log(summary)
        tearProbe.appendLog(outputPath + ".log", summary)
        Qt.exit(failures === 0 ? 0 : 1)
    }

    function finishRun() {
        if (gate4) {
            finishGate4()
            return
        }
        if (!running)
            return
        running = false
        const current = readInfo()
        const body = {
            gate: 3,
            title: "frameless QQuickWindow + Mica/Acrylic + custom title bar",
            qt_version: current.qt_version,
            os: current.os,
            dpr: current.dpr,
            screen: [Screen.width * Screen.devicePixelRatio,
                     Screen.height * Screen.devicePixelRatio],
            material_decision: materialInfo,
            checks: checks,
            notes: notes,
            failures: failures,
            check_count: checks.length,
            samples: samples,
            dwm: dwmSummary,
            initial_window: initialWindow,
            window: current
        }
        probe.writeResults(outputPath, JSON.stringify(body, null, 2))
        const summary = "SUMMARY failures=" + failures + " checks=" + checks.length
        console.log(summary)
        logLine(summary)
        Qt.exit(failures === 0 ? 0 : 1)
    }

    WindowProbe { id: probe }
    TearOffProbe { id: tearProbe }
    DirectoryListModel { id: dirModel }

    // Preloaded, so the measured tear-off is window creation and first frame,
    // not QML file I/O — the shell has its view components loaded already.
    Component {
        id: tearOffComponent
        TearOffWindow {}
    }

    Connections {
        target: dirModel

        function onFinishedChanged() {
            if (!root.gate4 || !dirModel.finished)
                return
            root.gate4Begin()
        }
    }

    Component.onCompleted: {
        if (!probe.attach()) {
            statusLine = "attach failed"
            console.log("probe: attach failed")
            return
        }
        material = probe.detectMaterial()
        probe.applyMaterial(material, dark)
        probe.setForeground()
        refresh()
        console.log("probe: material=" + material + " info=" + probe.windowInfo())
        if (autoRun)
            startRun()
        else if (gate4)
            startGate4()
    }

    Timer {
        id: planTimer
        interval: 20
        repeat: false
        onTriggered: pump()
    }

    // Diagnostic: does an idle translucent window present frames on its own,
    // does a trivial invokable cause one, or does the screen capture?
    Timer {
        id: idleTimer
        interval: 20
        repeat: true
        onTriggered: idleTick()
    }

    Timer {
        interval: 700
        running: root.idleProbe
        repeat: false
        onTriggered: {
            const current = JSON.parse(probe.windowInfo())
            const r = current.window_rect
            probe.beginSamples(r[0] + 450, r[1],
                               (r[2] - r[0]) - 900, current.titlebar_px)
            probe.clearStats()
            root.idleState = { tick: 0, framesStart: 0, framesIdle: 0,
                               framesInvokable: 0 }
            root.idleState.framesStart = JSON.parse(probe.chromeStats()).frames
            idleTimer.start()
        }
    }

    function idleTick() {
        idleState.tick += 1
        if (idleState.tick === 10) {
            idleState.framesIdle = JSON.parse(probe.chromeStats()).frames
        } else if (idleState.tick > 10 && idleState.tick <= 20) {
            probe.cursorPosition()
        } else if (idleState.tick === 21) {
            idleState.framesInvokable = JSON.parse(probe.chromeStats()).frames
        } else if (idleState.tick > 21 && idleState.tick <= 30) {
            probe.sample("idle", "")
        } else if (idleState.tick > 30) {
            idleTimer.stop()
            const framesEnd = JSON.parse(probe.chromeStats()).frames
            console.log("probe: idle frames start=" + idleState.framesStart
                        + " after10idle=" + idleState.framesIdle
                        + " after10invokable=" + idleState.framesInvokable
                        + " after9samples=" + framesEnd)
            Qt.quit()
        }
    }

    // ---- diagnostic modes keep the first two spikes' habit -------------

    Timer {
        id: probeTimer
        interval: 700
        running: root.probeOnly || root.flyoutProbe || root.snapProbe
        repeat: false
        onTriggered: {
            root.refresh()
            if (root.flyoutProbe || root.snapProbe) {
                const regions = JSON.parse(probe.chromeRegions())
                probe.moveCursor(regions.max[0], regions.max[1])
                console.log("probe: hovering max at " + regions.max
                            + " hit=" + probe.hitTest(regions.max[0], regions.max[1]))
                hoverTimer.start()
                return
            }
            probe.captureWindow(root.shotsDir + "/probe-window.png")
            console.log("probe: window=" + probe.windowInfo())
            console.log("probe: regions=" + probe.chromeRegions())
            Qt.quit()
        }
    }

    Timer {
        id: hoverTimer
        interval: 1600
        repeat: false
        onTriggered: {
            const currentInfo = JSON.parse(probe.windowInfo())
            const r = currentInfo.window_rect
            probe.capture(root.shotsDir + "/flyout-open.png",
                          r[0] - 80, r[1] - 80,
                          r[2] - r[0] + 160, r[3] - r[1] + 160)
            if (root.snapProbe) {
                const regions = JSON.parse(probe.chromeRegions())
                const target = [regions.max[0] - 218, regions.max[1] + 86]
                console.log("probe: clicking snap zone at " + target)
                probe.click(target[0], target[1])
                snapTimer.start()
                return
            }
            console.log("probe: after hover windows=" + probe.topWindows())
            console.log("probe: stats=" + probe.chromeStats())
            Qt.quit()
        }
    }

    Timer {
        id: snapTimer
        interval: 1200
        repeat: false
        onTriggered: {
            const currentInfo = JSON.parse(probe.windowInfo())
            const r = currentInfo.window_rect
            probe.capture(root.shotsDir + "/snap-after.png",
                          r[0] - 20, r[1] - 20,
                          r[2] - r[0] + 40, r[3] - r[1] + 40)
            console.log("probe: after snap=" + probe.windowInfo())
            Qt.quit()
        }
    }

    // ---- chrome -------------------------------------------------------

    Rectangle {
        id: titleBar
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 40
        color: root.chromeColor

        Rectangle {
            id: logo
            x: 14
            anchors.verticalCenter: parent.verticalCenter
            width: 20
            height: 20
            radius: 6
            color: root.dark ? "#6ea8fe" : "#2f6fdf"

            Text {
                anchors.centerIn: parent
                text: "D"
                color: "#ffffff"
                font.pixelSize: 12
                font.weight: Font.DemiBold
            }
        }

        Text {
            anchors.left: logo.right
            anchors.leftMargin: 10
            anchors.verticalCenter: parent.verticalCenter
            text: "dae"
            color: root.textColor
            font.pixelSize: 13
            font.weight: Font.DemiBold
        }

        Text {
            anchors.left: logo.right
            anchors.leftMargin: 48
            anchors.verticalCenter: parent.verticalCenter
            text: root.gate4 ? "gate 4 · tear-off / close"
                             : "gate 3 · frameless + mica + custom chrome"
            color: root.mutedColor
            font.pixelSize: 12
        }

        // These three buttons are native non-client zones: WM_NCHITTEST maps
        // them to HTMINBUTTON / HTMAXBUTTON / HTCLOSE, so no MouseArea here
        // could receive a click. They are drawn to the same 46 dp columns the
        // native side assumes. Hover feedback would need WM_NCMOUSEMOVE, which
        // this spike does not yet forward to QML.
        Item {
            id: windowButtons
            anchors.right: parent.right
            height: parent.height
            width: 46 * 3

            Repeater {
                model: 3

                Item {
                    required property int index
                    width: 46
                    height: 40
                    x: index * 46

                    readonly property bool isClose: index === 2
                    readonly property bool isMaximize: index === 1

                    Rectangle {
                        anchors.fill: parent
                        color: isClose ? "#c42b1c" : "transparent"
                    }

                    Text {
                        anchors.centerIn: parent
                        text: isClose ? "\uE8BB"
                              : isMaximize ? (root.maximized ? "\uE923" : "\uE922")
                              : "\uE921"
                        color: isClose ? "#ffffff" : root.textColor
                        font.family: "Segoe Fluent Icons"
                        font.pixelSize: 10
                    }
                }
            }
        }
    }

    Rectangle {
        id: sidebar
        anchors.left: parent.left
        anchors.top: titleBar.bottom
        anchors.bottom: parent.bottom
        width: 216
        color: root.chromeColor

        Column {
            x: 16
            y: 18
            spacing: 6

            Text {
                text: "材料"
                color: root.mutedColor
                font.pixelSize: 11
                font.weight: Font.DemiBold
            }

            Repeater {
                model: ["mica", "acrylic", "none"]
                delegate: Item {
                    id: materialChoice
                    required property string modelData
                    width: 184
                    height: 30
                    Rectangle {
                        anchors.fill: parent
                        radius: 7
                        color: root.material === materialChoice.modelData
                            ? (root.dark ? "#2b6f8fff" : "#2b2f6fdf")
                            : "transparent"
                    }
                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        text: materialChoice.modelData
                        color: root.textColor
                        font.pixelSize: 12
                    }
                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            root.material = materialChoice.modelData
                            probe.applyMaterial(root.material, root.dark)
                            root.refresh()
                        }
                    }
                }
            }

            Item { width: 1; height: 10 }

            Text {
                text: "主题"
                color: root.mutedColor
                font.pixelSize: 11
                font.weight: Font.DemiBold
            }

            Row {
                spacing: 8
                Repeater {
                    model: [false, true]
                    delegate: Item {
                        id: themeChoice
                        required property bool modelData
                        width: 88
                        height: 30
                        Rectangle {
                            anchors.fill: parent
                            radius: 7
                            color: root.dark === themeChoice.modelData
                                ? (root.dark ? "#2b6f8fff" : "#2b2f6fdf")
                                : (root.dark ? "#1affffff" : "#12000000")
                        }
                        Text {
                            anchors.centerIn: parent
                            text: themeChoice.modelData ? "深色" : "浅色"
                            color: root.textColor
                            font.pixelSize: 12
                        }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: {
                                root.dark = themeChoice.modelData
                                probe.applyMaterial(root.material, root.dark)
                                root.refresh()
                            }
                        }
                    }
                }
            }
        }
    }

    Rectangle {
        id: content
        anchors.left: sidebar.right
        anchors.right: parent.right
        anchors.top: titleBar.bottom
        anchors.bottom: parent.bottom
        color: root.contentColor

        Column {
            id: gate3Content
            x: 28
            y: 24
            spacing: 14
            visible: !root.gate4

            Text {
                text: "门槛 3"
                color: root.textColor
                font.pixelSize: 24
                font.weight: Font.DemiBold
            }

            Text {
                width: content.width - 56
                text: "无边框 + Mica + 自绘标题栏。拖动、缩放、贴靠与 DWM 主题切换都由原生消息路径完成；"
                    + "这个窗口的功能由 QML 脚本自动验证。"
                color: root.mutedColor
                font.pixelSize: 13
                wrapMode: Text.WordWrap
            }

            Rectangle {
                width: content.width - 56
                height: 118
                radius: 10
                color: root.dark ? "#111316" : "#f5f6f8"
                border.color: root.hairline

                Column {
                    x: 16
                    y: 12
                    spacing: 5
                    Text {
                        text: "状态"
                        color: root.mutedColor
                        font.pixelSize: 11
                        font.weight: Font.DemiBold
                    }
                    Text {
                        text: root.statusLine
                        color: root.textColor
                        font.family: "Cascadia Mono"
                        font.pixelSize: 12
                    }
                    Text {
                        text: root.info.zoomed !== undefined
                            ? ("window " + root.info.window_rect + " · client "
                               + root.info.client_rect + " · dpr " + root.info.dpr)
                            : ""
                        color: root.mutedColor
                        font.family: "Cascadia Mono"
                        font.pixelSize: 11
                    }
                    Text {
                        text: root.materialInfo.build !== undefined
                            ? ("build " + root.materialInfo.build + " · transparency "
                               + root.materialInfo.transparency + " · mica from "
                               + root.materialInfo.mica_min_build + " · acrylic from "
                               + root.materialInfo.acrylic_min_build)
                            : ""
                        color: root.mutedColor
                        font.family: "Cascadia Mono"
                        font.pixelSize: 11
                    }
                    Text {
                        text: root.checks.length > 0
                            ? ("checks " + root.checks.length + " · failures " + root.failures)
                            : ""
                        color: root.failures > 0 ? "#ff8a80" : root.mutedColor
                        font.family: "Cascadia Mono"
                        font.pixelSize: 11
                    }
                }
            }

            Text {
                text: "把光标停在最大化按钮上：Windows 11 的 Snap Layouts 应当出现——"
                    + "这也正是 WM_NCHITTEST 返回 HTMAXBUTTON 的实测含义。"
                width: content.width - 56
                color: root.mutedColor
                font.pixelSize: 12
                wrapMode: Text.WordWrap
            }
        }

        // Gate 4: the source tab. Its model is the same object the torn-off
        // window binds, so a tear-off adds a view, not a second copy of 120k
        // rows.
        ListView {
            id: sourceList
            anchors.fill: parent
            anchors.topMargin: 34
            visible: root.gate4
            model: dirModel
            reuseItems: true
            cacheBuffer: 2600
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            spacing: 0

            delegate: Item {
                id: sourceDelegate
                required property int index
                required property string fileName
                required property string sizeText
                required property string modifiedText
                required property bool isDir
                required property bool hidden

                width: ListView.view.width
                height: 26

                Rectangle {
                    anchors.fill: parent
                    color: (sourceDelegate.index % 2) === 0 ? root.contentColor
                                                          : (root.dark ? "#15181e" : "#f7f8fa")
                }

                Rectangle {
                    x: 12
                    y: 6
                    width: 14
                    height: 14
                    radius: 3
                    color: sourceDelegate.isDir ? "#5b8def" : "#3b4250"
                }

                Text {
                    x: 36
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.max(0, sourceDelegate.width - 36 - 340)
                    text: sourceDelegate.fileName
                    color: sourceDelegate.hidden ? "#6b7280" : root.textColor
                    font.pixelSize: 13
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                }

                Text {
                    anchors.right: sourceModified.left
                    anchors.rightMargin: 24
                    anchors.verticalCenter: parent.verticalCenter
                    width: 90
                    horizontalAlignment: Text.AlignRight
                    text: sourceDelegate.sizeText
                    color: root.mutedColor
                    font.pixelSize: 12
                    textFormat: Text.PlainText
                }

                Text {
                    id: sourceModified
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    width: 140
                    horizontalAlignment: Text.AlignRight
                    text: sourceDelegate.modifiedText
                    color: root.mutedColor
                    font.pixelSize: 12
                    textFormat: Text.PlainText
                }
            }
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 34
            visible: root.gate4
            color: root.chromeColor

            Text {
                x: 12
                anchors.verticalCenter: parent.verticalCenter
                text: "源窗口 · " + (dirModel.finished
                    ? dirModel.status
                    : (dirModel.loaded + " rows · reading…"))
                color: root.textColor
                font.family: "Cascadia Mono"
                font.pixelSize: 11
            }
        }
    }
}
