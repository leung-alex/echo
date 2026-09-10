// Perspective-correct panel, analytic shadow and optional lower-edge reflection.
// Parameters are logical DIP; input/output textures use premultiplied RGBA.
struct Params {
    viewport: vec4<f32>, // stage W/H, perspective distance, corner radius
    panel: vec4<f32>, // panel W/H, scale, opacity
    pose: vec4<f32>, // x/y/z, yaw radians
    appearance: vec4<f32>, // shade, edge feather, reflection alpha, reflection ratio
    shadow: vec4<f32>, // softness, expansion margin, vertical offset, alpha
    reflection: vec4<f32>, // gap, AA geometry padding px, AA coverage width px, camera offset X
    camera: vec4<f32>, // camera offset Y, reserved
};
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var panel_image: texture_2d<f32>;
@group(0) @binding(2) var panel_sampler: sampler;
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) @interpolate(flat) kind: u32,
};
@vertex fn vs_main(@builtin(vertex_index) i: u32, @builtin(instance_index) kind: u32) -> VOut {
    let uvs = array<vec2<f32>,6>(vec2(0.,0.),vec2(1.,0.),vec2(0.,1.),
                              vec2(0.,1.),vec2(1.,0.),vec2(1.,1.));
    let uv = uvs[i];
    var local = (uv-vec2(0.5))*p.panel.xy;
    // The rasterizer must cover BOTH sides of the analytic silhouette.
    if (kind == 0u) { local = (uv-vec2(0.5))*(p.panel.xy+vec2(2.*p.reflection.y*p.appearance.y)); }
    if (kind == 1u) { local = (uv-vec2(0.5))*(p.panel.xy+vec2(2.*p.shadow.y)); }
    if (kind == 2u) { local.y = p.panel.y*0.5 + p.reflection.x + uv.y*p.panel.y*p.appearance.w; }
    var projected = local*p.panel.z;
    if (kind == 1u) { projected.y += p.shadow.z; }
    let c = cos(p.pose.w); let s = sin(p.pose.w);
    let x = projected.x*c + p.pose.x;
    let y = projected.y + p.pose.y;
    let z = -projected.x*s + p.pose.z;
    let w = max(0.01, 1.-z/p.viewport.z);
    var out: VOut;
    out.clip = vec4(2.*(x+p.reflection.w*w)/p.viewport.x,-2.*(y+p.camera.x*w)/p.viewport.y,0.5*w,w);
    out.uv=select(uv,local/p.panel.xy+vec2(0.5),kind==0u);
    out.local=local;out.kind=kind;return out;
}
fn rounded_distance(pixel:vec2<f32>) -> f32 {
    let q = abs(pixel)-(p.panel.xy*0.5-vec2(p.viewport.w));
    return length(max(q,vec2(0.)))+min(max(q.x,q.y),0.)-p.viewport.w;
}
// Four bounded taps integrate the projected output-pixel footprint. They sample
// a higher-density cached card, not a larger whole-window render target.
fn filtered_panel(uv:vec2<f32>,dx:vec2<f32>,dy:vec2<f32>) -> vec4<f32> {
    let size=vec2<f32>(textureDimensions(panel_image));
    let span=max(length(dx*size),length(dy*size));
    let amount=clamp(span-1.,0.,1.);
    let x=dx*(0.25*amount);let y=dy*(0.25*amount);
    let a=textureSampleLevel(panel_image,panel_sampler,uv-x-y,0.);
    let b=textureSampleLevel(panel_image,panel_sampler,uv+x-y,0.);
    let c=textureSampleLevel(panel_image,panel_sampler,uv-x+y,0.);
    let d=textureSampleLevel(panel_image,panel_sampler,uv+x+y,0.);
    return (a+b+c+d)*0.25;
}
@fragment fn fs_main(v: VOut) -> @location(0) vec4<f32> {
    // Derivatives are evaluated before instance-kind branches.
    let dx=dpdx(v.uv);let dy=dpdy(v.uv);
    let distance=rounded_distance(v.local);
    let pixel_width=max(fwidth(distance)*p.reflection.z,0.0001);
    if (v.kind == 1u) {
        let dist=max(rounded_distance(v.local),0.);
        let softness=max(p.shadow.x,1.);
        let alpha=exp(-dist*dist/(softness*softness))*p.shadow.w*p.panel.w;
        return vec4(0.,0.,0.,alpha);
    }
    if (v.kind == 2u) {
        let uv=vec2(v.uv.x,1.-v.uv.y*p.appearance.w);
        let src=textureSampleLevel(panel_image,panel_sampler,uv,0.);
        let edge=1.-smoothstep(-1.,1.,rounded_distance((uv-vec2(0.5))*p.panel.xy));
        let alpha=p.appearance.z*p.panel.w*(1.-v.uv.y)*(1.-v.uv.y)*edge;
        return vec4(src.rgb*alpha,src.a*alpha);
    }
    let edge=clamp(0.5-distance/pixel_width,0.,1.);
    let src=filtered_panel(v.uv,dx,dy);
    let alpha=p.panel.w*edge;
    return vec4(src.rgb*(1.-p.appearance.x)*alpha,src.a*alpha);
}
