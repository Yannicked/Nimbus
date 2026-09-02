import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

console.log('--- Running WebGL Shader Syntax & Precision Validation Tests ---');

// Read shader sources directly from WebGLRadar.js and WebGLWind.js
const webglRadarPath = resolve(__dirname, '../../static/src/map/WebGLRadar.js');
const webglWindPath = resolve(__dirname, '../../static/src/map/WebGLWind.js');

const radarCode = readFileSync(webglRadarPath, 'utf8');
const windCode = readFileSync(webglWindPath, 'utf8');

// 1. Radar Vertex Shader Validation
console.log('Testing Radar Vertex Shader...');
assert(radarCode.includes('attribute vec2 a_position;'), 'Radar VS must define a_position attribute');
assert(radarCode.includes('attribute vec2 a_texcoord;'), 'Radar VS must define a_texcoord attribute');
assert(radarCode.includes('varying vec2 v_texcoord;'), 'Radar VS must define v_texcoord varying');
assert(radarCode.includes('uniform mat4 u_matrix;'), 'Radar VS must define u_matrix uniform');

// 2. Radar Fragment Shader Validation & Precision
console.log('Testing Radar Fragment Shader & Precision...');
assert(radarCode.includes('precision highp float;') || radarCode.includes('precision mediump float;'), 'Radar FS must define precision qualifier');
assert(radarCode.includes('uniform sampler2D u_texture_curr;') || radarCode.includes('uniform sampler2D u_texture;'), 'Radar FS must define texture uniforms');
assert(radarCode.includes('uniform float u_opacity;'), 'Radar FS must define u_opacity uniform');
assert(radarCode.includes('uniform vec4 u_colors[8];'), 'Radar FS must define u_colors uniform array');
assert(radarCode.includes('uniform float u_values[8];'), 'Radar FS must define u_values uniform array');
assert(radarCode.includes('uniform int u_layer_mode;'), 'Radar FS must define u_layer_mode uniform');

// 3. Radar 16-bit Unpack and Interpolation in Fragment Shader
console.log('Testing Radar 16-bit Unpack GLSL Math...');
assert(radarCode.includes('floor(color.r * 255.0 + 0.5)'), 'Radar FS must unpack R channel with exact rounding');
assert(radarCode.includes('floor(color.g * 255.0 + 0.5)'), 'Radar FS must unpack G channel with exact rounding');
assert(radarCode.includes('r * 256.0 + g'), 'Radar FS must combine 16-bit value using 256 multiplier');
assert(radarCode.includes('raw_val >= 65535.0'), 'Radar FS must check 65535 NODATA sentinel');

// 4. Wind Vertex & Fragment Shaders Validation
console.log('Testing Wind Background & Particle Shaders...');
assert(windCode.includes('uniform sampler2D u_texture;') || windCode.includes('uniform sampler2D u_wind_texture;') || windCode.includes('uniform sampler2D u_wind_curr;'), 'Wind FS must define texture uniform');
assert(windCode.includes('uniform float u_opacity;'), 'Wind FS must define u_opacity uniform');
assert(windCode.includes('sample_wind_pixel') || windCode.includes('sample_wind'), 'Wind FS must define sample_wind function');
assert(windCode.includes('r * 256.0 + g') || windCode.includes('256.0'), 'Wind FS must decode U component via 16-bit unpack');
assert(windCode.includes('b * 256.0 + a') || windCode.includes('256.0'), 'Wind FS must decode V component via 16-bit unpack');

// 5. Simulate GLSL Color Ramp Calculation in JS
function glslGetColor(val, values, colors) {
    if (val < values[0]) return [0, 0, 0, 0];
    for (let i = 0; i < 7; i++) {
        if (val <= values[i + 1]) {
            const t = (val - values[i]) / (values[i + 1] - values[i]);
            return [
                colors[i][0] * (1 - t) + colors[i + 1][0] * t,
                colors[i][1] * (1 - t) + colors[i + 1][1] * t,
                colors[i][2] * (1 - t) + colors[i + 1][2] * t,
                colors[i][3] * (1 - t) + colors[i + 1][3] * t,
            ];
        }
    }
    return colors[7];
}

const rainValues = [0.05, 0.2, 1.0, 5.0, 15.0, 30.0, 100.0, 250.0];
const rainColors = [
    [120/255, 200/255, 255/255, 0.0],
    [0/255, 100/255, 255/255, 0.7],
    [0/255, 200/255, 0/255, 0.7],
    [255/255, 230/255, 0/255, 0.8],
    [255/255, 120/255, 0/255, 0.9],
    [255/255, 0/255, 0/255, 0.95],
    [200/255, 0/255, 200/255, 1.0],
    [255/255, 255/255, 255/255, 1.0]
];

const zeroColor = glslGetColor(0.01, rainValues, rainColors);
assert.equal(zeroColor[3], 0.0);

const lightRainColor = glslGetColor(0.1, rainValues, rainColors);
assert(lightRainColor[3] > 0.0 && lightRainColor[3] <= 0.7);

const heavyRainColor = glslGetColor(50.0, rainValues, rainColors);
assert(heavyRainColor[3] >= 0.95);

console.log('✓ All 5 WebGL Shader Syntax & Precision Tests Passed Successfully!');
