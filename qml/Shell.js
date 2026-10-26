.pragma library

// token -> ShellWindow. A shared JS library is one instance per QQmlEngine, so
// every window in this process sees the same map. The map is how a tab dropped
// on another window finds the QML object that has to adopt it.
var windows = ({})

// The window that was last active. An external activation goes there first so
// a torn-off window, not always the original one, receives the new tab.
var lastActiveToken = ""

function register(token, window) {
    windows[token] = window
}

function unregister(token) {
    delete windows[token]
    if (lastActiveToken === token)
        lastActiveToken = ""
}

function windowFor(token) {
    return token ? (windows[token] || null) : null
}

function noteActive(token, active) {
    if (active && token)
        lastActiveToken = token
}

// The window that should handle an external activation: the last active one,
// then the main window if it is still alive, then any remaining window.
function activationTarget() {
    if (lastActiveToken && windows[lastActiveToken])
        return windows[lastActiveToken]
    if (windows["dae-main"])
        return windows["dae-main"]
    for (var token in windows) {
        if (Object.prototype.hasOwnProperty.call(windows, token))
            return windows[token]
    }
    return null
}

function normalizePath(path) {
    if (!path)
        return ""
    var text = String(path).replace(/\\/g, "/")
    while (text.length > 1 && text.charAt(text.length - 1) === "/")
        text = text.substring(0, text.length - 1)
    // Local Windows and UNC paths are case-insensitive; POSIX paths are not.
    if (/^[a-zA-Z]:/.test(text) || text.indexOf("//") === 0)
        text = text.toLowerCase()
    return text
}

function isSamePath(a, b) {
    return normalizePath(a) === normalizePath(b)
}

// An external activation restores a tab that already shows the target instead
// of opening a copy of the same folder. Search every window, not only the one
// that happened to be active last.
function findOpenTab(path) {
    for (var token in windows) {
        if (!Object.prototype.hasOwnProperty.call(windows, token))
            continue
        var window = windows[token]
        var tabs = window.tabs || []
        for (var index = 0; index < tabs.length; index += 1) {
            if (!tabs[index].trash && isSamePath(tabs[index].listing.path, path))
                return { "token": token, "index": index }
        }
    }
    return null
}
