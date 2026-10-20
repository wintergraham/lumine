#[path = "../crates/lumine-gpu/src/source_fill.rs"]
mod source_fill;
#[path = "../crates/lumine-gpu/src/source_skia154.rs"]
mod source_skia154;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let f: Vec<f32> = a.iter().map(|x| x.parse().unwrap()).collect();
    assert!(f.len() == 12 || f.len() == 21);
    let source = [f[5] as u8, f[6] as u8, f[7] as u8, f[8] as u8];
    let background = [f[9] as u8, f[10] as u8, f[11] as u8];
    let mut image = vec![[background[0], background[1], background[2], 255]; 192 * 160];
    let mask = if f.len() == 21 {
        let mask = source_skia154::spans([f[12], f[13], f[14], f[15]], f[16], [192, 160]).unwrap();
        let color = [f[17] as u8, f[18] as u8, f[19] as u8, f[20] as u8];
        for span in mask.iter() {
            for x in span.x..span.x + span.width {
                let pixel = &mut image[span.y as usize * 192 + x as usize];
                let rgb = source_fill::over_opaque_coverage(
                    color,
                    [pixel[0], pixel[1], pixel[2]],
                    span.coverage as u8,
                    span.blend == 1,
                );
                *pixel = [rgb[0], rgb[1], rgb[2], 255];
            }
        }
        Some(mask)
    } else {
        None
    };
    let spans = if mask.is_some() {
        source_skia154::spans_clipped(
            [f[0], f[1], f[2], f[3]],
            f[4],
            [192, 160],
            [
                f[12] as i32,
                f[13] as i32,
                (f[12] + f[14]) as i32,
                (f[13] + f[15]) as i32,
            ],
        )
        .unwrap()
    } else {
        source_skia154::spans([f[0], f[1], f[2], f[3]], f[4], [192, 160]).unwrap()
    };
    for span in spans.iter() {
        for x in span.x..span.x + span.width {
            let (coverage, two_pixels) = if let Some(mask) = &mask {
                let index = mask.partition_point(|s| s.y < span.y || (s.y == span.y && s.x <= x));
                let coverage = index
                    .checked_sub(1)
                    .and_then(|i| mask.get(i))
                    .filter(|s| s.y == span.y && x < s.x + s.width)
                    .map_or(0, |s| s.coverage);
                (((span.coverage * coverage + 127) / 255) as u8, false)
            } else {
                (span.coverage as u8, span.blend == 1)
            };
            if coverage == 0 {
                continue;
            }
            let pixel = &mut image[span.y as usize * 192 + x as usize];
            let rgb = source_fill::over_opaque_coverage(
                source,
                [pixel[0], pixel[1], pixel[2]],
                coverage,
                two_pixels,
            );
            *pixel = [rgb[0], rgb[1], rgb[2], 255];
        }
    }
    use std::io::Write;
    let mut o = std::io::stdout().lock();
    for pixel in image {
        o.write_all(&pixel).unwrap();
    }
}
