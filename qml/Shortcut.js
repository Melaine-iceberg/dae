.pragma library

// Key-binding helpers shared by the shell, the explorer views and the settings
// panel. Bindings are stored in `settings.toml` in TanStack Hotkeys' canonical
// form (`Mod` means Cmd on macOS and Ctrl elsewhere); this file turns them into
// something `Qt.Key` events, `Shortcut.sequence` and a readable label can use.
//
// Qt reports keys by keycode, not by the `event.key` names a browser would
// use. `keyName` is the single mapping table, so recording a binding and
// matching an existing one cannot disagree about what "ArrowUp" or "Delete"
// means. Unknown keys return an empty string and are ignored while recording.

function keyName(keyCode) {
    if (keyCode >= Qt.Key_A && keyCode <= Qt.Key_Z)
        return String.fromCharCode(65 + keyCode - Qt.Key_A).toLowerCase()
    if (keyCode >= Qt.Key_0 && keyCode <= Qt.Key_9)
        return String.fromCharCode(48 + keyCode - Qt.Key_0)
    if (keyCode >= Qt.Key_F1 && keyCode <= Qt.Key_F35)
        return "f" + (keyCode - Qt.Key_F1 + 1)

    switch (keyCode) {
    case Qt.Key_Space: return "space"
    case Qt.Key_Tab: return "tab"
    case Qt.Key_Backtab: return "tab"
    case Qt.Key_Backspace: return "backspace"
    case Qt.Key_Return: return "enter"
    case Qt.Key_Enter: return "enter"
    case Qt.Key_Escape: return "escape"
    case Qt.Key_Delete: return "delete"
    case Qt.Key_Insert: return "insert"
    case Qt.Key_Home: return "home"
    case Qt.Key_End: return "end"
    case Qt.Key_PageUp: return "pageup"
    case Qt.Key_PageDown: return "pagedown"
    case Qt.Key_Left: return "left"
    case Qt.Key_Right: return "right"
    case Qt.Key_Up: return "up"
    case Qt.Key_Down: return "down"
    case Qt.Key_Comma: return ","
    case Qt.Key_Period: return "."
    case Qt.Key_Slash: return "/"
    case Qt.Key_Semicolon: return ";"
    case Qt.Key_Apostrophe: return "'"
    case Qt.Key_BracketLeft: return "["
    case Qt.Key_BracketRight: return "]"
    case Qt.Key_Backslash: return "\\"
    case Qt.Key_Minus: return "-"
    case Qt.Key_Equal: return "="
    case Qt.Key_QuoteLeft: return "`"
    default: return ""
    }
}

// The canonical ids in `default_shortcuts()` are unambiguous, but existing
// user edits or hand-written TOML may use Qt's spelling ("Up", "Del", "PgUp").
// Folding both spellings onto one name keeps `matches()` tolerant.
function canonicalKey(key) {
    var lowered = String(key).toLowerCase()
    switch (lowered) {
    case "up": case "arrowup": return "up"
    case "down": case "arrowdown": return "down"
    case "left": case "arrowleft": return "left"
    case "right": case "arrowright": return "right"
    case "pgup": case "pageup": return "pageup"
    case "pgdown": case "pagedown": return "pagedown"
    case "ins": case "insert": return "insert"
    case "del": case "delete": return "delete"
    case "esc": case "escape": return "escape"
    case "return": case "enter": return "enter"
    case " ": case "space": case "spacebar": return "space"
    case "ctrl": case "control": return "ctrl"
    default: return lowered
    }
}

function parse(binding) {
    if (!binding)
        return null
    var pieces = String(binding).split("+")
    if (pieces.length === 0)
        return null
    var key = canonicalKey(pieces.pop().trim())
    if (key === "")
        return null

    var result = { key: key, mod: false, ctrl: false, shift: false, alt: false, meta: false }
    for (var i = 0; i < pieces.length; ++i) {
        var piece = canonicalKey(pieces[i].trim())
        if (piece === "mod" || piece === "cmd" || piece === "command" || piece === "super")
            result.mod = true
        else if (piece === "ctrl")
            result.ctrl = true
        else if (piece === "shift")
            result.shift = true
        else if (piece === "alt" || piece === "option")
            result.alt = true
        else if (piece === "meta")
            result.meta = true
        else
            return null
    }
    return result
}

function matches(event, binding) {
    var parsed = parse(binding)
    if (!parsed)
        return false
    if (keyName(event.key) !== parsed.key)
        return false

    var ctrl = (event.modifiers & Qt.ControlModifier) !== 0
    var shift = (event.modifiers & Qt.ShiftModifier) !== 0
    var alt = (event.modifiers & Qt.AltModifier) !== 0
    var meta = (event.modifiers & Qt.MetaModifier) !== 0

    if (shift !== parsed.shift || alt !== parsed.alt)
        return false

    if (parsed.mod) {
        // Canonical `Mod` is Cmd on macOS and Ctrl elsewhere. Without the
        // platform here, accepting either keeps the in-view key handling
        // working on every OS; `Shortcut.sequence` still uses the exact one.
        if (!(ctrl || meta))
            return false
    } else {
        if (ctrl !== parsed.ctrl || meta !== parsed.meta)
            return false
    }
    return true
}

// Returns a canonical binding for a key press, or "" for a modifier-only press
// (which the capture dialog ignores). `isMac` comes from QML because the
// primary modifier differs by platform.
function record(event, isMac) {
    var key = keyName(event.key)
    if (key === "")
        return ""
    // Stored bindings follow the registry's spelling: letters and function
    // keys are uppercase, punctuation is left alone.
    if (key.length === 1 || (key.length > 1 && key.charAt(0) === "f"))
        key = key.toUpperCase()

    var modifiers = []
    if (event.modifiers & Qt.ControlModifier)
        modifiers.push(isMac ? "Ctrl" : "Mod")
    if (event.modifiers & Qt.MetaModifier)
        modifiers.push(isMac ? "Mod" : "Meta")
    if (event.modifiers & Qt.AltModifier)
        modifiers.push("Alt")
    if (event.modifiers & Qt.ShiftModifier)
        modifiers.push("Shift")
    modifiers.push(key)
    return modifiers.join("+")
}

function display(binding, isMac) {
    if (!binding)
        return ""
    var parsed = parse(binding)
    if (!parsed)
        return binding

    var pieces = []
    if (parsed.mod)
        pieces.push(isMac ? "⌘" : "Ctrl")
    if (parsed.ctrl)
        pieces.push(isMac ? "⌃" : "Ctrl")
    if (parsed.alt)
        pieces.push(isMac ? "⌥" : "Alt")
    if (parsed.shift)
        pieces.push(isMac ? "⇧" : "Shift")
    if (parsed.meta)
        pieces.push("Meta")

    switch (parsed.key) {
    case "up": pieces.push("↑"); break
    case "down": pieces.push("↓"); break
    case "left": pieces.push("←"); break
    case "right": pieces.push("→"); break
    case "pageup": pieces.push("PgUp"); break
    case "pagedown": pieces.push("PgDn"); break
    case "delete": pieces.push("Del"); break
    case "insert": pieces.push("Ins"); break
    case "escape": pieces.push("Esc"); break
    case "enter": pieces.push("Enter"); break
    case "backspace": pieces.push("Backspace"); break
    case "space": pieces.push("Space"); break
    default: pieces.push(parsed.key.length === 1 ? parsed.key.toUpperCase() : parsed.key)
    }
    return pieces.join("+")
}

// `Shortcut.sequence` wants Qt's key spelling, not the canonical one. An empty
// binding yields "" so the Shortcut stays disabled.
function qtSequence(binding, isMac) {
    var parsed = parse(binding)
    if (!parsed)
        return ""

    var pieces = []
    if (parsed.mod)
        pieces.push(isMac ? "Meta" : "Ctrl")
    if (parsed.ctrl)
        pieces.push("Ctrl")
    if (parsed.alt)
        pieces.push("Alt")
    if (parsed.shift)
        pieces.push("Shift")
    if (parsed.meta)
        pieces.push("Meta")

    switch (parsed.key) {
    case "up": pieces.push("Up"); break
    case "down": pieces.push("Down"); break
    case "left": pieces.push("Left"); break
    case "right": pieces.push("Right"); break
    case "pageup": pieces.push("PgUp"); break
    case "pagedown": pieces.push("PgDn"); break
    case "delete": pieces.push("Del"); break
    case "insert": pieces.push("Ins"); break
    case "escape": pieces.push("Esc"); break
    case "enter": pieces.push("Return"); break
    case "backspace": pieces.push("Backspace"); break
    case "space": pieces.push("Space"); break
    case "tab": pieces.push("Tab"); break
    default:
        if (parsed.key.length > 1 && parsed.key.charAt(0) === "f")
            pieces.push(parsed.key.toUpperCase())
        else if (parsed.key.length === 1)
            // Qt's sequence parser only knows key names, so a letter has to be
            // "K", not the canonical lowercase "k".
            pieces.push(parsed.key.toUpperCase())
        else
            pieces.push(parsed.key)
    }
    return pieces.join("+")
}

function bindingsEqual(first, second) {
    var a = parse(first)
    var b = parse(second)
    if (!a || !b)
        return first === second
    return a.key === b.key && a.mod === b.mod && a.ctrl === b.ctrl
            && a.shift === b.shift && a.alt === b.alt && a.meta === b.meta
}
