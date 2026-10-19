//! Impression (Windows) : fenêtre d'impression du système, puis chaque page est rendue
//! (300 dpi au plus) et envoyée à l'imprimante, ajustée à la zone imprimable et tournée
//! si son orientation diffère de celle du papier.

use crate::pdf::Rendered;

/// Travail d'impression choisi par l'utilisateur. `dc` : contexte de l'imprimante (passé
/// au thread moteur, qui seul l'utilise ensuite).
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Job {
    pub dc: usize,
    pub pages: Vec<usize>,
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HDC, HORZRES, LOGPIXELSX,
        SRCCOPY, SetStretchBltMode, HALFTONE, StretchDIBits, VERTRES,
    };
    use windows_sys::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
    use windows_sys::Win32::UI::Controls::Dialogs::{
        PD_NOSELECTION, PD_PAGENUMS, PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW, PrintDlgW,
    };

    /// Fenêtre d'impression ; `None` : annulée.
    pub fn choose(count: usize) -> Option<Job> {
        let mut pd: PRINTDLGW = unsafe { std::mem::zeroed() };
        pd.lStructSize = std::mem::size_of::<PRINTDLGW>() as u32;
        pd.Flags = PD_RETURNDC | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE;
        pd.nMinPage = 1;
        pd.nMaxPage = count.min(u16::MAX as usize) as u16;
        pd.nFromPage = 1;
        pd.nToPage = pd.nMaxPage;
        pd.nCopies = 1;
        let ok = unsafe { PrintDlgW(&mut pd) } != 0;
        unsafe {
            if !pd.hDevMode.is_null() {
                GlobalFree(pd.hDevMode);
            }
            if !pd.hDevNames.is_null() {
                GlobalFree(pd.hDevNames);
            }
        }
        if !ok || pd.hDC.is_null() {
            return None;
        }
        let pages = if pd.Flags & PD_PAGENUMS != 0 {
            (pd.nFromPage.max(1) as usize - 1..(pd.nToPage as usize).min(count)).collect()
        } else {
            (0..count).collect()
        };
        Some(Job { dc: pd.hDC as usize, pages })
    }

    /// Imprime ; `render(page, zoom, rotation)` rend une page. Renvoie le nombre de pages.
    pub fn run(job: Job, title: &str, sizes: &[(f32, f32)],
               mut render: impl FnMut(usize, f32, i32) -> Result<Rendered, String>,
               mut progress: impl FnMut(usize, usize)) -> Result<usize, String> {
        let dc = job.dc as HDC;
        let result = (|| unsafe {
            let dpi = GetDeviceCaps(dc, LOGPIXELSX as _).max(72) as f32;
            let (pw, ph) = (GetDeviceCaps(dc, HORZRES as _) as f32, GetDeviceCaps(dc, VERTRES as _) as f32);
            let name: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            let mut info: DOCINFOW = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<DOCINFOW>() as i32;
            info.lpszDocName = name.as_ptr();
            if StartDocW(dc, &info) <= 0 {
                return Err("L'imprimante a refusé le travail d'impression.".to_string());
            }
            SetStretchBltMode(dc, HALFTONE);
            for (k, &i) in job.pages.iter().enumerate() {
                progress(k + 1, job.pages.len());
                let (w, h) = sizes[i];
                // Page couchée sur papier debout (ou l'inverse) : tournée d'un quart de tour.
                let turn = (w > h) != (pw > ph);
                let (w, h) = if turn { (h, w) } else { (w, h) };
                let fit = (pw / (w / 72.0 * dpi)).min(ph / (h / 72.0 * dpi)).min(1.0);
                let (tw, th) = (w / 72.0 * dpi * fit, h / 72.0 * dpi * fit);
                let r = match render(i, dpi.min(300.0) / 72.0 * fit, if turn { 90 } else { 0 }) {
                    Ok(r) => r,
                    Err(e) => {
                        AbortDoc(dc);
                        return Err(e);
                    }
                };
                // Lignes BGR alignées sur 4 octets, de haut en bas (hauteur négative).
                let stride = (r.width as usize * 3).div_ceil(4) * 4;
                let mut bits = vec![0u8; stride * r.height as usize];
                for (y, row) in r.rgb.chunks(r.width as usize * 3).enumerate() {
                    for (x, px) in row.chunks(3).enumerate() {
                        let o = y * stride + x * 3;
                        bits[o..o + 3].copy_from_slice(&[px[2], px[1], px[0]]);
                    }
                }
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = r.width as i32;
                bmi.bmiHeader.biHeight = -(r.height as i32);
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 24;
                bmi.bmiHeader.biCompression = BI_RGB;
                if StartPage(dc) <= 0 {
                    AbortDoc(dc);
                    return Err("L'imprimante a refusé une page.".to_string());
                }
                StretchDIBits(dc, ((pw - tw) / 2.0) as i32, ((ph - th) / 2.0) as i32, tw as i32, th as i32,
                              0, 0, r.width as i32, r.height as i32, bits.as_ptr() as *const c_void, &bmi,
                              DIB_RGB_COLORS, SRCCOPY);
                EndPage(dc);
            }
            EndDoc(dc);
            Ok(job.pages.len())
        })();
        unsafe { DeleteDC(dc) };
        result
    }
}

#[cfg(windows)]
pub use win::{choose, run};

#[cfg(not(windows))]
pub fn choose(_count: usize) -> Option<Job> {
    None
}

#[cfg(not(windows))]
pub fn run(_job: Job, _title: &str, _sizes: &[(f32, f32)], _render: impl FnMut(usize, f32, i32) -> Result<Rendered, String>,
           _progress: impl FnMut(usize, usize)) -> Result<usize, String> {
    Err("L'impression n'est disponible que sous Windows.".into())
}
