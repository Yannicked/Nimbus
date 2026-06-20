import { CONFIG } from '../config.js';
import { state } from '../state.js';
import { DOM } from '../ui/dom.js';
import { getOrLoadTexture } from './index.js';

export class WebGLRadarLayer {
    constructor(id = 'radar-webgl-layer', isCompare = false) {
        this.id = id;
        this.isCompare = isCompare;
        this.type = 'custom';
        this.renderingMode = '2d';
    }

    onAdd(mapInstance, gl) {
        if (this.isCompare) {
            state.glContextRight = gl;
        } else {
            state.glContext = gl;
        }
        console.log(`Initializing WebGL Radar Layer (${this.id}) shaders and buffers...`);
        
        // 1. Compile Shaders
        const vertexShaderSource = `
            attribute vec2 a_position;
            attribute vec2 a_texcoord;
            varying vec2 v_texcoord;
            uniform mat4 u_matrix;
            void main() {
                gl_Position = u_matrix * vec4(a_position, 0.0, 1.0);
                v_texcoord = a_texcoord;
            }
        `;
        
        const fragmentShaderSource = `
            precision mediump float;
            varying vec2 v_texcoord;
            uniform sampler2D u_texture;
            uniform float u_opacity;
            uniform vec4 u_colors[8];
            uniform float u_values[8];
            uniform int u_layer_mode;
            
            vec4 getColor(float val) {
                if (val < u_values[0]) return vec4(0.0);
                if (val <= u_values[1]) {
                    float t = (val - u_values[0]) / (u_values[1] - u_values[0]);
                    return mix(u_colors[0], u_colors[1], t);
                }
                if (val <= u_values[2]) {
                    float t = (val - u_values[1]) / (u_values[2] - u_values[1]);
                    return mix(u_colors[1], u_colors[2], t);
                }
                if (val <= u_values[3]) {
                    float t = (val - u_values[2]) / (u_values[3] - u_values[2]);
                    return mix(u_colors[2], u_colors[3], t);
                }
                if (val <= u_values[4]) {
                    float t = (val - u_values[3]) / (u_values[4] - u_values[3]);
                    return mix(u_colors[3], u_colors[4], t);
                }
                if (val <= u_values[5]) {
                    float t = (val - u_values[4]) / (u_values[5] - u_values[4]);
                    return mix(u_colors[4], u_colors[5], t);
                }
                if (val <= u_values[6]) {
                    float t = (val - u_values[5]) / (u_values[6] - u_values[5]);
                    return mix(u_colors[5], u_colors[6], t);
                }
                if (val <= u_values[7]) {
                    float t = (val - u_values[6]) / (u_values[7] - u_values[6]);
                    return mix(u_colors[6], u_colors[7], t);
                }
                return u_colors[7];
            }
            
            float sample_radar_pixel(sampler2D tex, float col, float row) {
                vec2 uv = vec2((col + 0.5) / 700.0, (row + 0.5) / 765.0);
                vec4 color = texture2D(tex, uv);
                if (color.a < 0.99) {
                    return -9999.0;
                }
                float r = floor(color.r * 255.0 + 0.5);
                float g = floor(color.g * 255.0 + 0.5);
                float raw_val = r * 256.0 + g;
                if (raw_val >= 65535.0) {
                    return -9999.0;
                }
                return raw_val;
            }

            float interpolate_radar(sampler2D tex, float x_norm, float y_norm) {
                // Clamp to valid UV range
                x_norm = clamp(x_norm, 0.0, 1.0);
                y_norm = clamp(y_norm, 0.0, 1.0);
                
                float px = x_norm * 699.0;
                float py = y_norm * 764.0;
                
                float x0 = floor(px);
                float y0 = floor(py);
                float x1 = min(x0 + 1.0, 699.0);
                float y1 = min(y0 + 1.0, 764.0);
                
                float tx = px - x0;
                float ty = py - y0;
                
                float p00 = sample_radar_pixel(tex, x0, y0);
                float p10 = sample_radar_pixel(tex, x1, y0);
                float p01 = sample_radar_pixel(tex, x0, y1);
                float p11 = sample_radar_pixel(tex, x1, y1);
                
                bool v00 = (p00 != -9999.0);
                bool v10 = (p10 != -9999.0);
                bool v01 = (p01 != -9999.0);
                bool v11 = (p11 != -9999.0);
                
                if (!v00 && !v10 && !v01 && !v11) {
                    return -9999.0;
                }
                
                if (!v00 || !v10 || !v01 || !v11) {
                    float cx = floor(px + 0.5);
                    float cy = floor(py + 0.5);
                    return sample_radar_pixel(tex, cx, cy);
                }
                
                float p0 = mix(p00, p10, tx);
                float p1 = mix(p01, p11, tx);
                return mix(p0, p1, ty);
            }
            
            void main() {
                float raw = interpolate_radar(u_texture, v_texcoord.x, v_texcoord.y);
                
                if (raw == -9999.0 || raw == 0.0) {
                    discard;
                }
                
                float val;
                if (u_layer_mode == 1) {
                    val = raw / 10.0 - 273.15;
                } else if (u_layer_mode == 2) {
                    val = raw;
                } else {
                    val = raw * 0.01;
                }
                
                vec4 c = getColor(val);
                if (c.a == 0.0) {
                    discard;
                }
                gl_FragColor = vec4(c.rgb, c.a * u_opacity);
            }
        `;
        
        function compileShader(source, type) {
            const shader = gl.createShader(type);
            gl.shaderSource(shader, source);
            gl.compileShader(shader);
            if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
                console.error("Shader compilation error:", gl.getShaderInfoLog(shader));
            }
            return shader;
        }
        
        const vs = compileShader(vertexShaderSource, gl.VERTEX_SHADER);
        const fs = compileShader(fragmentShaderSource, gl.FRAGMENT_SHADER);
        
        const program = gl.createProgram();
        gl.attachShader(program, vs);
        gl.attachShader(program, fs);
        gl.linkProgram(program);
        
        if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
            console.error("Program linking error:", gl.getProgramInfoLog(program));
        }
        
        if (this.isCompare) {
            state.radarProgramRight = program;
        } else {
            state.radarProgram = program;
        }
        
        // 2. Set up Mercator projection vertex buffer
        const MAP_LIMIT = 20037508.342789244;
        function toMerc(x, y) {
            const ux = (x + MAP_LIMIT) / (2.0 * MAP_LIMIT);
            const uy = (MAP_LIMIT - y) / (2.0 * MAP_LIMIT);
            return [ux, uy];
        }
        
        // Bounding box: MERCATOR_LEFT: 0.0, MERCATOR_RIGHT: 1210000.0, MERCATOR_BOTTOM: 6250000.0, MERCATOR_TOP: 7560000.0
        const BL = toMerc(0.0, 6250000.0);
        const BR = toMerc(1210000.0, 6250000.0);
        const TR = toMerc(1210000.0, 7560000.0);
        const TL = toMerc(0.0, 7560000.0);
        
        // Define two triangles forming a quad (counter-clockwise order)
        const vertices = new Float32Array([
            BL[0], BL[1], // SW
            BR[0], BR[1], // SE
            TL[0], TL[1], // NW
            TL[0], TL[1], // NW
            BR[0], BR[1], // SE
            TR[0], TR[1]  // NE
        ]);
        
        const posBuf = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, posBuf);
        gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);
        
        // Texture Coordinates (corrected to match UNPACK_FLIP_Y_WEBGL=true orientation)
        const texcoords = new Float32Array([
            0, 0, // BL
            1, 0, // BR
            0, 1, // TL
            0, 1, // TL
            1, 0, // BR
            1, 1  // TR
        ]);
        
        const texBuf = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, texBuf);
        gl.bufferData(gl.ARRAY_BUFFER, texcoords, gl.STATIC_DRAW);

        if (this.isCompare) {
            state.positionBufferRight = posBuf;
            state.texcoordBufferRight = texBuf;
        } else {
            state.positionBuffer = posBuf;
            state.texcoordBuffer = texBuf;
        }
    }
    
    render(gl, matrix) {
        if (!state.metadata) return;
        const program = this.isCompare ? state.radarProgramRight : state.radarProgram;
        const posBuf = this.isCompare ? state.positionBufferRight : state.positionBuffer;
        const texBuf = this.isCompare ? state.texcoordBufferRight : state.texcoordBuffer;
        if (!program) return;
        
        const layerMode = this.isCompare ? state.compareLayerMode : state.currentLayerMode;
        const ens = this.isCompare ? state.compareEns : state.currentEns;
        const currentIndex = Math.round(state.currentTimeIndex);
        const timeVal = state.metadata.times[currentIndex];
        if (timeVal === undefined) return;
        
        const texture = getOrLoadTexture(gl, timeVal, this.isCompare);
        if (!texture) return;

        // Force NEAREST filtering so manual 16-bit bilinear interpolation is precise
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.bindTexture(gl.TEXTURE_2D, null);
        
        gl.useProgram(program);
        
        // 1. Save and disable depth test to ensure it always renders on top of the base map
        const depthTestEnabled = gl.isEnabled(gl.DEPTH_TEST);
        if (depthTestEnabled) {
            gl.disable(gl.DEPTH_TEST);
        }
        
        // 2. Bind default VAO to prevent mutating MapLibre's internal VAO state in WebGL 2
        if (gl.bindVertexArray) {
            gl.bindVertexArray(null);
        }
        
        // Bind position attribute
        const aPosition = gl.getAttribLocation(program, 'a_position');
        gl.enableVertexAttribArray(aPosition);
        gl.bindBuffer(gl.ARRAY_BUFFER, posBuf);
        gl.vertexAttribPointer(aPosition, 2, gl.FLOAT, false, 0, 0);
        
        // Bind texture coordinates attribute
        const aTexcoord = gl.getAttribLocation(program, 'a_texcoord');
        gl.enableVertexAttribArray(aTexcoord);
        gl.bindBuffer(gl.ARRAY_BUFFER, texBuf);
        gl.vertexAttribPointer(aTexcoord, 2, gl.FLOAT, false, 0, 0);
        
        // Set projection matrix
        gl.uniformMatrix4fv(gl.getUniformLocation(program, 'u_matrix'), false, matrix);
        
        // Bind texture
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.uniform1i(gl.getUniformLocation(program, 'u_texture'), 0);
        
        // Opacity uniform
        const opacity = parseFloat(DOM.opacitySlider.value) / 100;
        gl.uniform1f(gl.getUniformLocation(program, 'u_opacity'), opacity);
        
        // u_layer_mode uniform
        let modeInt = 0;
        if (layerMode === 'temp') {
            modeInt = 1;
        } else if (layerMode === 'solar') {
            modeInt = 2;
        } else if (ens === 'spread') {
            modeInt = 3;
        }
        gl.uniform1i(gl.getUniformLocation(program, 'u_layer_mode'), modeInt);
        
        // Dynamic color stops uniform configuration
        const isProb = ens === 'prob';
        const isSpread = ens === 'spread';
        let colors, values;
        
        if (layerMode === 'temp') {
            colors = [
                [0/255, 43/255, 128/255, 0.8],
                [0/255, 204/255, 255/255, 0.8],
                [0/255, 255/255, 102/255, 0.8],
                [255/255, 255/255, 0/255, 0.8],
                [255/255, 153/255, 0/255, 0.85],
                [255/255, 77/255, 77/255, 0.9],
                [204/255, 0/255, 0/255, 0.95],
                [153/255, 0/255, 77/255, 1.0]
            ];
            values = [-10.0, 0.0, 10.0, 20.0, 25.0, 30.0, 35.0, 40.0];
        } else if (layerMode === 'solar') {
            colors = [
                [0/255, 0/255, 0/255, 0.0],
                [253/255, 224/255, 71/255, 0.3],
                [250/255, 204/255, 21/255, 0.5],
                [234/255, 179/255, 8/255, 0.7],
                [249/255, 115/255, 22/255, 0.85],
                [239/255, 68/255, 68/255, 0.95],
                [239/255, 68/255, 68/255, 0.95], // padding
                [239/255, 68/255, 68/255, 0.95]  // padding
            ];
            values = [10.0, 100.0, 250.0, 500.0, 750.0, 1000.0, 1000.0, 1000.0];
        } else if (isProb) {
            colors = [
                [180/255, 200/255, 220/255, 0.0],
                [100/255, 160/255, 255/255, 0.5],
                [0/255, 100/255, 255/255, 0.65],
                [0/255, 200/255, 100/255, 0.75],
                [220/255, 0/255, 220/255, 0.85],
                [255/255, 255/255, 255/255, 0.95],
                [255/255, 255/255, 255/255, 0.95], // padding
                [255/255, 255/255, 255/255, 0.95]  // padding
            ];
            values = [0.10, 0.30, 0.50, 0.70, 0.90, 1.00, 1.00, 1.00];
        } else if (isSpread) {
            colors = [
                [99/255, 102/255, 241/255, 0.0],
                [99/255, 102/255, 241/255, 0.4],
                [168/255, 85/255, 247/255, 0.6],
                [236/255, 72/255, 153/255, 0.75],
                [244/255, 63/255, 94/255, 0.9],
                [255/255, 255/255, 255/255, 0.95],
                [255/255, 255/255, 255/255, 0.95], // padding
                [255/255, 255/255, 255/255, 0.95]  // padding
            ];
            values = [0.05, 0.2, 1.0, 5.0, 15.0, 30.0, 30.0, 30.0];
        } else {
            colors = [
                [120/255, 200/255, 255/255, 0.0],
                [0/255, 100/255, 255/255, 0.7],
                [0/255, 200/255, 0/255, 0.7],
                [255/255, 230/255, 0/255, 0.8],
                [255/255, 120/255, 0/255, 0.9],
                [255/255, 0/255, 0/255, 0.95],
                [200/255, 0/255, 200/255, 1.0],
                [255/255, 255/255, 255/255, 1.0]
            ];
            values = [0.05, 0.2, 1.0, 5.0, 15.0, 30.0, 100.0, 250.0];
        }
        
        const flatColors = new Float32Array(colors.reduce((acc, val) => acc.concat(val), []));
        const flatValues = new Float32Array(values);
        
        gl.uniform4fv(gl.getUniformLocation(program, 'u_colors[0]'), flatColors);
        gl.uniform1fv(gl.getUniformLocation(program, 'u_values[0]'), flatValues);
        
        // Alpha Blending config
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        
        gl.drawArrays(gl.TRIANGLES, 0, 6);
        
        // 3. Cleanup state to prevent leaks
        gl.disableVertexAttribArray(aPosition);
        gl.disableVertexAttribArray(aTexcoord);
        gl.bindBuffer(gl.ARRAY_BUFFER, null);
        if (depthTestEnabled) {
            gl.enable(gl.DEPTH_TEST);
        }
    }
    
    onRemove(map, gl) {
        const program = this.isCompare ? state.radarProgramRight : state.radarProgram;
        const posBuf = this.isCompare ? state.positionBufferRight : state.positionBuffer;
        const texBuf = this.isCompare ? state.texcoordBufferRight : state.texcoordBuffer;

        if (program) {
            gl.deleteProgram(program);
            if (this.isCompare) state.radarProgramRight = null; else state.radarProgram = null;
        }
        if (posBuf) {
            gl.deleteBuffer(posBuf);
            if (this.isCompare) state.positionBufferRight = null; else state.positionBuffer = null;
        }
        if (texBuf) {
            gl.deleteBuffer(texBuf);
            if (this.isCompare) state.texcoordBufferRight = null; else state.texcoordBuffer = null;
        }
    }
}
