#!/usr/bin/env python3
"""Point aqtinstall at Qt 6.11+ Windows desktop packages.

Qt 6.11 moved the Windows desktop packages into per-architecture folders:

    online/qtsdkrepository/windows_x86/desktop/qt6_6120/qt6_6120_msvc2022_64/Updates.xml

aqtinstall 3.3.0 still derives `qt6_6120/qt6_6120/Updates.xml` from the
version alone, which is a 404, and `install-qt` fails with

    ERROR : Failed to locate XML data for Qt version '6.12.0'.

`ArchiveId.to_folder` already knows how to build the per-arch folder — it just
receives an empty extension, because `QtRepoProperty.extension_for_arch` only
answers for wasm and Android. Filling that answer in is a one-function patch,
so the rest of aqtinstall — module resolution, checksums, 7z extraction,
`--outputdir` layout — runs unmodified.

Linux is unaffected: `linux_x64/desktop/qt6_6120/qt6_6120/Updates.xml` still
exists, which is why `install-qt-action` keeps working there.

Delete this script and the workflow step that calls it once aqtinstall
supports the layout — the upstream work item is
https://github.com/miurahr/aqtinstall/issues/1048 .

Run it with the interpreter that will run `aqt`; it edits that
interpreter's site-packages.
"""

import inspect
import sys

try:
    import aqt.metadata as metadata
except ImportError:
    sys.exit("aqtinstall is not installed for this interpreter: python -m pip install aqtinstall==3.3.0")

path = inspect.getsourcefile(metadata)
with open(path, encoding="utf-8") as handle:
    source = handle.read()

if 'architecture.startswith("win64_msvc")' in source:
    print(f"aqt per-arch patch already applied: {path}")
    sys.exit(0)

# The tail of `extension_for_arch`: the Android branch is the last one before
# the empty answer, so the anchor is unique in the file.
anchor = (
    "            if ext in ArchiveId.EXTENSIONS_REQUIRED_ANDROID_QT6:\n"
    "                return ext\n"
    '        return ""'
)
patched = (
    "            if ext in ArchiveId.EXTENSIONS_REQUIRED_ANDROID_QT6:\n"
    "                return ext\n"
    '        if architecture.startswith("win64_msvc"):\n'
    "            # Qt 6.11+ desktop packages live in qt6_<ver>_<arch>/, where the\n"
    "            # arch part is the architecture without its `win64_` prefix.\n"
    '            return architecture[len("win64_") :]\n'
    '        return ""'
)

if source.count(anchor) != 1:
    sys.exit(
        "this aqtinstall build does not match the patch anchor; check whether it "
        "already supports the per-arch layout and drop this script if it does"
    )

with open(path, "w", encoding="utf-8") as handle:
    handle.write(source.replace(anchor, patched))

print(f"patched {path}")
