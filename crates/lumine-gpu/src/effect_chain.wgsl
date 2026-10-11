@group(0) @binding(0) var<storage, read> p: array<f32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var source: texture_2d<f32>;
@group(0) @binding(3) var linear: sampler;
const BAYER = array<f32, 16>(0., 8., 2., 10., 12., 4., 14., 6., 3., 11., 1., 9., 15., 7., 13., 5.);

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let size = textureDimensions(output);
    if any(invocation.xy >= size) { return; }
    let xy = invocation.xy;
    var value = textureLoad(source, vec2<i32>(xy), 0);
    if p[0] == 2. {
        let cell = xy / u32(p[1]) % vec2<u32>(4);
        let threshold = (BAYER[cell.y * 4u + cell.x] + 0.5) / 16.;
        let steps = p[2] - 1.;
        let straight = clamp(value.rgb / max(value.a, 1e-8), vec3(0.), vec3(1.));
        value = vec4(floor(straight * steps + threshold) / steps * value.a, value.a);
    } else {
        let dimensions = vec2<f32>(size);
        let center = vec2<f32>(xy) + 0.5;
        var axis = vec2(1., 0.);
        if p[0] == 1. { axis = vec2(0., 1.); }
        value *= p[5];
        for (var tap = 1u; tap < u32(p[3]); tap++) {
            let offset = axis * p[4u + tap * 2u];
            let weight = p[5u + tap * 2u];
            let a = clamp(center - offset, vec2(0.5), dimensions - 0.5) / dimensions;
            let b = clamp(center + offset, vec2(0.5), dimensions - 0.5) / dimensions;
            value += (textureSampleLevel(source, linear, a, 0.) + textureSampleLevel(source, linear, b, 0.)) * weight;
        }
    }
    textureStore(output, vec2<i32>(xy), value);
}
