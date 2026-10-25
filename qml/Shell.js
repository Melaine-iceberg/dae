.pragma library

// token -> ShellWindow. A shared JS library is one instance per QQmlEngine, so
// every window in this process sees the same map. The map is how a tab dropped
// on another window finds the QML object that has to adopt it.
var windows = ({})

function register(token, window) {
    windows[token] = window
}

function unregister(token) {
    delete windows[token]
}

function windowFor(token) {
    return token ? (windows[token] || null) : null
}
