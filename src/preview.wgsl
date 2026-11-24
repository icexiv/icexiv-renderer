struct Camera { view_projection: mat4x4<f32>, light: vec4<f32>, eye: vec4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var base_color: texture_2d<f32>;
@group(1) @binding(1) var base_sampler: sampler;
struct Vertex { @builtin(position) clip: vec4<f32>, @location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32> }
@vertex fn vs(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>) -> Vertex {
    return Vertex(camera.view_projection * vec4f(position, 1), position, normal, uv);
}
// Studio glass approximation: transmit the already-drawn surface, with a
// stronger reflection at grazing angles and a small reflected key light.
@fragment fn fs_glass(v: Vertex) -> @location(0) vec4f {
    let view = normalize(camera.eye.xyz - v.pos);
    let face = cross(dpdy(v.pos), dpdx(v.pos));
    let normal = normalize(select(face, v.normal, dot(v.normal, v.normal) > 0.01));
    let n = select(-normal, normal, dot(normal, view) > 0.0);
    let rim = pow(1.0 - clamp(dot(view, n), 0.0, 1.0), 5.0);
    let half_vector = normalize(view + camera.light.xyz);
    let highlight = pow(max(dot(n, half_vector), 0.0), 96.0);
    let sky = clamp(reflect(-view, n).y * 0.5 + 0.5, 0.0, 1.0);
    let tint = textureSample(base_color, base_sampler, v.uv).rgb;
    let reflection = mix(vec3f(0.10, 0.13, 0.15), vec3f(0.75, 0.87, 0.96), sky);
    return vec4f(mix(reflection * tint, vec3f(1.0), highlight),
                 clamp(0.08 + 0.60 * rim + 0.82 * highlight, 0.0, 1.0));
}
@fragment fn fs(v: Vertex) -> @location(0) vec4f {
    let face = cross(dpdy(v.pos), dpdx(v.pos));
    // Double-sided cloth can contain coincident front/back triangles. Orient
    // authored normals to the visible side for a stable neutral preview.
    let oriented = select(v.normal, -v.normal, dot(v.normal, face) < 0.0);
    let n = normalize(select(face, oriented, dot(oriented, oriented) > 0.01));
    let diffuse = max(dot(n, camera.light.xyz), 0.0);
    let sky = n.y * 0.5 + 0.5;
    let fill = mix(vec3f(0.21, 0.20, 0.19), vec3f(0.42, 0.45, 0.48), sky);
    return vec4f(textureSample(base_color, base_sampler, v.uv).rgb * (fill + 0.72 * diffuse), 1.0);
}
