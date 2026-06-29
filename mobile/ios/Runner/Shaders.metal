#include <metal_stdlib>
using namespace metal;

struct VertexInput {
    float2 position [[attribute(0)]];
};

struct VertexOutput {
    float4 position [[position]];
    float2 texcoord;
};

struct Uniforms {
    float2 bl;
    float2 br;
    float2 tl;
    float2 tr;
    float2 screenSize;
    float opacity;
    float layerMode;
    float valStops[8];
    float4 colStops[8];
};

vertex VertexOutput vertexMain(VertexInput in [[stage_in]],
                               constant Uniforms& uniforms [[buffer(0)]]) {
    VertexOutput out;
    
    // Bilinear interpolation for coordinates mapping
    float2 screen_pos = mix(
        mix(uniforms.bl, uniforms.br, in.position.x),
        mix(uniforms.tl, uniforms.tr, in.position.x),
        in.position.y
    );
    
    // Convert screen coordinates to Normalized Device Coordinates (NDC)
    float ndc_x = (screen_pos.x / uniforms.screenSize.x) * 2.0 - 1.0;
    float ndc_y = 1.0 - (screen_pos.y / uniforms.screenSize.y) * 2.0; // Invert Y
    
    out.position = float4(ndc_x, ndc_y, 0.0, 1.0);
    out.texcoord = in.position;
    return out;
}

float4 getColor(float val, constant Uniforms& uniforms) {
    if (val < uniforms.valStops[0]) return float4(0.0);
    for (int i = 0; i < 7; i++) {
        if (val <= uniforms.valStops[i + 1]) {
            float t = (val - uniforms.valStops[i]) / (uniforms.valStops[i + 1] - uniforms.valStops[i]);
            return mix(uniforms.colStops[i], uniforms.colStops[i + 1], t);
        }
    }
    return uniforms.colStops[7];
}

fragment float4 fragmentMain(VertexOutput in [[stage_in]],
                             texture2d<float> u_texture [[texture(0)]],
                             sampler sampler_state [[sampler(0)]],
                             constant Uniforms& uniforms [[buffer(0)]]) {
    
    // Clamp to pixel centers to avoid interpolation bleed
    float2 clamped_coord = float2(
        0.5 / 700.0 + in.texcoord.x * (699.0 / 700.0),
        0.5 / 765.0 + in.texcoord.y * (764.0 / 765.0)
    );
    
    float4 tex = u_texture.sample(sampler_state, clamped_coord);
    if (uniforms.layerMode != 3.0 && tex.a < 0.99) {
        discard_fragment();
    }
    
    float r = tex.r * 255.0;
    float g = tex.g * 255.0;
    float raw_val = r * 256.0 + g;
    
    if (raw_val >= 65535.0 || raw_val == 0.0) {
        discard_fragment();
    }
    
    float val;
    int modeInt = int(uniforms.layerMode);
    if (modeInt == 1) { // temp
        val = raw_val / 10.0 - 273.15;
    } else if (modeInt == 2) { // solar
        val = raw_val;
    } else if (modeInt == 3) { // wind speed field
        float u_raw = raw_val;
        float v_raw = tex.b * 255.0 * 256.0 + tex.a * 255.0;
        float u_phys = u_raw / 100.0 - 100.0;
        float v_phys = v_raw / 100.0 - 100.0;
        val = sqrt(u_phys * u_phys + v_phys * v_phys);
    } else { // rain
        val = raw_val * 0.01;
    }
    
    float4 c = getColor(val, uniforms);
    if (c.a == 0.0) {
        discard_fragment();
    }
    
    return float4(c.rgb, c.a * uniforms.opacity);
}

struct ParticleVertexInput {
    float2 position [[attribute(0)]];
    float alpha [[attribute(1)]];
};

struct ParticleVertexOutput {
    float4 position [[position]];
    float alpha;
};

vertex ParticleVertexOutput particleVertexMain(ParticleVertexInput in [[stage_in]]) {
    ParticleVertexOutput out;
    out.position = float4(in.position, 0.0, 1.0);
    out.alpha = in.alpha;
    return out;
}

fragment float4 particleFragmentMain(ParticleVertexOutput in [[stage_in]],
                                     constant float& opacity [[buffer(0)]]) {
    return float4(1.0, 1.0, 1.0, opacity * in.alpha * 0.6);
}

