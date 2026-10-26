// Debug builds define QT_QML_DEBUG (see build.rs). Including this header with
// the define installs QQmlDebuggingEnabler from static initialization, which
// is what lets the QML preview service attach to the process. Release builds
// compile this file with the define absent, so it emits no enabler.

#include <QtQml/qqmldebug.h>
