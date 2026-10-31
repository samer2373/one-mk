// SPDX-License-Identifier: MIT OR Apache-2.0
//! One decoded frame -> each local monitor's slice of the shared canvas.
//!
//! Design section 3: one centre-anchored "cover" transform maps the source
//! over the whole committed canvas `B` (including space reserved for absent
//! computers); every output pixel centre is mapped back to a source pixel with
//! exact rational arithmetic, so neighbouring monitors (and other computers
//! using the same rule) never round their seams independently. Sampling is
//! nearest-neighbour on the CPU (laboratory; the GPU path is future work).

use one_mk_wall_sync::bridge::Monitor;
use one_mk_wall_sync::presentation::{PresentationPlan, Rect};

/// Where one monitor's pixels come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorMap {
    /// Top-left of this monitor inside the presentation surface.
    pub x: usize,
    pub y: usize,
    /// Source column for each output column, and source row for each output row.
    pub columns: Vec<u32>,
    pub rows: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Size of the one surface spanning the local monitors (logical pixels).
    pub width: usize,
    pub height: usize,
    pub source: (u32, u32),
    pub monitors: Vec<MonitorMap>,
}

/// Source index sampled by canvas-relative output pixel `position` (cover fit).
/// `canvas` and `source` are lengths along one axis; `numerator / denominator`
/// is the uniform scale chosen on the dominant axis.
fn sample(position: i64, canvas: i64, source: i64, numerator: i64, denominator: i64) -> u32 {
    // u = (position + 1/2 - offset) / scale, offset = (canvas - source * scale) / 2
    let value = (i128::from(2 * position + 1) * i128::from(denominator)
        - (i128::from(canvas) * i128::from(denominator) - i128::from(source) * i128::from(numerator)))
        .div_euclid(2 * i128::from(numerator));
    value.clamp(0, i128::from(source) - 1) as u32
}

impl Layout {
    /// The surface and per-monitor sampling for an admitted plan. `monitors`
    /// are the bridge's local rectangles, in the order of the plan's slices.
    pub fn new(plan: &PresentationPlan, monitors: &[Monitor], source: (u32, u32)) -> Result<Self, String> {
        if monitors.is_empty() || monitors.len() != plan.slices.len() { return Err("plan does not match the local monitors".into()); }
        let (source_width, source_height) = (i64::from(source.0), i64::from(source.1));
        if source_width == 0 || source_height == 0 { return Err("empty source".into()); }
        let Rect { x: canvas_x, y: canvas_y, width: canvas_width, height: canvas_height } = plan.canvas;
        let (canvas_width, canvas_height) = (i64::from(canvas_width), i64::from(canvas_height));
        // Cover: the larger of the two axis scales, kept as an exact fraction.
        let (numerator, denominator) = if canvas_width * source_height >= canvas_height * source_width {
            (canvas_width, source_width)
        } else {
            (canvas_height, source_height)
        };
        let left = monitors.iter().map(|monitor| monitor.0).min().ok_or("no monitors")?;
        let top = monitors.iter().map(|monitor| monitor.1).min().ok_or("no monitors")?;
        let right = monitors.iter().map(|monitor| i64::from(monitor.0) + i64::from(monitor.2)).max().ok_or("no monitors")?;
        let bottom = monitors.iter().map(|monitor| i64::from(monitor.1) + i64::from(monitor.3)).max().ok_or("no monitors")?;
        let (width, height) = (right - i64::from(left), bottom - i64::from(top));
        if !(1..=16384).contains(&width) || !(1..=16384).contains(&height) || width * height > 64_000_000 {
            return Err("local surface exceeds the pixel budget".into());
        }
        let mut maps = Vec::with_capacity(monitors.len());
        for (slice, &(x, y, monitor_width, monitor_height, _)) in plan.slices.iter().zip(monitors) {
            if i64::from(slice.canvas.width) != i64::from(monitor_width) || i64::from(slice.canvas.height) != i64::from(monitor_height) {
                return Err("slice size differs from its monitor".into());
            }
            let origin_x = i64::from(slice.canvas.x) - i64::from(canvas_x);
            let origin_y = i64::from(slice.canvas.y) - i64::from(canvas_y);
            let columns = (0..i64::from(monitor_width))
                .map(|column| sample(origin_x + column, canvas_width, source_width, numerator, denominator)).collect();
            let rows = (0..i64::from(monitor_height))
                .map(|row| sample(origin_y + row, canvas_height, source_height, numerator, denominator)).collect();
            maps.push(MonitorMap { x: (i64::from(x) - i64::from(left)) as usize, y: (i64::from(y) - i64::from(top)) as usize, columns, rows });
        }
        Ok(Self { width: width as usize, height: height as usize, source, monitors: maps })
    }

    /// Writes every monitor's slice of `frame` (source-sized, one u32 per
    /// pixel) into `surface` (`width * height`). Pixels outside every monitor
    /// are left untouched.
    pub fn compose(&self, frame: &[u32], surface: &mut [u32]) {
        let source_width = self.source.0 as usize;
        for map in &self.monitors {
            let width = map.columns.len();
            let mut previous: Option<(u32, usize)> = None;
            for (row, &source_row) in map.rows.iter().enumerate() {
                let start = (map.y + row) * self.width + map.x;
                match previous {
                    Some((last, from)) if last == source_row => surface.copy_within(from..from + width, start),
                    _ => {
                        let line = &frame[source_row as usize * source_width..][..source_width];
                        for (target, &column) in surface[start..start + width].iter_mut().zip(&map.columns) {
                            *target = line[column as usize];
                        }
                    }
                }
                previous = Some((source_row, start));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_wall_sync::presentation::Slice;

    fn plan(canvas: Rect, slices: &[(i32, i32, u32, u32)]) -> PresentationPlan {
        PresentationPlan {
            generation: [1; 16], consent_revision: 1, sequence: 1, layout_serial: 1, media_id: [2; 32], canvas,
            slices: slices.iter().enumerate().map(|(index, &(x, y, width, height))| Slice {
                screen: format!("monitor-{index}"), canvas: Rect { x, y, width, height } }).collect(),
            looping: true, paused: false,
        }
    }

    #[test]
    fn integer_cover_maps_each_source_pixel_to_a_block_and_skips_the_reserved_slice() {
        // The laboratory canvas: an absent computer reserves x -1920..0.
        let plan = plan(Rect { x: -1920, y: 0, width: 5760, height: 1080 }, &[(0, 0, 1920, 1080), (1920, 0, 1920, 1080)]);
        let monitors = [(0, 0, 1920, 1080, 1.0), (1920, 0, 1920, 1080, 1.0)];
        let layout = Layout::new(&plan, &monitors, (576, 108)).unwrap();
        assert_eq!((layout.width, layout.height), (3840, 1080));
        // Monitor 0 starts at canvas column 1920 = source column 192, never 0..191.
        assert_eq!(layout.monitors[0].columns[0], 192);
        assert_eq!(layout.monitors[0].columns[9], 192);
        assert_eq!(layout.monitors[0].columns[10], 193);
        assert_eq!(layout.monitors[1].columns[0], 384);
        assert_eq!(*layout.monitors[1].columns.last().unwrap(), 575);
        assert_eq!((layout.monitors[0].rows[0], layout.monitors[0].rows[1079]), (0, 107));
        assert!(layout.monitors.iter().all(|map| map.columns.iter().all(|&column| column >= 192)));
        // The seam is continuous: the last column of monitor 0 and the first of
        // monitor 1 are adjacent canvas columns.
        assert_eq!(layout.monitors[0].columns[1919], 383);
    }

    #[test]
    fn cover_crops_the_centre_of_a_mismatched_aspect() {
        // A 16:9 1080p source on a 16:3 canvas: scale 3, rows 360..720 visible.
        let wide = plan(Rect { x: -1920, y: 0, width: 5760, height: 1080 }, &[(0, 0, 1920, 1080), (1920, 0, 1920, 1080)]);
        let monitors = [(0, 0, 1920, 1080, 1.0), (1920, 0, 1920, 1080, 1.0)];
        let layout = Layout::new(&wide, &monitors, (1920, 1080)).unwrap();
        assert_eq!(layout.monitors[0].rows[0], 360);
        assert_eq!(*layout.monitors[0].rows.last().unwrap(), 719);
        assert_eq!(layout.monitors[0].columns[0], 640);
        assert_eq!(*layout.monitors[1].columns.last().unwrap(), 1919);
        // A tall canvas crops columns instead.
        let tall = plan(Rect { x: 0, y: 0, width: 100, height: 400 }, &[(0, 0, 100, 400)]);
        let layout = Layout::new(&tall, &[(0, 0, 100, 400, 1.0)], (100, 100)).unwrap();
        assert_eq!((layout.monitors[0].columns[0], *layout.monitors[0].columns.last().unwrap()), (37, 62));
        assert_eq!((layout.monitors[0].rows[0], *layout.monitors[0].rows.last().unwrap()), (0, 99));
    }

    #[test]
    fn compose_writes_each_slice_and_leaves_gaps_alone() {
        let plan = plan(Rect { x: 0, y: 0, width: 8, height: 2 }, &[(0, 0, 2, 2), (6, 0, 2, 2)]);
        // Monitors are 2x2 at local x 0 and 4 (a 2-pixel gap), canvas x 0 and 6.
        let monitors = [(0, 0, 2, 2, 1.0), (4, 0, 2, 2, 1.0)];
        let layout = Layout::new(&plan, &monitors, (4, 1)).unwrap();
        let frame = [10, 11, 12, 13];
        let mut surface = vec![7_u32; 12];
        layout.compose(&frame, &mut surface);
        assert_eq!(surface, [10, 10, 7, 7, 13, 13, 10, 10, 7, 7, 13, 13]);
    }

    #[test]
    fn mismatched_plans_are_refused() {
        let plan = plan(Rect { x: 0, y: 0, width: 8, height: 2 }, &[(0, 0, 2, 2)]);
        assert!(Layout::new(&plan, &[], (4, 1)).is_err());
        assert!(Layout::new(&plan, &[(0, 0, 3, 2, 1.0)], (4, 1)).is_err());
        assert!(Layout::new(&plan, &[(0, 0, 2, 2, 1.0)], (0, 1)).is_err());
    }
}
