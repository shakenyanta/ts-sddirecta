use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

// All calculations use physical pixels, including the measured native window frame.
fn fitted_inner_size(
    available: PhysicalSize<u32>,
    inner: PhysicalSize<u32>,
    outer: PhysicalSize<u32>,
) -> Option<PhysicalSize<u32>> {
    let frame_width = outer.width.saturating_sub(inner.width);
    let frame_height = outer.height.saturating_sub(inner.height);
    let width = inner.width.min(available.width.checked_sub(frame_width)?);
    let height = inner
        .height
        .min(available.height.checked_sub(frame_height)?);
    (width > 0 && height > 0).then(|| PhysicalSize::new(width, height))
}

pub fn fit_startup_window(window: &WebviewWindow) -> Result<(), String> {
    let monitor = match window.current_monitor().map_err(|e| e.to_string())? {
        Some(monitor) => monitor,
        None => window
            .primary_monitor()
            .map_err(|e| e.to_string())?
            .ok_or("起動画面を取得できませんでした")?,
    };
    let work = monitor.work_area();
    let margin = (8.0 * monitor.scale_factor()).round() as u32;
    let margin = margin.min(work.size.width / 4).min(work.size.height / 4);
    let available = PhysicalSize::new(
        work.size.width.saturating_sub(margin * 2),
        work.size.height.saturating_sub(margin * 2),
    );
    let inner = window.inner_size().map_err(|e| e.to_string())?;
    let outer = window.outer_size().map_err(|e| e.to_string())?;
    let size = fitted_inner_size(available, inner, outer)
        .ok_or("ウィンドウ枠を作業領域に収められませんでした")?;
    if size != inner {
        window.set_size(size).map_err(|e| e.to_string())?;
    }

    // Measure again after resizing so the title bar and borders stay inside the work area.
    let outer = window.outer_size().map_err(|e| e.to_string())?;
    if outer.width > work.size.width || outer.height > work.size.height {
        return Err("サイズ変更後のウィンドウが作業領域を超えています".into());
    }
    let position = PhysicalPosition::new(
        work.position.x + ((work.size.width - outer.width) / 2) as i32,
        work.position.y + ((work.size.height - outer.height) / 2) as i32,
    );
    window.set_position(position).map_err(|e| e.to_string())?;
    crate::diagnostics::record(
        "INFO",
        &format!(
            "startup window fitted scale={} work={:?} inner={:?} outer={:?} position={:?}",
            monitor.scale_factor(),
            work,
            size,
            outer,
            position
        ),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_pro_and_lite_sizes_when_they_fit() {
        for (width, height) in [(300, 350), (1000, 600)] {
            let inner = PhysicalSize::new(width, height);
            assert_eq!(
                fitted_inner_size(
                    PhysicalSize::new(1264, 704),
                    inner,
                    PhysicalSize::new(width + 16, height + 39),
                ),
                Some(inner)
            );
        }
    }

    #[test]
    fn shrinks_pro_on_a_small_laptop_at_150_percent() {
        // 1280x720 screen, 60px taskbar, 12px margins, 150% window/frame sizes.
        assert_eq!(
            fitted_inner_size(
                PhysicalSize::new(1256, 636),
                PhysicalSize::new(1500, 900),
                PhysicalSize::new(1524, 959),
            ),
            Some(PhysicalSize::new(1232, 577))
        );
    }

    #[test]
    fn fits_pro_on_an_800_by_480_screen() {
        // Reserve a 40px taskbar and 8px margins on each side at 100% scale.
        assert_eq!(
            fitted_inner_size(
                PhysicalSize::new(784, 424),
                PhysicalSize::new(1000, 600),
                PhysicalSize::new(1016, 639),
            ),
            Some(PhysicalSize::new(768, 385))
        );
    }

    #[test]
    fn shrinks_both_dimensions_including_title_bar() {
        assert_eq!(
            fitted_inner_size(
                PhysicalSize::new(1000, 600),
                PhysicalSize::new(1800, 700),
                PhysicalSize::new(1832, 778),
            ),
            Some(PhysicalSize::new(968, 522))
        );
    }

    #[test]
    fn rejects_work_area_smaller_than_the_frame() {
        assert_eq!(
            fitted_inner_size(
                PhysicalSize::new(16, 39),
                PhysicalSize::new(300, 350),
                PhysicalSize::new(316, 389),
            ),
            None
        );
    }
}
