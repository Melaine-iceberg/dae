//! Startup timeline marks that only QML can see.
//!
//! `DAE_STARTUP_PERF=1` already prints the Rust side of the timeline
//! (`main`, `qgui-application`, `qml-engine`, `qml-loaded`, `first-frame`).
//! Everything between `qml-engine` and `qml-loaded` happens inside
//! `engine.load`, where Rust only knows when the call returns — not whether
//! the time went into instantiating the component tree or into the work its
//! `Component.onCompleted` starts. The views mark those moments themselves:
//!
//! * `qml-root-completed` — the root component's `onCompleted` began, i.e. the
//!   object tree exists and the startup work (tab creation, model refresh) is
//!   about to run.
//! * `qml-first-model` — the first surface's model published rows. Marked by
//!   the models (`model.rs`, `overview_model.rs`) rather than by QML: a QML
//!   handler runs after the view's own model connection, so it would report
//!   the layout pass as the data moment and invert the split.
//! * `qml-first-list` — that list laid those rows out (`contentHeight` grew).
//!
//! Both the overview and the explorer list observe their models, and the
//! non-visible one can publish rows too, so `mark` is once per stage: the
//! first view to reach the moment is the startup one. The C++ side does
//! nothing unless `DAE_STARTUP_PERF` is set, so an ordinary launch pays one
//! hash lookup per call at most.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        /// QML usage: `Startup.mark("qml-first-model")`.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type Startup = super::StartupRust;

        #[qinvokable]
        fn mark(self: Pin<&mut Startup>, stage: &QString);
    }
}

use std::pin::Pin;

use cxx_qt_lib::QString;

#[derive(Default)]
pub struct StartupRust;

impl qobject::Startup {
    fn mark(self: Pin<&mut Self>, stage: &QString) {
        crate::window::startup_mark_once(&stage.to_string());
    }
}
