// Copyright 2023-2023 CrabNebula Ltd.
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::os::windows::ffi::OsStrExt;
use std::{ffi::c_void, iter::once, path::Path};
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::{
    Graphics::{
        Gdi::{CreateBitmap, HBITMAP},
        Imaging::{
            CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapDecoder,
            IWICImagingFactory, WICConvertBitmapSource, WICDecodeMetadataCacheOnDemand,
        },
    },
    System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
};

use crate::Result;

/// A decoded image in premultiplied BGRA — the layout both the Shell's drag
/// image and `UpdateLayeredWindow` expect, so a single decode serves both.
pub(crate) struct PremultipliedBitmap {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) pixels: Vec<u8>,
}

impl PremultipliedBitmap {
    /// Wraps the pixels in a device-dependent bitmap, which is all the Shell's
    /// drag image takes.
    pub(crate) fn to_device_bitmap(&self) -> HBITMAP {
        unsafe {
            CreateBitmap(
                self.width,
                self.height,
                1,
                32,
                Some(self.pixels.as_ptr() as *const c_void),
            )
        }
    }
}

/// Decodes whichever image the caller supplied, whatever format it is in.
pub(crate) fn read_image(item: &crate::Image) -> Result<PremultipliedBitmap> {
    match item {
        crate::Image::Raw(bytes) => read_bytes_to_premultiplied_bgra(bytes),
        crate::Image::File(path) => read_path_to_premultiplied_bgra(path),
    }
}

pub(crate) fn read_bytes_to_premultiplied_bgra(bytes: &[u8]) -> Result<PremultipliedBitmap> {
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

        let stream = factory.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;

        let decoder = factory.CreateDecoderFromStream(
            &stream,
            std::ptr::null(),
            WICDecodeMetadataCacheOnDemand,
        )?;

        decoder_to_premultiplied_bgra(decoder)
    }
}

pub(crate) fn read_path_to_premultiplied_bgra(path: &Path) -> Result<PremultipliedBitmap> {
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

        let path = dunce::canonicalize(path)?;
        let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(once(0)).collect();

        let decoder = factory.CreateDecoderFromFilename(
            PCWSTR::from_raw(wide_path.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;

        decoder_to_premultiplied_bgra(decoder)
    }
}

pub(crate) fn read_bytes_to_hbitmap(bytes: &[u8]) -> Result<HBITMAP> {
    Ok(read_bytes_to_premultiplied_bgra(bytes)?.to_device_bitmap())
}

pub(crate) fn read_path_to_hbitmap(path: &Path) -> Result<HBITMAP> {
    Ok(read_path_to_premultiplied_bgra(path)?.to_device_bitmap())
}

fn decoder_to_premultiplied_bgra(decoder: IWICBitmapDecoder) -> Result<PremultipliedBitmap> {
    unsafe {
        let frame = decoder.GetFrame(0)?;

        let mut width: u32 = 0;
        let mut height: u32 = 0;
        frame.GetSize(&mut width, &mut height)?;

        let mut pixels: Vec<u8> = vec![0; (width * height * 4) as usize];
        let pixel_format = frame.GetPixelFormat()?;
        if pixel_format != GUID_WICPixelFormat32bppPBGRA {
            let bitmap_source = WICConvertBitmapSource(&GUID_WICPixelFormat32bppPBGRA, &frame)?;
            bitmap_source.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
        } else {
            frame.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
        }

        Ok(PremultipliedBitmap {
            width: width as i32,
            height: height as i32,
            pixels,
        })
    }
}
