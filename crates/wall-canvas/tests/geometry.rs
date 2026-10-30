use one_mk_wall_canvas::*;

fn output(id: &str, x: i64, y: i64, width: u32, height: u32) -> Output {
    Output { device: "A".into(), screen: id.into(), x, y, width, height,
        scale: Scale::One, rotation: Rotation::Zero, buffer_width: width, buffer_height: height }
}

#[test]
fn reserved_negative_origin_and_bezel_golden() {
    let first = output("one", 0, 0, 1920, 1080);
    let mut reserved = output("two", 2020, -120, 1280, 1024);
    reserved.device = "offline-B".into();
    let layout = Layout::new(vec![first, reserved]).unwrap();
    assert_eq!(layout.bounds(), Bounds { x: 0, y: -120, width: 3300, height: 1200 });
    let source = Source::new(3300, 1200, 1, 1, Rotation::Zero).unwrap();
    assert_eq!(layout.sample(0, 0, 0, source, Fit::Stretch).unwrap(), Some(Sample { x: Rational::new(1, 2), y: Rational::new(241, 2) }));
    assert_eq!(layout.sample(1, 0, 0, source, Fit::Tile).unwrap(), Some(Sample { x: Rational::new(4041, 2), y: Rational::new(1, 2) }));
}

#[test]
fn reject_mirrors_overlap_duplicates_overflow_and_huge_layouts() {
    let first = output("one", 0, 0, 4, 4);
    assert_eq!(Layout::new(vec![first.clone(), output("two", 0, 0, 4, 4)]).unwrap_err(), Error::Overlap);
    assert_eq!(Layout::new(vec![first.clone(), first.clone()]).unwrap_err(), Error::Duplicate);
    assert_eq!(Layout::new(vec![output("one", i64::MAX, 0, 4, 4)]).unwrap_err(), Error::Bounds);
    assert_eq!(Layout::new(vec![output("one", 999_999, 0, 4, 4)]).unwrap_err(), Error::Bounds);
    assert_eq!(Layout::new(vec![first; 17]).unwrap_err(), Error::Bounds);
}

#[test]
fn fits_and_source_aspect_rotation_goldens() {
    let layout = Layout::new(vec![output("one", 0, 0, 4, 4)]).unwrap();
    let pixels = vec![[1,0,0,255], [2,0,0,255]];
    let source = Source::new(2, 1, 1, 1, Rotation::Zero).unwrap();
    assert_eq!(layout.sample(0,0,0,source,Fit::Cover).unwrap(),Some(Sample { x: Rational::new(5,8),y: Rational::new(1,8) }));
    let raster = |fit| layout.crop(0, source, fit, &pixels, [9,0,0,255]).unwrap().iter().map(|pixel| pixel[0]).collect::<Vec<_>>();
    assert_eq!(raster(Fit::Contain), vec![9,9,9,9,1,1,2,2,1,1,2,2,9,9,9,9]);
    assert_eq!(raster(Fit::Cover), vec![1,1,2,2,1,1,2,2,1,1,2,2,1,1,2,2]);
    assert_eq!(raster(Fit::Tile), vec![1,2,1,2,1,2,1,2,1,2,1,2,1,2,1,2]);
    let portrait = Source::new(2, 1, 2, 1, Rotation::Clockwise90).unwrap();
    let sample = layout.sample(0, 0, 0, portrait, Fit::Stretch).unwrap().unwrap();
    assert_eq!(sample, Sample { x: Rational::new(1,4), y: Rational::new(7,8) });
}

#[test]
fn adjacent_slices_equal_whole_canvas_for_every_fit_and_size() {
    for width in 1..=16 {
        for height in 1..=12 {
            let split = Layout::new(vec![output("left", -19, -7, width, height), output("right", -19 + i64::from(width), -7, width, height)]).unwrap();
            let whole = Layout::new(vec![output("whole", -19, -7, width * 2, height)]).unwrap();
            let source = Source::new(7, 5, 5, 4, Rotation::Clockwise270).unwrap();
            for fit in [Fit::Cover, Fit::Contain, Fit::Stretch, Fit::Tile] {
                for y in 0..height {
                    for x in 0..width * 2 {
                        let actual = split.sample(usize::from(x >= width), x % width, y, source, fit).unwrap();
                        assert_eq!(actual, whole.sample(0, x, y, source, fit).unwrap());
                        assert_eq!(actual, split.sample(usize::from(x >= width), x % width, y, source, fit).unwrap());
                    }
                }
            }
        }
    }
}

#[test]
fn output_density_and_rotation_do_not_change_canvas_extent() {
    for (scale, numerator) in [(Scale::One,4), (Scale::FiveFourths,5), (Scale::ThreeHalves,6), (Scale::Two,8)] {
        for rotation in [Rotation::Zero, Rotation::Clockwise90, Rotation::Half, Rotation::Clockwise270] {
            let mut monitor = output("one", -8, -4, 8, 4);
            monitor.scale = scale;
            monitor.rotation = rotation;
            let (width, height) = if matches!(rotation, Rotation::Clockwise90 | Rotation::Clockwise270) { (4,8) } else { (8,4) };
            monitor.buffer_width = width * numerator / 4;
            monitor.buffer_height = height * numerator / 4;
            let layout = Layout::new(vec![monitor.clone()]).unwrap();
            assert_eq!(layout.bounds().width, 8);
            let source = Source::new(8,4,1,1,Rotation::Zero).unwrap();
            let mut owners = [0;32];
            for y in 0..monitor.buffer_height {
                for x in 0..monitor.buffer_width {
                    let sample = layout.sample(0,x,y,source,Fit::Stretch).unwrap().unwrap();
                    let index = sample.y.floor() * 8 + sample.x.floor();
                    owners[index as usize] += 1;
                }
            }
            assert!(owners.iter().all(|count| *count > 0));
        }
    }
}

#[test]
fn invalid_sources_and_pixel_addresses_are_errors_not_panics() {
    assert!(Source::new(0,1,1,1,Rotation::Zero).is_err());
    assert!(Source::new(1,1,1,0,Rotation::Zero).is_err());
    let layout = Layout::new(vec![output("one",0,0,4,4)]).unwrap();
    let source = Source::new(1,1,1,1,Rotation::Zero).unwrap();
    assert!(layout.sample(1,0,0,source,Fit::Cover).is_err());
    assert!(layout.sample(0,4,0,source,Fit::Cover).is_err());
    assert!(layout.crop(0,source,Fit::Cover,&[],[0;4]).is_err());
}

#[test]
fn union_coverage_preserves_holes_and_never_double_owns_a_point() {
    for gap in 0..8 {
        let layout = Layout::new(vec![output("left",-8,-4,8,8),output("right",gap,-4,8,8)]).unwrap();
        let mut covered = 0;
        for y in -4..4 {
            for x in -8..gap+8 {
                let owners = layout.outputs().iter().filter(|monitor| x >= monitor.x && x < monitor.x+i64::from(monitor.width)
                    && y >= monitor.y && y < monitor.y+i64::from(monitor.height)).count();
                assert_eq!(owners,usize::from(x < 0 || x >= gap));
                covered += owners;
            }
        }
        assert_eq!(covered,128);
    }
}

#[test]
fn extreme_valid_aspect_ratios_and_edges_do_not_overflow() {
    let layout = Layout::new(vec![output("left",-1_000_000,-1_000_000,4,4),output("right",999_996,999_996,4,4)]).unwrap();
    for numerator in [1,9999,10000] {
        for denominator in [1,9999,10000] {
            for rotation in [Rotation::Zero,Rotation::Clockwise90,Rotation::Half,Rotation::Clockwise270] {
                let source = Source::new(999_999,1_000_000,numerator,denominator,rotation).unwrap();
                for fit in [Fit::Cover,Fit::Contain,Fit::Stretch,Fit::Tile] {
                    for monitor in 0..2 {
                        if let Some(sample) = layout.sample(monitor,3,3,source,fit).unwrap() {
                            assert!((0..999_999).contains(&sample.x.floor()));
                            assert!((0..1_000_000).contains(&sample.y.floor()));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn rotated_tile_exact_edge_uses_last_texel_without_index_overflow() {
    let mut monitor = output("one",0,0,4,4);
    monitor.scale = Scale::FiveFourths;
    monitor.buffer_width = 5; monitor.buffer_height = 5;
    let layout = Layout::new(vec![monitor]).unwrap();
    let source = Source::new(1,1,2,5,Rotation::Half).unwrap();
    assert_eq!(layout.crop(0,source,Fit::Tile,&[[42,0,0,255]],[0,0,0,255]).unwrap(),vec![[42,0,0,255];25]);
}

#[test]
fn explicit_computer_offsets_resolve_but_ambiguous_or_inconsistent_cycles_do_not() {
    let mut right = output("two",0,0,4,4); right.device = "B".into();
    let outputs = vec![output("one",0,0,4,4),right];
    let placements = vec![Placement { device: "A".into(),relative_to: None,offset: Some((-8,-4)) },
        Placement { device: "B".into(),relative_to: Some("A".into()),offset: Some((12,0)) }];
    let layout = Layout::resolve(outputs.clone(),&placements).unwrap();
    assert_eq!(layout.bounds(),Bounds { x: -8,y: -4,width: 16,height: 4 });
    let mut ambiguous = placements.clone(); ambiguous[1].offset = None;
    assert_eq!(Layout::resolve(outputs.clone(),&ambiguous).unwrap_err(),Error::Ambiguous);
    let mut cycle = placements; cycle[0].relative_to = Some("B".into()); cycle[0].offset = Some((-12,0));
    assert_eq!(Layout::resolve(outputs.clone(),&cycle).unwrap_err(),Error::Ambiguous);
    cycle[0].offset = Some((-11,0));
    assert_eq!(Layout::resolve(outputs,&cycle).unwrap_err(),Error::Inconsistent);
}
