use oklab::Oklab;

pub fn interpolate_oklab(start: Oklab, end: Oklab, t: f32) -> Oklab {
    Oklab { l: start.l + t * (end.l - start.l), a: start.a + t * (end.a - start.a), b: start.b + t * (end.b - start.b) }
}

pub fn interpolate_multiple<I>(colors: I, t: f32) -> Oklab
where
    I: IntoIterator<Item = Oklab> + Copy,
{
    let count = colors.into_iter().count();
    let u = t * count as f32;
    let i = u.floor() as usize;
    let color_1 = colors.into_iter().nth(i).unwrap();
    let color_2 = colors.into_iter().nth((i + 1) % count).unwrap();
    let local_t = u - i as f32;

    interpolate_oklab(color_1, color_2, local_t)
}
