//! The Windows share sheet, reached from the file manager's context menu.
//!
//! Sharing is a Windows-only capability here. The entry point is the
//! `IDataTransferManagerInterop` that Raymond Chen's two-part post describes:
//! a desktop process has no `CoreWindow`, so the `ForWindow` variants are the
//! only way to show the sheet and to hand it data. The sheet's data request is
//! answered by a `TypedEventHandler` registered on the window's
//! `DataTransferManager`; the handler installs the selected files as storage
//! items.
//!
//! The storage items are resolved on a worker thread (an MTA) because
//! `IAsyncOperation::get()` blocks, and doing that on the UI thread's STA would
//! deadlock. They cross back as `AgileReference`s, which is what WinRT defines
//! for exactly this hand-off. The custom `IIterable` is built on the UI thread
//! and used by the handler there.
//!
//! The last registration is kept alive for the window: the sheet may fire the
//! handler after `ShowShareUIForWindow` returns, and the handler object must
//! outlive that call. A new share replaces the previous registration.

use cxx_qt_lib::QStringList;

/// Whether this build has a system share sheet.
pub fn supported() -> bool {
    cfg!(windows)
}

/// Opens the share sheet for `paths` on the window `handle`. Returns false
/// when the platform cannot share, the window handle is unknown, or WinRT
/// refused; the caller shows the same "sharing is unavailable" toast it uses
/// when the entry is hidden.
pub fn share_paths(paths: &QStringList, handle: f64) -> bool {
    #[cfg(windows)]
    {
        return windows_share::share(paths, handle);
    }
    #[cfg(not(windows))]
    {
        let _ = (paths, handle);
        false
    }
}

#[cfg(windows)]
mod windows_share {
    use std::cell::{Cell, RefCell};

    use cxx_qt_lib::QStringList;
    use windows::ApplicationModel::DataTransfer::{
        DataPackage, DataRequest, DataRequestedEventArgs, DataTransferManager,
    };
    use windows::core::{AgileReference, HSTRING, Interface, Ref, Result};
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::{IStorageItem, StorageFile};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED, RoInitialize};
    use windows::Win32::UI::Shell::IDataTransferManagerInterop;
    use windows_collections::{IIterable, IIterable_Impl, IIterator, IIterator_Impl};

    /// A resolved set of storage items, kept alive between the share request
    /// and the sheet's callback.
    struct Registration {
        manager: DataTransferManager,
        #[allow(dead_code)]
        handler: TypedEventHandler<DataTransferManager, DataRequestedEventArgs>,
        token: i64,
    }

    // The share sheet and its handler are apartment-bound, so the last
    // registration is thread-local rather than a process-wide static.
    thread_local! {
        static ACTIVE: RefCell<Option<Registration>> = const { RefCell::new(None) };
    }

    pub(super) fn share(paths: &QStringList, handle: f64) -> bool {
        if handle <= 0.0 || paths.is_empty() {
            return false;
        }
        let hwnd = HWND(handle as usize as *mut core::ffi::c_void);
        let paths: Vec<String> = paths.iter().map(|path| path.to_string()).collect();

        let items = match std::thread::spawn(move || resolve_items(&paths).map_err(|e| e.to_string())).join() {
            Ok(Ok(items)) => items,
            Ok(Err(error)) => {
                log::warn!("dae-share resolve: {error}");
                return false;
            }
            Err(_) => {
                log::warn!("dae-share resolve: thread panicked");
                return false;
            }
        };
        let resolved: Vec<IStorageItem> = items
            .iter()
            .filter_map(|item| item.resolve().ok())
            .collect();
        if resolved.is_empty() {
            log::warn!("dae-share resolve: no storage items");
            return false;
        }

        // Qt owns the UI thread's COM apartment; `RoInitialize` here either
        // joins it or reports that it is already initialized, and both are
        // fine for the interop calls below.
        unsafe {
            let _ = RoInitialize(RO_INIT_SINGLETHREADED);
        }

        let interop: IDataTransferManagerInterop = match windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>() {
            Ok(interop) => interop,
            Err(error) => {
                log::warn!("dae-share factory: {error}");
                return false;
            }
        };
        let manager: DataTransferManager = match unsafe { interop.GetForWindow(hwnd) } {
            Ok(manager) => manager,
            Err(error) => {
                log::warn!("dae-share GetForWindow: {error}");
                return false;
            }
        };

        let iterable: IIterable<IStorageItem> = StorageItems {
            items: resolved,
            index: Cell::new(0),
        }
        .into();

        // `TypedEventHandler::new` requires a `Send` closure, but the
        // iterable is a UI-apartment COM object that never leaves the thread
        // the handler runs on. The wrapper is the narrowest way to say that.
        struct SendIterable(IIterable<IStorageItem>);
        unsafe impl Send for SendIterable {}
        impl SendIterable {
            fn iterable(&self) -> &IIterable<IStorageItem> {
                &self.0
            }
        }

        let iterable = SendIterable(iterable);
        let handler = TypedEventHandler::new(
            move |_sender: Ref<DataTransferManager>,
                  args: Ref<DataRequestedEventArgs>| -> Result<()> {
                let args = args.ok()?;
                let request: DataRequest = args.Request()?;
                let data: DataPackage = request.Data()?;
                data.Properties()?.SetTitle(&HSTRING::from("dae"))?;
                data.SetStorageItems(iterable.iterable(), true)?;
                Ok(())
            },
        );

        let token = match manager.DataRequested(&handler) {
            Ok(token) => token,
            Err(error) => {
                log::warn!("dae-share DataRequested: {error}");
                return false;
            }
        };

        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(previous) = active.take() {
                let _ = previous.manager.RemoveDataRequested(previous.token);
            }
            *active = Some(Registration {
                manager,
                handler,
                token,
            });
        });

        match unsafe { interop.ShowShareUIForWindow(hwnd) } {
            Ok(()) => true,
            Err(error) => {
                log::warn!("dae-share ShowShareUIForWindow: {error}");
                false
            }
        }
    }

    /// Resolves each path to a `StorageFile` on an MTA thread and hands back
    /// agile references, because `IAsyncOperation::get()` blocks and the UI
    /// thread is an STA.
    fn resolve_items(paths: &[String]) -> Result<Vec<AgileReference<IStorageItem>>> {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let mut items = Vec::new();
        for path in paths {
            // The listing holds forward-slash paths; `StorageFile` takes a
            // Win32 path and rejects the mixed form with ERROR_BAD_PATHNAME.
            let native = path.replace('/', "\\");
            let operation = StorageFile::GetFileFromPathAsync(&HSTRING::from(native.as_str()))?;
            let file = operation.join()?;
            let item: IStorageItem = file.cast()?;
            items.push(AgileReference::new(&item)?);
        }
        Ok(items)
    }

    /// The iterable the share sheet walks. `First` returns a fresh cursor over
    /// the same items; `Current`/`MoveNext`/`GetMany` are the four methods
    /// `IIterator` defines.
    #[windows::core::implement(IIterable<IStorageItem>, IIterator<IStorageItem>)]
    struct StorageItems {
        items: Vec<IStorageItem>,
        index: Cell<usize>,
    }

    impl IIterable_Impl<IStorageItem> for StorageItems_Impl {
        fn First(&self) -> Result<IIterator<IStorageItem>> {
            Ok(StorageItems {
                items: self.items.clone(),
                index: Cell::new(0),
            }
            .into())
        }
    }

    impl IIterator_Impl<IStorageItem> for StorageItems_Impl {
        fn Current(&self) -> Result<IStorageItem> {
            self.items
                .get(self.index.get())
                .cloned()
                .ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_BOUNDS))
        }

        fn HasCurrent(&self) -> Result<bool> {
            Ok(self.index.get() < self.items.len())
        }

        fn MoveNext(&self) -> Result<bool> {
            let next = self.index.get().saturating_add(1);
            self.index.set(next);
            Ok(next < self.items.len())
        }

        fn GetMany(&self, items: &mut [Option<IStorageItem>]) -> Result<u32> {
            let mut written = 0;
            while written < items.len() && self.index.get() < self.items.len() {
                items[written] = self.items.get(self.index.get()).cloned();
                self.index.set(self.index.get() + 1);
                written += 1;
            }
            Ok(written as u32)
        }
    }
}
