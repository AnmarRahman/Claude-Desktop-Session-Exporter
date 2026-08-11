use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::mem::size_of;
use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::capture::CaptureError;
use crate::models::{DetectedWindow, VisibleContentCapture};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SetForegroundWindow};

const BMP_FILE_HEADER_SIZE: u32 = 14;
const BMP_INFO_HEADER_SIZE: u32 = 40;
const BYTES_PER_PIXEL: usize = 4;

pub fn capture_window_to_bmp(
    window: &DetectedWindow,
    path: &Path,
) -> Result<VisibleContentCapture, CaptureError> {
    let hwnd =
        window.hwnd.as_deref().and_then(parse_hwnd).ok_or_else(|| {
            CaptureError::Native("Claude HWND was missing or invalid.".to_string())
        })?;
    let hwnd = HWND(hwnd as *mut c_void);
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
    thread::sleep(Duration::from_millis(250));

    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }
        .map_err(|error| CaptureError::Native(error.to_string()))?;
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return Err(CaptureError::Native(format!(
            "Detected Claude window has invalid bounds: {}x{}.",
            width, height
        )));
    }

    let mut warnings = Vec::new();
    let bitmap = unsafe { capture_bitmap(rect, width, height, &mut warnings)? };
    write_bmp(path, width, height, &bitmap)?;

    warnings.push(
        "Visible capture saved as BMP. OCR text extraction is not available because no OCR engine was found on this machine.".to_string(),
    );

    Ok(VisibleContentCapture {
        image_path: path.display().to_string(),
        text: None,
        warnings,
    })
}

unsafe fn capture_bitmap(
    rect: RECT,
    width: i32,
    height: i32,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>, CaptureError> {
    let screen_dc = GetDC(None);
    if screen_dc.is_invalid() {
        return Err(CaptureError::Native(
            "Could not acquire the screen device context.".to_string(),
        ));
    }

    let memory_dc = CreateCompatibleDC(Some(screen_dc));
    if memory_dc.is_invalid() {
        let _ = ReleaseDC(None, screen_dc);
        return Err(CaptureError::Native(
            "Could not create an in-memory device context.".to_string(),
        ));
    }

    let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
    if bitmap.is_invalid() {
        let _ = DeleteDC(memory_dc);
        let _ = ReleaseDC(None, screen_dc);
        return Err(CaptureError::Native(
            "Could not create an in-memory bitmap for the Claude window.".to_string(),
        ));
    }

    let previous = SelectObject(memory_dc, HGDIOBJ(bitmap.0));
    warnings.push(
        "Brought Claude to the foreground and captured its visible screen region.".to_string(),
    );
    let copied = BitBlt(
        memory_dc,
        0,
        0,
        width,
        height,
        Some(screen_dc),
        rect.left,
        rect.top,
        SRCCOPY,
    );
    if copied.is_err() {
        let _ = SelectObject(memory_dc, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory_dc);
        let _ = ReleaseDC(None, screen_dc);
        return Err(CaptureError::Native(
            "Could not copy pixels from the detected Claude window.".to_string(),
        ));
    }

    let mut buffer = vec![0u8; width as usize * height as usize * BYTES_PER_PIXEL];
    let mut bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: buffer.len() as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [Default::default()],
    };

    let scan_lines = GetDIBits(
        memory_dc,
        bitmap,
        0,
        height as u32,
        Some(buffer.as_mut_ptr() as *mut c_void),
        &mut bitmap_info,
        DIB_RGB_COLORS,
    );

    let _ = SelectObject(memory_dc, previous);
    let _ = DeleteObject(HGDIOBJ(bitmap.0));
    let _ = DeleteDC(memory_dc);
    let _ = ReleaseDC(None, screen_dc);

    if scan_lines == 0 {
        return Err(CaptureError::Native(
            "Could not read pixels from the captured Claude bitmap.".to_string(),
        ));
    }

    Ok(buffer)
}

fn write_bmp(path: &Path, width: i32, height: i32, pixels: &[u8]) -> Result<(), CaptureError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    }

    let pixel_bytes = pixels.len() as u32;
    let pixel_offset = BMP_FILE_HEADER_SIZE + BMP_INFO_HEADER_SIZE;
    let file_size = pixel_offset + pixel_bytes;
    let mut file =
        fs::File::create(path).map_err(|error| CaptureError::Diagnostic(error.to_string()))?;

    file.write_all(b"BM")
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&file_size.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&0u16.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&0u16.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&pixel_offset.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;

    file.write_all(&BMP_INFO_HEADER_SIZE.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&width.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&(-height).to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&1u16.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&32u16.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&0u32.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&pixel_bytes.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&0i32.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&0i32.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&0u32.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(&0u32.to_le_bytes())
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    file.write_all(pixels)
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;

    Ok(())
}

fn parse_hwnd(value: &str) -> Option<isize> {
    let trimmed = value.trim();
    if let Some(hex) = trimmed.strip_prefix("0x") {
        isize::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse().ok()
    }
}
