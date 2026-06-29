#version 460 core
#include <flutter/runtime_effect.glsl>

uniform vec2 u_size;
uniform float u_opacity;
uniform float u_layer_mode;
uniform float u_val0;
uniform float u_val1;
uniform float u_val2;
uniform float u_val3;
uniform float u_val4;
uniform float u_val5;
uniform float u_val6;
uniform float u_val7;
uniform vec4 u_col0;
uniform vec4 u_col1;
uniform vec4 u_col2;
uniform vec4 u_col3;
uniform vec4 u_col4;
uniform vec4 u_col5;
uniform vec4 u_col6;
uniform vec4 u_col7;
uniform sampler2D u_texture;

out vec4 fragColor;

vec4 getColor(float val) {
    if (val < u_val0) return vec4(0.0);
    if (val <= u_val1) {
        float t = (val - u_val0) / (u_val1 - u_val0);
        return mix(u_col0, u_col1, t);
    }
    if (val <= u_val2) {
        float t = (val - u_val1) / (u_val2 - u_val1);
        return mix(u_col1, u_col2, t);
    }
    if (val <= u_val3) {
        float t = (val - u_val2) / (u_val3 - u_val2);
        return mix(u_col2, u_col3, t);
    }
    if (val <= u_val4) {
        float t = (val - u_val3) / (u_val4 - u_val3);
        return mix(u_col3, u_col4, t);
    }
    if (val <= u_val5) {
        float t = (val - u_val4) / (u_val5 - u_val4);
        return mix(u_col4, u_col5, t);
    }
    if (val <= u_val6) {
        float t = (val - u_val5) / (u_val6 - u_val5);
        return mix(u_col5, u_col6, t);
    }
    if (val <= u_val7) {
        float t = (val - u_val6) / (u_val7 - u_val6);
        return mix(u_col6, u_col7, t);
    }
    return u_col7;
}

void main() {
    vec2 uv = FlutterFragCoord().xy / u_size;

    // Avoid border/interpolation artifacts by clamping coordinates to pixel centers
    vec2 clamped_coord = vec2(
        0.5 / 700.0 + uv.x * (699.0 / 700.0),
        0.5 / 765.0 + uv.y * (764.0 / 765.0)
    );

    vec4 tex = texture(u_texture, clamped_coord);

    // Check for NODATA/transparency
    if (u_layer_mode != 3.0 && tex.a < 0.99) {
        fragColor = vec4(0.0);
        return;
    }

    float r = tex.r * 255.0;
    float g = tex.g * 255.0;
    float raw_val = r * 256.0 + g;

    if (raw_val >= 65535.0 || raw_val == 0.0) {
        fragColor = vec4(0.0);
        return;
    }

    float val;
    int modeInt = int(u_layer_mode);
    if (modeInt == 1) { // temp
        val = raw_val / 10.0 - 273.15;
    } else if (modeInt == 2) { // solar
        val = raw_val;
    } else if (modeInt == 3) { // wind speed field
        // Decoded wind components: R/G for u, B/A for v
        float u_raw = raw_val;
        float v_raw = tex.b * 255.0 * 256.0 + tex.a * 255.0;
        float u_phys = u_raw / 100.0 - 100.0;
        float v_phys = v_raw / 100.0 - 100.0;
        val = sqrt(u_phys * u_phys + v_phys * v_phys);
    } else { // rain
        val = raw_val * 0.01;
    }

    vec4 c = getColor(val);
    if (c.a == 0.0) {
        fragColor = vec4(0.0);
        return;
    }

    fragColor = vec4(c.rgb, c.a * u_opacity);
}
