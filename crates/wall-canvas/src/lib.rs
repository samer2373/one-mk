//! Exact, I/O-free geometry for a committed canvas.

use std::collections::BTreeSet;

const EDGE_LIMIT: i64 = 1_000_000;
const PIXEL_LIMIT: u64 = 16_777_216;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Bounds, Duplicate, Overlap, Density, Address, Pixels, Ambiguous, Inconsistent }

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Bounds => "canvas exceeds 8 devices, 16 outputs, ID or coordinate bounds",
            Self::Duplicate => "duplicate computer/screen identity",
            Self::Overlap => "mirrored or overlapping outputs are not supported; fix the layout preview",
            Self::Density => "physical buffer does not match the oriented logical size and scale",
            Self::Address => "output or pixel is outside the committed layout",
            Self::Pixels => "reference image has invalid dimensions or exceeds 16 megapixels",
            Self::Ambiguous => "computer placement needs explicit offsets to an anchored computer; side-only or unanchored cycles are ambiguous",
            Self::Inconsistent => "relative computer placements form an inconsistent cycle",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale { One, FiveFourths, ThreeHalves, Two }
impl Scale {
    fn quarters(self) -> i128 {
        match self { Self::One => 4, Self::FiveFourths => 5, Self::ThreeHalves => 6, Self::Two => 8 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation { Zero, Clockwise90, Half, Clockwise270 }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub device: String,
    pub screen: String,
    pub x: i64,
    pub y: i64,
    pub width: u32,
    pub height: u32,
    pub scale: Scale,
    pub rotation: Rotation,
    pub buffer_width: u32,
    pub buffer_height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds { pub x: i64, pub y: i64, pub width: u32, pub height: u32 }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout { outputs: Vec<Output>, bounds: Bounds }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement { pub device: String, pub relative_to: Option<String>, pub offset: Option<(i64,i64)> }

impl Layout {
    pub fn resolve(mut outputs: Vec<Output>, placements: &[Placement]) -> Result<Self,Error> {
        if outputs.len() > 16 || placements.is_empty() || placements.len() > 8 { return Err(Error::Bounds); }
        let mut by_device = std::collections::BTreeMap::new();
        for placement in placements {
            if placement.device.is_empty() || placement.device.len() > 256 || placement.relative_to.as_ref().is_some_and(|parent| parent.len() > 256)
                || !outputs.iter().any(|output| output.device == placement.device) { return Err(Error::Bounds); }
            if by_device.insert(&placement.device,placement).is_some() { return Err(Error::Duplicate); }
        }
        for output in &mut outputs {
            let mut current = &output.device;
            let (mut total_x,mut total_y) = (0i64,0i64);
            let mut visited = std::collections::BTreeMap::new();
            loop {
                if let Some((previous_x,previous_y)) = visited.insert(current,(total_x,total_y)) {
                    return Err(if (previous_x,previous_y) == (total_x,total_y) { Error::Ambiguous } else { Error::Inconsistent });
                }
                let placement = by_device.get(current).ok_or(Error::Ambiguous)?;
                let (offset_x,offset_y) = placement.offset.ok_or(Error::Ambiguous)?;
                total_x = total_x.checked_add(offset_x).ok_or(Error::Bounds)?;
                total_y = total_y.checked_add(offset_y).ok_or(Error::Bounds)?;
                if let Some(parent) = &placement.relative_to { current = parent; }
                else { break; }
            }
            output.x = output.x.checked_add(total_x).ok_or(Error::Bounds)?;
            output.y = output.y.checked_add(total_y).ok_or(Error::Bounds)?;
        }
        Self::new(outputs)
    }

    pub fn new(outputs: Vec<Output>) -> Result<Self, Error> {
        if outputs.is_empty() || outputs.len() > 16 { return Err(Error::Bounds); }
        let mut devices = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let (mut left, mut top, mut right, mut bottom) = (EDGE_LIMIT, EDGE_LIMIT, -EDGE_LIMIT, -EDGE_LIMIT);
        for (index, output) in outputs.iter().enumerate() {
            if [&output.device, &output.screen].iter().any(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control)) {
                return Err(Error::Bounds);
            }
            devices.insert(&output.device);
            if devices.len() > 8 { return Err(Error::Bounds); }
            if !identities.insert((&output.device, &output.screen)) { return Err(Error::Duplicate); }
            let far_x = output.x.checked_add(i64::from(output.width)).ok_or(Error::Bounds)?;
            let far_y = output.y.checked_add(i64::from(output.height)).ok_or(Error::Bounds)?;
            if output.width == 0 || output.height == 0 || [output.x, output.y, far_x, far_y].iter().any(|edge| !(-EDGE_LIMIT..=EDGE_LIMIT).contains(edge)) {
                return Err(Error::Bounds);
            }
            let (buffer_width, buffer_height) = match output.rotation {
                Rotation::Clockwise90 | Rotation::Clockwise270 => (output.buffer_height, output.buffer_width),
                _ => (output.buffer_width, output.buffer_height),
            };
            if i128::from(buffer_width) * 4 != i128::from(output.width) * output.scale.quarters()
                || i128::from(buffer_height) * 4 != i128::from(output.height) * output.scale.quarters() {
                return Err(Error::Density);
            }
            for previous in &outputs[..index] {
                if output.x < previous.x + i64::from(previous.width) && previous.x < far_x
                    && output.y < previous.y + i64::from(previous.height) && previous.y < far_y {
                    return Err(Error::Overlap);
                }
            }
            left = left.min(output.x); top = top.min(output.y);
            right = right.max(far_x); bottom = bottom.max(far_y);
        }
        Ok(Self { outputs, bounds: Bounds { x: left, y: top, width: (right-left) as u32, height: (bottom-top) as u32 } })
    }

    pub fn bounds(&self) -> Bounds { self.bounds }
    pub fn outputs(&self) -> &[Output] { &self.outputs }

    pub fn sample(&self, monitor: usize, x: u32, y: u32, source: Source, fit: Fit) -> Result<Option<Sample>, Error> {
        let output = self.outputs.get(monitor).ok_or(Error::Address)?;
        if x >= output.buffer_width || y >= output.buffer_height { return Err(Error::Address); }
        let density = output.scale.quarters();
        let local_x = Rational::new((i128::from(x) * 2 + 1) * 2, density);
        let local_y = Rational::new((i128::from(y) * 2 + 1) * 2, density);
        let buffer_width = Rational::new(i128::from(output.buffer_width)*4, density);
        let buffer_height = Rational::new(i128::from(output.buffer_height)*4, density);
        let (local_x, local_y) = rotate(local_x, local_y, buffer_width, buffer_height, output.rotation);
        let canvas_x = local_x.add(Rational::integer(i128::from(output.x-self.bounds.x)));
        let canvas_y = local_y.add(Rational::integer(i128::from(output.y-self.bounds.y)));
        let (display_width, display_height) = source.display_size();
        let horizontal = Rational::integer(i128::from(self.bounds.width)).div(display_width);
        let vertical = Rational::integer(i128::from(self.bounds.height)).div(display_height);
        let (scale_x, scale_y) = match fit {
            Fit::Cover => { let scale = horizontal.max(vertical); (scale,scale) }
            Fit::Contain => { let scale = horizontal.min(vertical); (scale,scale) }
            Fit::Stretch => (horizontal,vertical),
            Fit::Tile => (Rational::integer(1),Rational::integer(1)),
        };
        let (display_x, display_y) = if fit == Fit::Tile {
            (canvas_x.modulo(display_width), canvas_y.modulo(display_height))
        } else {
            let offset_x = Rational::integer(i128::from(self.bounds.width)).sub(display_width.mul(scale_x)).div(Rational::integer(2));
            let offset_y = Rational::integer(i128::from(self.bounds.height)).sub(display_height.mul(scale_y)).div(Rational::integer(2));
            (canvas_x.sub(offset_x).div(scale_x), canvas_y.sub(offset_y).div(scale_y))
        };
        if display_x < Rational::integer(0) || display_y < Rational::integer(0) || display_x >= display_width || display_y >= display_height {
            return Ok(None);
        }
        let (aspect_width, raw_height) = (source.aspect_width(), Rational::integer(i128::from(source.height)));
        let (raw_x, raw_y) = match source.rotation {
            Rotation::Zero => (display_x,display_y),
            Rotation::Clockwise90 => (display_y,raw_height.sub(display_x)),
            Rotation::Half => (aspect_width.sub(display_x),raw_height.sub(display_y)),
            Rotation::Clockwise270 => (aspect_width.sub(display_y),display_x),
        };
        Ok(Some(Sample { x: raw_x.div(source.aspect), y: raw_y }))
    }

    pub fn crop(&self, monitor: usize, source: Source, fit: Fit, pixels: &[[u8;4]], padding: [u8;4]) -> Result<Vec<[u8;4]>, Error> {
        let output = self.outputs.get(monitor).ok_or(Error::Address)?;
        let count = u64::from(output.buffer_width)*u64::from(output.buffer_height);
        let input_count = u64::from(source.width)*u64::from(source.height);
        if count > PIXEL_LIMIT || input_count > PIXEL_LIMIT || input_count != pixels.len() as u64 || padding[3] != 255 { return Err(Error::Pixels); }
        let mut result = Vec::with_capacity(count as usize);
        for y in 0..output.buffer_height {
            for x in 0..output.buffer_width {
                result.push(match self.sample(monitor,x,y,source,fit)? {
                    Some(sample) => {
                        let source_x = sample.x.floor().min(i128::from(source.width)-1);
                        let source_y = sample.y.floor().min(i128::from(source.height)-1);
                        pixels[(source_y*i128::from(source.width)+source_x) as usize]
                    }
                    None => padding,
                });
            }
        }
        Ok(result)
    }
}

fn rotate(x: Rational, y: Rational, width: Rational, height: Rational, rotation: Rotation) -> (Rational,Rational) {
    match rotation {
        Rotation::Zero => (x,y), Rotation::Clockwise90 => (height.sub(y),x),
        Rotation::Half => (width.sub(x),height.sub(y)), Rotation::Clockwise270 => (y,width.sub(x)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit { Cover, Contain, Stretch, Tile }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source { width: u32, height: u32, aspect: Rational, rotation: Rotation }
impl Source {
    pub fn new(width: u32, height: u32, sar_numerator: u32, sar_denominator: u32, rotation: Rotation) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > 1_000_000 || height > 1_000_000
            || !(1..=10_000).contains(&sar_numerator) || !(1..=10_000).contains(&sar_denominator) { return Err(Error::Bounds); }
        Ok(Self { width,height,aspect: Rational::new(i128::from(sar_numerator),i128::from(sar_denominator)),rotation })
    }
    fn aspect_width(self) -> Rational { Rational::integer(i128::from(self.width)).mul(self.aspect) }
    fn display_size(self) -> (Rational,Rational) {
        let dimensions = (self.aspect_width(),Rational::integer(i128::from(self.height)));
        match self.rotation { Rotation::Clockwise90 | Rotation::Clockwise270 => (dimensions.1,dimensions.0), _ => dimensions }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample { pub x: Rational, pub y: Rational }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rational { numerator: i128, denominator: i128 }
impl Rational {
    pub fn new(numerator: i128, denominator: i128) -> Self {
        assert!(denominator > 0);
        let (mut left,mut right) = (numerator.unsigned_abs(),denominator as u128);
        while right != 0 { (left,right) = (right,left % right); }
        let divisor = left as i128;
        Self { numerator: numerator/divisor, denominator: denominator/divisor }
    }
    pub fn floor(self) -> i128 { self.numerator.div_euclid(self.denominator) }
    pub fn parts(self) -> (i128,i128) { (self.numerator,self.denominator) }
    fn integer(value: i128) -> Self { Self::new(value,1) }
    fn add(self, other: Self) -> Self { Self::new(self.numerator*other.denominator+other.numerator*self.denominator,self.denominator*other.denominator) }
    fn sub(self, other: Self) -> Self { Self::new(self.numerator*other.denominator-other.numerator*self.denominator,self.denominator*other.denominator) }
    fn mul(self, other: Self) -> Self { Self::new(self.numerator*other.numerator,self.denominator*other.denominator) }
    fn div(self, other: Self) -> Self { Self::new(self.numerator*other.denominator,self.denominator*other.numerator) }
    fn modulo(self, period: Self) -> Self { self.sub(period.mul(Self::integer(self.div(period).floor()))) }
}
impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) }
}
impl Ord for Rational {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering { (self.numerator*other.denominator).cmp(&(other.numerator*self.denominator)) }
}
