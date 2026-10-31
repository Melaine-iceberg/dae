// The reference that keeps the precompiled QML units in the binary.
//
// `cxx-qt-build` compiles every C++ file into one static archive, and a static
// archive only gives up an object when something references a symbol defined in
// it. The generated loader (`<OUT_DIR>/qmlcache/dae_qmlcache_loader.cpp`)
// registers the units from a CRT constructor and is referenced by nobody, so
// without this file the linker drops that object — and, with it, every unit
// object whose `qmlData` it names. The engine then silently compiles the `.qml`
// sources at run time again: the full first-launch cost comes back and the user
// cache is refilled with 53 `.qmlc` files.
//
// `main.rs` calls `dae_qml_cache_init()` for exactly that reason. It is always
// declared and always callable; the symbol it pulls in only exists in a build
// that generated the units, which is what `DAE_QML_AOT` marks.
#include <QtCore/qglobal.h>

#ifdef DAE_QML_AOT
// Declared only to create the reference; the generated loader defines it.
int QT_MANGLE_NAMESPACE(qInitResources_dae_qmlcache)();
#endif

extern "C" void dae_qml_cache_init()
{
#ifdef DAE_QML_AOT
    QT_MANGLE_NAMESPACE(qInitResources_dae_qmlcache)();
#endif
}
