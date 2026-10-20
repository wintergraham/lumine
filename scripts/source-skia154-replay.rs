#[path = "../crates/lumine-gpu/src/source_fill.rs"]
mod source_fill;
#[path = "../crates/lumine-gpu/src/source_skia154.rs"]
mod source_skia154;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let f: Vec<f32> = a.iter().map(|x| x.parse().unwrap()).collect();
    let spans = source_skia154::spans([f[0], f[1], f[2], f[3]], f[4], [192, 160]).unwrap();
    let source = [f[5] as u8, f[6] as u8, f[7] as u8, f[8] as u8];
    let background = [f[9] as u8, f[10] as u8, f[11] as u8];
    let mut image = vec![[background[0], background[1], background[2], 255]; 192 * 160];
    for span in spans.iter() {
        for x in span.x..span.x + span.width {
            let pixel = &mut image[span.y as usize * 192 + x as usize];
            let rgb = source_fill::over_opaque_coverage(
                source,
                [pixel[0], pixel[1], pixel[2]],
                span.coverage as u8,
                span.blend == 1,
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
