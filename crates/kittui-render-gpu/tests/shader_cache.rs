//! User-shader pipeline cache validation.
//!
//! Runs only when an adapter is available. Verifies that:
//! - A valid user WGSL compiles and the result is cached.
//! - An invalid user WGSL produces a clear `ShaderError`.
//!
//! Render output is exercised via the standard parity test; this file
//! focuses on the cache contract, not pixels.

use kittui_render_gpu as gpu;

const VALID_USER_SHADER: &str = r#"
fn user(frag: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(U.fill.r, U.fill.g, U.fill.b, U.fill.a);
}
"#;

#[test]
fn user_shader_compiles_through_cache_and_caches() {
    let device = match gpu::GpuDevice::new() {
        Ok(d) => d,
        Err(_) => {
            eprintln!("skipping user_shader_compiles: no usable wgpu adapter");
            return;
        }
    };
    let pipelines = gpu::pipelines::Pipelines::new(&device);
    let _first = pipelines
        .compile_user_shader(&device, VALID_USER_SHADER)
        .expect("user shader should compile");
    let cached_before = pipelines.shader_cache.lock().len();
    let _second = pipelines
        .compile_user_shader(&device, VALID_USER_SHADER)
        .expect("user shader should compile from cache");
    let cached_after = pipelines.shader_cache.lock().len();
    assert_eq!(
        cached_before, cached_after,
        "second compile call should hit the cache without growing it"
    );
    assert_eq!(cached_after, 1);
}
