use crate::Side;

pub fn next_side(occupied: impl IntoIterator<Item = Side>) -> Option<Side> {
    let occupied: Vec<_> = occupied.into_iter().collect();
    [Side::Left, Side::Right, Side::Top, Side::Bottom].into_iter().find(|side| !occupied.contains(side))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_a_free_side_without_moving_existing_computers() {
        assert_eq!(next_side([]), Some(Side::Left));
        assert_eq!(next_side([Side::Left]), Some(Side::Right));
        assert_eq!(next_side([Side::Left, Side::Right]), Some(Side::Top));
        assert_eq!(next_side([Side::Bottom, Side::Left, Side::Top, Side::Right]), None);
        assert_eq!(next_side([Side::Right, Side::Right]), Some(Side::Left));
    }
}
