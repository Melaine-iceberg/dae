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
//! * `qml-first-list` — that surface laid those rows out (`contentHeight`
//!   grew). The explorer list and the overview board both mark it.
//!
//! Both the overview and the explorer list observe their models, and the
//! non-visible one can publish rows too, so `mark` is once per stage: the
//! first view to reach the moment is the startup one. The C++ side does
//! nothing unless `DAE_STARTUP_PERF` is set, so an ordinary launch pays one
//! hash lookup per call at most.

use qtbridge::{QmlElement, qobject};

/// QML usage: `Startup.mark("qml-first-model")`.
///
/// Ported from CXX-Qt to Qt Bridge: the manual [`QmlElement`] impl keeps the
/// module and element name the QML tree already imports, and `NoQmlElement`
/// keeps the macro from registering a second, crate-named copy.
#[derive(Default)]
pub struct Startup;

#[qobject(NoQmlElement)]
impl Startup {
    #[qslot]
    fn mark(&mut self, stage: String) {
        crate::window::startup_mark_once(&stage);
    }
}

impl QmlElement for Startup {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "Startup";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = true;
}
