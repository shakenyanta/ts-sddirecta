use base64::{engine::general_purpose::STANDARD, Engine};
use std::{mem::{size_of, zeroed}, path::Path};
use windows_sys::Win32::{
    Graphics::Gdi::{CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS},
    System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
    UI::{Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ADDOVERLAYS, SHGFI_ICON, SHGFI_SMALLICON},
        WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL, HICON}},
};

const SIDE: usize = 16;

// fs::canonicalize returns verbatim paths, which Shell APIs do not accept.
fn shell_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    }
}

/// Shell extraction reads the associated icon without launching the file or shortcut.
pub fn data_url(path: &Path) -> Result<String, String> {
    // Tauri's blocking worker threads do not necessarily have COM initialized.
    let initialized = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    if initialized < 0 { return Err(format!("COM初期化失敗: {initialized:#x}")); }
    let result = (|| {
        let path: Vec<u16> = shell_path(path).encode_utf16().chain(Some(0)).collect();
        let mut info: SHFILEINFOW = unsafe { zeroed() };
        let found = unsafe { SHGetFileInfoW(path.as_ptr(), 0, &mut info, size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON | SHGFI_ADDOVERLAYS) };
        if found == 0 || info.hIcon == 0 { return Err("ファイルアイコンを取得できませんでした".into()); }
        let result = render(info.hIcon);
        unsafe { DestroyIcon(info.hIcon); }
        result
    })();
    unsafe { CoUninitialize(); }
    result
}

fn render(icon: HICON) -> Result<String, String> {
    unsafe {
        let dc = CreateCompatibleDC(0);
        if dc == 0 { return Err("アイコン描画用DCを作成できませんでした".into()); }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIDE as i32, biHeight: -(SIDE as i32), biPlanes: 1, biBitCount: 32,
            biCompression: BI_RGB, ..zeroed() };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, 0, 0);
        if bitmap == 0 {
            DeleteDC(dc);
            return Err("アイコン描画用画像を作成できませんでした".into());
        }
        let previous = SelectObject(dc, bitmap);
        let result = (|| {
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, SIDE * SIDE * 4);
            // Render on both black and white to recover transparency for alpha and legacy icons.
            pixels.fill(0);
            if DrawIconEx(dc, 0, 0, icon, SIDE as i32, SIDE as i32, 0, 0, DI_NORMAL) == 0 {
                return Err("アイコンを描画できませんでした".into());
            }
            GdiFlush();
            let black = pixels.to_vec();
            pixels.fill(255);
            if DrawIconEx(dc, 0, 0, icon, SIDE as i32, SIDE as i32, 0, 0, DI_NORMAL) == 0 {
                return Err("アイコンを描画できませんでした".into());
            }
            GdiFlush();
            let mut rgba = Vec::with_capacity(black.len());
            for (black, white) in black.chunks_exact(4).zip(pixels.chunks_exact(4)) {
                let alpha = 255 - (0..3).map(|i| white[i].saturating_sub(black[i])).max().unwrap();
                for channel in [2, 1, 0] {
                    rgba.push(if alpha == 0 { 0 } else { (black[channel] as u32 * 255 / alpha as u32).min(255) as u8 });
                }
                rgba.push(alpha);
            }
            let mut png = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut png, SIDE as u32, SIDE as u32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
                writer.write_image_data(&rgba).map_err(|error| error.to_string())?;
            }
            Ok(format!("data:image/png;base64,{}", STANDARD.encode(png)))
        })();
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_paths_preserve_drive_and_unc_locations() {
        for (input, expected) in [
            (r"C:\日本語\app.exe", r"C:\日本語\app.exe"),
            (r"\\?\C:\日本語\app.exe", r"C:\日本語\app.exe"),
            (r"\\?\UNC\server\share\app.lnk", r"\\server\share\app.lnk"),
        ] {
            assert_eq!(shell_path(Path::new(input)), expected);
        }
    }

    #[test]
    fn shell_icons_encode_as_transparent_16_pixel_pngs() {
        let root = tempfile::tempdir().unwrap();
        let text = root.path().join("日本語.txt");
        std::fs::write(&text, b"text").unwrap();
        for path in [std::fs::canonicalize(text).unwrap(), std::env::current_exe().unwrap()] {
            let url = data_url(&path).unwrap();
            let bytes = STANDARD.decode(url.strip_prefix("data:image/png;base64,").unwrap()).unwrap();
            let decoder = png::Decoder::new(bytes.as_slice());
            let mut reader = decoder.read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!((info.width, info.height, info.color_type), (16, 16, png::ColorType::Rgba));
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] > 0));
        }
    }
}
