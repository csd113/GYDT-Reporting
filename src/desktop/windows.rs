use winsafe::{HWND, HwndPlace, POINT, RECT, SIZE, co};

// Keep normal text scaling. The Windows startup size is 75% of 1136 × 823.
pub(super) const REPORTING_SIZE: [f32; 2] = [852.0, 617.25];

fn edges(rect: RECT) -> [i32; 4] {
    [rect.left, rect.top, rect.right, rect.bottom]
}

pub(super) fn fit_to_work_area(last: &mut Option<[i32; 4]>) {
    let Some(window) = HWND::GetActiveWindow() else {
        return;
    };
    if window.GetWindowThreadProcessId().1 != std::process::id() {
        return;
    }
    let Ok(monitor) = window
        .MonitorFromWindow(co::MONITOR::DEFAULTTONEAREST)
        .GetMonitorInfo()
    else {
        return;
    };
    let work = edges(monitor.rcWork);
    if *last == Some(work) {
        return;
    }
    let Ok(bounds) = window.GetWindowRect() else {
        return;
    };
    let current = edges(bounds);
    let Some(fitted) = fit_outer_rect(current, work) else {
        return;
    };
    let [left, top, right, bottom] = fitted;
    if current == fitted
        || window
            .SetWindowPos(
                HwndPlace::None,
                POINT { x: left, y: top },
                SIZE {
                    cx: right - left,
                    cy: bottom - top,
                },
                co::SWP::NOZORDER | co::SWP::NOACTIVATE,
            )
            .is_ok()
    {
        *last = Some(work);
    }
}

fn fit_outer_rect(current: [i32; 4], work: [i32; 4]) -> Option<[i32; 4]> {
    let [left, top, right, bottom] = current;
    let [min_x, min_y, max_x, max_y] = work;
    let width = (right - left).min(max_x - min_x);
    let height = (bottom - top).min(max_y - min_y);
    if width <= 0 || height <= 0 {
        return None;
    }
    let x = left.clamp(min_x, max_x - width);
    let y = top.clamp(min_y, max_y - height);
    Some([x, y, x + width, y + height])
}

/// Rename only an accepted drop, in place and without replacing an existing file.
pub(super) fn rename_dropped_photo(
    source: &std::path::Path,
    title: &str,
    photo: &super::model::PhotoCopy,
) -> super::model::Result<String> {
    let title = super::model::flat_text(title);
    if title.is_empty() {
        return Err(
            "The sample has no submission number yet. The source was left unchanged.".into(),
        );
    }
    let name = format!(
        "{}.{}",
        super::model::filename_stem(&title),
        photo.extension
    );
    if source
        .file_name()
        .is_some_and(|current| current == name.as_str())
    {
        return Ok(name);
    }
    let current = super::archive::read_file(source, super::model::MAX_JPEG, "JPG/JPEG photo")
        .map_err(|error| format!("Could not recheck the original before renaming: {error}"))?;
    if current != photo.bytes()? {
        return Err("The source changed after selection. The source was left unchanged.".into());
    }
    let from = source.canonicalize()?;
    let to = from.with_file_name(&name);
    let source_text = from.to_str().ok_or("The original path cannot be read.")?;
    let target_text = to.to_str().ok_or("The destination path cannot be read.")?;
    winsafe::MoveFile(source_text, target_text).map_err(|error| {
        if matches!(error, co::ERROR::FILE_EXISTS | co::ERROR::ALREADY_EXISTS) {
            format!("{name} already exists. Neither original was overwritten.")
        } else {
            format!("Windows could not rename the original to {name}: {error}. The attachment is still available for export.")
        }
    })?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::fit_outer_rect;

    #[test]
    fn windows_work_area_contains_decoration_and_excludes_taskbar() {
        for (current, work, expected) in [
            ([44, 20, 1330, 990], [0, 0, 1366, 720], [44, 0, 1330, 720]),
            (
                [220, 150, 1510, 1116],
                [0, 0, 1920, 1008],
                [220, 42, 1510, 1008],
            ),
            (
                [130, 220, 1210, 1028],
                [0, 52, 1920, 1080],
                [130, 220, 1210, 1028],
            ),
            (
                [-1400, 0, -40, 1000],
                [-1366, 0, -72, 712],
                [-1366, 0, -72, 712],
            ),
        ] {
            assert_eq!(fit_outer_rect(current, work), Some(expected));
        }
    }
}
