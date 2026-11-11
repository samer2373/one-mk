//! Multi-computer, mixed-density, rotated and negative-origin layouts: the canvas maths has to
//! put every pixel of every screen at one exact place of one picture, wherever the screens stand.

use one_mk_wall_canvas::*;

const SCALES: [(Scale, u32); 4] = [(Scale::One, 4), (Scale::FiveFourths, 5), (Scale::ThreeHalves, 6), (Scale::Two, 8)];
const ROTATIONS: [Rotation; 4] = [Rotation::Zero, Rotation::Clockwise90, Rotation::Half, Rotation::Clockwise270];
const FITS: [Fit; 4] = [Fit::Cover, Fit::Contain, Fit::Stretch, Fit::Tile];

/// A screen of `width` x `height` logical units, its pixels as the scale and rotation make them.
fn screen(device: &str, name: &str, x: i64, y: i64, width: u32, height: u32, scale: (Scale, u32), rotation: Rotation) -> Output {
    let (buffer_width, buffer_height) = (width * scale.1 / 4, height * scale.1 / 4);
    let (buffer_width, buffer_height) = if matches!(rotation, Rotation::Clockwise90 | Rotation::Clockwise270) { (buffer_height, buffer_width) } else { (buffer_width, buffer_height) };
    Output { device: device.into(), screen: name.into(), x, y, width, height, scale: scale.0, rotation, buffer_width, buffer_height }
}

fn stretch_source(layout: &Layout) -> Source {
    let bounds = layout.bounds();
    Source::new(bounds.width, bounds.height, 1, 1, Rotation::Zero).unwrap()
}

/// Every pixel of every screen lands inside its own screen's rectangle of the picture, and every
/// unit cell of that rectangle is hit by at least one pixel: no seam, gap or bleed.
fn assert_tiles_exactly(layout: &Layout, what: &str) {
    let bounds = layout.bounds();
    let source = stretch_source(layout);
    for (index, output) in layout.outputs().iter().enumerate() {
        let (left, top) = (output.x - bounds.x, output.y - bounds.y);
        let mut hit = vec![false; output.width as usize * output.height as usize];
        for y in 0..output.buffer_height {
            for x in 0..output.buffer_width {
                let sample = layout.sample(index, x, y, source, Fit::Stretch).unwrap().expect("a stretched picture covers every pixel");
                let (cell_x, cell_y) = (sample.x.floor() - i128::from(left), sample.y.floor() - i128::from(top));
                assert!((0..i128::from(output.width)).contains(&cell_x) && (0..i128::from(output.height)).contains(&cell_y),
                    "{what}: screen {index} pixel ({x},{y}) lands at cell ({cell_x},{cell_y}), outside its own {}x{} rectangle", output.width, output.height);
                hit[cell_y as usize * output.width as usize + cell_x as usize] = true;
            }
        }
        assert!(hit.iter().all(|cell| *cell), "{what}: screen {index} leaves cells of its rectangle without a pixel");
    }
}

#[test]
fn mixed_density_and_rotation_neighbours_tile_their_rectangles_exactly() {
    for (left_scale, left_rotation) in SCALES.iter().flat_map(|scale| ROTATIONS.iter().map(move |rotation| (*scale, *rotation))) {
        for right_scale in SCALES {
            let layout = Layout::new(vec![
                screen("A", "left", -16, -8, 16, 8, left_scale, left_rotation),
                screen("B", "right", 0, -4, 8, 12, right_scale, Rotation::Zero),
                screen("C", "below", -16, 0, 16, 4, SCALES[0], Rotation::Half),
            ]).unwrap();
            assert_eq!(layout.bounds(), Bounds { x: -16, y: -8, width: 24, height: 16 });
            assert_tiles_exactly(&layout, &format!("{left_scale:?} {left_rotation:?} beside {right_scale:?}"));
        }
    }
}

#[test]
fn pixel_centres_of_unrotated_mixed_density_screens_are_exactly_where_the_scale_puts_them() {
    let layout = Layout::new(vec![
        screen("A", "one", -10, -6, 10, 6, SCALES[0], Rotation::Zero),
        screen("B", "two", 0, -6, 10, 6, SCALES[3], Rotation::Zero),
        screen("C", "five-fourths", 0, 0, 8, 4, SCALES[1], Rotation::Zero),
    ]).unwrap();
    let source = stretch_source(&layout);
    let bounds = layout.bounds();
    for (index, output) in layout.outputs().iter().enumerate() {
        let quarters = i128::from(match output.scale { Scale::One => 4, Scale::FiveFourths => 5, Scale::ThreeHalves => 6, Scale::Two => 8 });
        for y in 0..output.buffer_height {
            for x in 0..output.buffer_width {
                let sample = layout.sample(index, x, y, source, Fit::Stretch).unwrap().unwrap();
                // Pixel centre (x + 1/2) of `quarters/4` pixels per unit, in units from the picture's corner.
                let expected_x = Rational::new((2 * i128::from(x) + 1) * 2, quarters);
                let expected_y = Rational::new((2 * i128::from(y) + 1) * 2, quarters);
                let (ex, ey) = (expected_x.parts(), expected_y.parts());
                let shift_x = i128::from(output.x - bounds.x);
                let shift_y = i128::from(output.y - bounds.y);
                assert_eq!(sample.x.parts(), (ex.0 + shift_x * ex.1, ex.1), "screen {index} x {x}");
                assert_eq!(sample.y.parts(), (ey.0 + shift_y * ey.1, ey.1), "screen {index} y {y}");
            }
        }
    }
}

#[test]
fn moving_every_screen_together_changes_nothing_for_any_fit() {
    let make = |dx: i64, dy: i64| Layout::new(vec![
        screen("A", "one", -12 + dx, -5 + dy, 12, 6, SCALES[2], Rotation::Clockwise90),
        screen("B", "two", 3 + dx, -9 + dy, 8, 12, SCALES[1], Rotation::Zero),
    ]).unwrap();
    let base = make(0, 0);
    let source = Source::new(31, 17, 4, 3, Rotation::Clockwise270).unwrap();
    for (dx, dy) in [(5, 7), (-300, 41), (999_000, -999_000 + 20), (-12, -5)] {
        let moved = make(dx, dy);
        assert_eq!(moved.bounds().width, base.bounds().width);
        assert_eq!(moved.bounds().height, base.bounds().height);
        assert_eq!((moved.bounds().x, moved.bounds().y), (base.bounds().x + dx, base.bounds().y + dy));
        for fit in FITS {
            for monitor in 0..2 {
                let output = &base.outputs()[monitor];
                for y in 0..output.buffer_height {
                    for x in 0..output.buffer_width {
                        assert_eq!(base.sample(monitor, x, y, source, fit).unwrap(), moved.sample(monitor, x, y, source, fit).unwrap(), "{fit:?} moved by ({dx},{dy}) screen {monitor} ({x},{y})");
                    }
                }
            }
        }
    }
}

#[test]
fn the_order_screens_are_listed_in_does_not_change_what_each_screen_shows() {
    let a = screen("A", "one", -8, -4, 8, 4, SCALES[3], Rotation::Zero);
    let b = screen("B", "two", 0, -4, 8, 4, SCALES[0], Rotation::Half);
    let c = screen("C", "three", -4, 0, 8, 6, SCALES[2], Rotation::Clockwise270);
    let forward = Layout::new(vec![a.clone(), b.clone(), c.clone()]).unwrap();
    let backward = Layout::new(vec![c, b, a]).unwrap();
    assert_eq!(forward.bounds(), backward.bounds());
    let source = Source::new(40, 15, 1, 1, Rotation::Zero).unwrap();
    for fit in FITS {
        for (index, output) in forward.outputs().iter().enumerate() {
            let other = backward.outputs().iter().position(|candidate| candidate.screen == output.screen).unwrap();
            for y in 0..output.buffer_height {
                for x in 0..output.buffer_width {
                    assert_eq!(forward.sample(index, x, y, source, fit).unwrap(), backward.sample(other, x, y, source, fit).unwrap());
                }
            }
        }
    }
}

#[test]
fn cover_fills_every_pixel_and_contain_never_samples_outside_the_picture() {
    let layout = Layout::new(vec![
        screen("A", "wide", -20, -5, 20, 8, SCALES[1], Rotation::Zero),
        screen("B", "tall", 0, -7, 5, 11, SCALES[3], Rotation::Clockwise90),
    ]).unwrap();
    for (width, height, numerator, denominator) in [(1, 1, 1, 1), (16, 9, 1, 1), (9, 16, 1, 1), (100, 3, 1, 1), (3, 100, 1, 1), (7, 5, 10_000, 1), (7, 5, 1, 10_000), (64, 48, 4, 3)] {
        for rotation in ROTATIONS {
            let source = Source::new(width, height, numerator, denominator, rotation).unwrap();
            for monitor in 0..2 {
                let output = &layout.outputs()[monitor];
                for y in 0..output.buffer_height {
                    for x in 0..output.buffer_width {
                        let cover = layout.sample(monitor, x, y, source, Fit::Cover).unwrap();
                        assert!(cover.is_some(), "cover left ({x},{y}) of screen {monitor} empty for {width}x{height} {numerator}/{denominator} {rotation:?}");
                        for fit in FITS {
                            if let Some(sample) = layout.sample(monitor, x, y, source, fit).unwrap() {
                                assert!((0..=i128::from(width)).contains(&sample.x.floor()) && (0..=i128::from(height)).contains(&sample.y.floor()),
                                    "{fit:?} sampled ({}, {}) outside the {width}x{height} picture", sample.x.floor(), sample.y.floor());
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn a_picture_with_the_canvas_aspect_has_no_letterbox_and_a_square_one_has_two_bars() {
    let layout = Layout::new(vec![screen("A", "one", -8, -4, 8, 4, SCALES[0], Rotation::Zero), screen("B", "two", 0, -4, 8, 4, SCALES[0], Rotation::Zero)]).unwrap();
    let exact = Source::new(16, 4, 1, 1, Rotation::Zero).unwrap();
    let pixels = vec![[7, 7, 7, 255]; 16 * 4];
    for monitor in 0..2 {
        assert!(layout.crop(monitor, exact, Fit::Contain, &pixels, [0, 0, 0, 255]).unwrap().iter().all(|pixel| pixel[0] == 7));
    }
    // A square picture on a 4:1 canvas: bars left and right of it, split across the two screens.
    let square = Source::new(4, 4, 1, 1, Rotation::Zero).unwrap();
    let square_pixels = vec![[9, 9, 9, 255]; 16];
    let left = layout.crop(0, square, Fit::Contain, &square_pixels, [1, 1, 1, 255]).unwrap();
    let right = layout.crop(1, square, Fit::Contain, &square_pixels, [1, 1, 1, 255]).unwrap();
    let row = |pixels: &Vec<[u8; 4]>| pixels[..8].iter().map(|pixel| pixel[0]).collect::<Vec<_>>();
    // The 4-wide picture is centred on the seam, at 6..10 of the 16-wide canvas: two units on each screen.
    assert_eq!(row(&left), [1, 1, 1, 1, 1, 1, 9, 9]);
    assert_eq!(row(&right), [9, 9, 1, 1, 1, 1, 1, 1]);
}

#[test]
fn tiling_with_negative_origins_never_leaves_the_picture() {
    let layout = Layout::new(vec![screen("A", "one", -50, -30, 30, 20, SCALES[2], Rotation::Zero), screen("B", "two", -20, -30, 20, 20, SCALES[3], Rotation::Half)]).unwrap();
    let source = Source::new(7, 5, 3, 2, Rotation::Clockwise90).unwrap();
    for monitor in 0..2 {
        let output = &layout.outputs()[monitor];
        let pixels: Vec<[u8; 4]> = (0..35u8).map(|value| [value, 0, 0, 255]).collect();
        let crop = layout.crop(monitor, source, Fit::Tile, &pixels, [0, 0, 0, 255]).unwrap();
        assert_eq!(crop.len(), output.buffer_width as usize * output.buffer_height as usize);
        for y in 0..output.buffer_height {
            for x in 0..output.buffer_width {
                let sample = layout.sample(monitor, x, y, source, Fit::Tile).unwrap().expect("a tiled picture covers every pixel");
                // A pixel centre exactly on a tile edge of a rotated picture reports the far edge; crop clamps it to the last texel.
                assert!((0..=7).contains(&sample.x.floor()) && (0..=5).contains(&sample.y.floor()), "{sample:?}");
            }
        }
    }
}

#[test]
fn three_computers_placed_by_chained_offsets_resolve_with_negative_origins() {
    let on = |device: &str, name: &str| screen(device, name, 0, 0, 8, 4, SCALES[0], Rotation::Zero);
    let mut second = on("B", "b-one");
    second.scale = Scale::Two;
    second.buffer_width = 16;
    second.buffer_height = 8;
    let outputs = vec![on("A", "a-one"), second, on("C", "c-one")];
    let placements = vec![
        Placement { device: "C".into(), relative_to: Some("B".into()), offset: Some((8, -2)) },
        Placement { device: "A".into(), relative_to: None, offset: Some((-30, -40)) },
        Placement { device: "B".into(), relative_to: Some("A".into()), offset: Some((8, 1)) },
    ];
    let layout = Layout::resolve(outputs.clone(), &placements).unwrap();
    let origins: Vec<_> = layout.outputs().iter().map(|output| (output.device.as_str(), output.x, output.y)).collect();
    assert_eq!(origins, [("A", -30, -40), ("B", -22, -39), ("C", -14, -41)]);
    assert_eq!(layout.bounds(), Bounds { x: -30, y: -41, width: 24, height: 6 });
    // The order of the placements does not matter, and a missing parent is an error, not a guess.
    let mut shuffled = placements.clone();
    shuffled.reverse();
    assert_eq!(Layout::resolve(outputs.clone(), &shuffled).unwrap().bounds(), layout.bounds());
    let orphan = vec![placements[0].clone(), placements[2].clone()];
    assert_eq!(Layout::resolve(outputs.clone(), &orphan).unwrap_err(), Error::Ambiguous);
    // Placing the same computer twice, a computer with no screens, or none at all.
    let mut twice = placements.clone();
    twice.push(placements[1].clone());
    assert_eq!(Layout::resolve(outputs.clone(), &twice).unwrap_err(), Error::Duplicate);
    let mut stranger = placements.clone();
    stranger.push(Placement { device: "Z".into(), relative_to: None, offset: Some((0, 0)) });
    assert_eq!(Layout::resolve(outputs.clone(), &stranger).unwrap_err(), Error::Bounds);
    assert_eq!(Layout::resolve(outputs.clone(), &[]).unwrap_err(), Error::Bounds);
    // Offsets that push a screen out of the coordinate range, or overflow while adding up.
    let mut far = placements.clone();
    far[1].offset = Some((i64::MAX, 0));
    assert_eq!(Layout::resolve(outputs.clone(), &far).unwrap_err(), Error::Bounds);
    let mut sum = placements.clone();
    sum[1].offset = Some((i64::MAX, 0));
    sum[2].offset = Some((1, 0));
    assert_eq!(Layout::resolve(outputs.clone(), &sum).unwrap_err(), Error::Bounds);
    // Two computers placed on top of each other overlap.
    let mut stacked = placements;
    stacked[0].offset = Some((0, 0));
    stacked[2].offset = Some((0, 0));
    assert_eq!(Layout::resolve(outputs, &stacked).unwrap_err(), Error::Overlap);
}

#[test]
fn every_disagreement_between_buffer_and_logical_size_is_a_density_error() {
    for rotation in ROTATIONS {
        let ok = screen("A", "one", 0, 0, 16, 8, SCALES[1], rotation);
        assert!(Layout::new(vec![ok.clone()]).is_ok());
        for (extra_width, extra_height) in [(1u32, 0u32), (0, 1)] {
            let mut bad = ok.clone();
            bad.buffer_width += extra_width;
            bad.buffer_height += extra_height;
            assert_eq!(Layout::new(vec![bad]).unwrap_err(), Error::Density, "{rotation:?}");
        }
        // The right buffer for the wrong scale.
        let mut wrong_scale = ok.clone();
        wrong_scale.scale = Scale::Two;
        assert_eq!(Layout::new(vec![wrong_scale]).unwrap_err(), Error::Density);
        // The unrotated buffer for a rotated screen.
        let mut unrotated = ok;
        unrotated.rotation = if matches!(rotation, Rotation::Clockwise90 | Rotation::Clockwise270) { Rotation::Zero } else { Rotation::Clockwise90 };
        assert_eq!(Layout::new(vec![unrotated]).unwrap_err(), Error::Density);
    }
}

#[test]
fn names_counts_sizes_and_edges_are_bounded() {
    let base = screen("A", "one", 0, 0, 4, 4, SCALES[0], Rotation::Zero);
    let renamed = |device: &str, name: &str| Output { device: device.into(), screen: name.into(), ..base.clone() };
    for bad in [renamed("", "one"), renamed("A", ""), renamed(&"x".repeat(257), "one"), renamed("A", &"y".repeat(257)), renamed("A\n", "one"), renamed("A", "o\u{7}ne"), renamed("A", "one\0")] {
        assert_eq!(Layout::new(vec![bad]).unwrap_err(), Error::Bounds);
    }
    assert!(Layout::new(vec![renamed(&"x".repeat(256), &"y".repeat(256))]).is_ok());
    assert!(Layout::new(vec![renamed("ünï-cödé 画面", "モニター 1")]).is_ok());
    // Nine computers, seventeen screens, zero-size screens and edges of the coordinate range.
    let nine: Vec<_> = (0..9).map(|index| Output { device: format!("D{index}"), x: index * 10, ..base.clone() }).collect();
    assert_eq!(Layout::new(nine).unwrap_err(), Error::Bounds);
    let eight: Vec<_> = (0..8).map(|index| Output { device: format!("D{index}"), x: index * 10, ..base.clone() }).collect();
    assert_eq!(Layout::new(eight).unwrap().bounds().width, 74);
    let seventeen: Vec<_> = (0..17).map(|index| Output { screen: format!("S{index}"), x: index * 10, ..base.clone() }).collect();
    assert_eq!(Layout::new(seventeen).unwrap_err(), Error::Bounds);
    let sixteen: Vec<_> = (0..16).map(|index| Output { screen: format!("S{index}"), x: index * 10, ..base.clone() }).collect();
    assert!(Layout::new(sixteen).is_ok());
    assert_eq!(Layout::new(vec![Output { width: 0, buffer_width: 0, ..base.clone() }]).unwrap_err(), Error::Bounds);
    assert_eq!(Layout::new(vec![Output { height: 0, buffer_height: 0, ..base.clone() }]).unwrap_err(), Error::Bounds);
    assert_eq!(Layout::new(vec![Output { x: -1_000_001, ..base.clone() }]).unwrap_err(), Error::Bounds);
    assert!(Layout::new(vec![Output { x: -1_000_000, y: -1_000_000, ..base.clone() }]).is_ok());
    assert!(Layout::new(vec![Output { x: 999_996, y: 999_996, ..base.clone() }]).is_ok());
    assert_eq!(Layout::new(vec![Output { x: 999_997, ..base.clone() }]).unwrap_err(), Error::Bounds);
    assert_eq!(Layout::new(vec![Output { x: i64::MIN, ..base.clone() }]).unwrap_err(), Error::Bounds);
    assert_eq!(Layout::new(vec![]).unwrap_err(), Error::Bounds);
    // Screens that only touch do not overlap; one unit of overlap does.
    assert!(Layout::new(vec![base.clone(), Output { screen: "two".into(), x: 4, ..base.clone() }]).is_ok());
    assert_eq!(Layout::new(vec![base.clone(), Output { screen: "two".into(), x: 3, ..base.clone() }]).unwrap_err(), Error::Overlap);
    assert_eq!(Layout::new(vec![base.clone(), Output { screen: "two".into(), x: 0, y: 3, ..base }]).unwrap_err(), Error::Overlap);
}

#[test]
fn crop_refuses_wrong_pixel_data_instead_of_reading_it() {
    let layout = Layout::new(vec![screen("A", "one", 0, 0, 4, 4, SCALES[0], Rotation::Zero)]).unwrap();
    let source = Source::new(2, 2, 1, 1, Rotation::Zero).unwrap();
    let four = [[1, 2, 3, 255]; 4];
    assert!(layout.crop(0, source, Fit::Cover, &four, [0, 0, 0, 255]).is_ok());
    assert_eq!(layout.crop(0, source, Fit::Cover, &four[..3], [0, 0, 0, 255]).unwrap_err(), Error::Pixels);
    assert_eq!(layout.crop(0, source, Fit::Cover, &[[0; 4]; 5], [0, 0, 0, 255]).unwrap_err(), Error::Pixels);
    assert_eq!(layout.crop(0, source, Fit::Cover, &four, [0, 0, 0, 254]).unwrap_err(), Error::Pixels, "transparent padding");
    assert_eq!(layout.crop(1, source, Fit::Cover, &four, [0, 0, 0, 255]).unwrap_err(), Error::Address);
    // A picture over the 16 megapixel limit is refused by its dimensions, before its pixels are touched.
    let huge = Source::new(5000, 5000, 1, 1, Rotation::Zero).unwrap();
    assert_eq!(layout.crop(0, huge, Fit::Cover, &[], [0, 0, 0, 255]).unwrap_err(), Error::Pixels);
    // Source sizes and aspect ratios at and beyond their limits.
    assert!(Source::new(1_000_000, 1, 10_000, 10_000, Rotation::Zero).is_ok());
    for (width, height, numerator, denominator) in [(1_000_001, 1, 1, 1), (1, 1_000_001, 1, 1), (1, 1, 10_001, 1), (1, 1, 1, 10_001), (1, 1, 0, 1), (0, 0, 1, 1)] {
        assert_eq!(Source::new(width, height, numerator, denominator, Rotation::Zero).unwrap_err(), Error::Bounds, "{width} {height} {numerator} {denominator}");
    }
}

#[test]
fn pixel_addresses_are_exact() {
    let layout = Layout::new(vec![screen("A", "one", 0, 0, 8, 4, SCALES[3], Rotation::Clockwise90)]).unwrap();
    let output = &layout.outputs()[0];
    assert_eq!((output.buffer_width, output.buffer_height), (8, 16));
    let source = Source::new(1, 1, 1, 1, Rotation::Zero).unwrap();
    assert!(layout.sample(0, 7, 15, source, Fit::Stretch).is_ok());
    for (x, y) in [(8, 0), (0, 16), (u32::MAX, 0), (0, u32::MAX)] {
        assert_eq!(layout.sample(0, x, y, source, Fit::Stretch).unwrap_err(), Error::Address, "({x},{y})");
    }
    assert_eq!(layout.sample(usize::MAX, 0, 0, source, Fit::Stretch).unwrap_err(), Error::Address);
}
