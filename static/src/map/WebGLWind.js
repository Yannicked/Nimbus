import { CONFIG } from '../config.js';
import { state } from '../state.js';
import { DOM } from '../ui/dom.js';
import { getOrLoadTexture } from './index.js';

export class WebGLWindLayer {
    constructor() {
        this.id = 'wind-webgl-layer';
        this.type = 'custom';
        this.renderingMode = '2d';

        // GPU Simulation Settings
        this.numParticles = 2048;
        this.trailLength = 48;
        this.currentStateIndex = 0;

        // WebGL resources owned by this layer
        this.stateTextures = [null, null];
        this.stateFBOs = [null, null];
        this.updateProgram = null;
        this.quadBuffer = null;
        this.particleUVBuffer = null;
    }

    // Keep this for MapLibre/index.js callbacks, but make it CPU-overhead-free
    updateWindPixelData(img) {
        state.windPixelData = true;
    }

    getWindVelocity(mx, my) {
        return [0, 0]; // No longer used for CPU particle rendering
    }

    initParticles() {
        // No-op on CPU since we initialize fully on GPU inside onAdd
    }

    // Helper to compile shaders and link program
    _createProgram(gl, vsSource, fsSource) {
        const vs = gl.createShader(gl.VERTEX_SHADER);
        gl.shaderSource(vs, vsSource);
        gl.compileShader(vs);
        if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) {
            console.error("Shader VS compile error:", gl.getShaderInfoLog(vs));
        }

        const fs = gl.createShader(gl.FRAGMENT_SHADER);
        gl.shaderSource(fs, fsSource);
        gl.compileShader(fs);
        if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
            console.error("Shader FS compile error:", gl.getShaderInfoLog(fs));
        }

        const program = gl.createProgram();
        gl.attachShader(program, vs);
        gl.attachShader(program, fs);
        gl.linkProgram(program);
        if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
            console.error("Program link error:", gl.getProgramInfoLog(program));
        }
        return program;
    }

    onAdd(mapInstance, gl) {
        state.glContext = gl;
        console.log("Initializing WebGL Wind Layer fully on GPU...");

        // 1. Compile background heat map overlay shader (with manual 16-bit bilinear interpolation)
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
            
            vec4 getColor(float val) {
                if (val < 0.0) return vec4(0.0);
                if (val <= 2.0) {
                    float t = val / 2.0;
                    return mix(vec4(96.0/255.0, 165.0/255.0, 250.0/255.0, 0.02), vec4(34.0/255.0, 211.0/255.0, 238.0/255.0, 0.35), t);
                }
                if (val <= 5.0) {
                    float t = (val - 2.0) / 3.0;
                    return mix(vec4(34.0/255.0, 211.0/255.0, 238.0/255.0, 0.35), vec4(74.0/255.0, 222.0/255.0, 128.0/255.0, 0.55), t);
                }
                if (val <= 10.0) {
                    float t = (val - 5.0) / 5.0;
                    return mix(vec4(74.0/255.0, 222.0/255.0, 128.0/255.0, 0.55), vec4(250.0/255.0, 204.0/255.0, 21.0/255.0, 0.7), t);
                }
                if (val <= 15.0) {
                    float t = (val - 10.0) / 5.0;
                    return mix(vec4(250.0/255.0, 204.0/255.0, 21.0/255.0, 0.7), vec4(251.0/255.0, 146.0/255.0, 60.0/255.0, 0.8), t);
                }
                if (val <= 20.0) {
                    float t = (val - 15.0) / 5.0;
                    return mix(vec4(251.0/255.0, 146.0/255.0, 60.0/255.0, 0.8), vec4(248.0/255.0, 113.0/255.0, 113.0/255.0, 0.85), t);
                }
                if (val <= 25.0) {
                    float t = (val - 20.0) / 5.0;
                    return mix(vec4(248.0/255.0, 113.0/255.0, 113.0/255.0, 0.85), vec4(236.0/255.0, 72.0/255.0, 153.0/255.0, 0.9), t);
                }
                return vec4(236.0/255.0, 72.0/255.0, 153.0/255.0, 0.9);
            }

            float sample_wind_pixel(sampler2D tex, float col, float row, float half_offset) {
                vec2 uv = vec2((col + 0.5) / 700.0, (row + 0.5) / 765.0 * 0.5 + half_offset);
                vec4 color = texture2D(tex, uv);
                float r = floor(color.r * 255.0 + 0.5);
                float g = floor(color.g * 255.0 + 0.5);
                return r * 256.0 + g;
            }

            float interpolate_wind(sampler2D tex, float x_norm, float y_norm, float half_offset) {
                float px = x_norm * 699.0;
                float py = y_norm * 764.0;
                
                float x0 = floor(px);
                float y0 = floor(py);
                float x1 = min(x0 + 1.0, 699.0);
                float y1 = min(y0 + 1.0, 764.0);
                
                float tx = px - x0;
                float ty = py - y0;
                
                float p00 = sample_wind_pixel(tex, x0, y0, half_offset);
                float p10 = sample_wind_pixel(tex, x1, y0, half_offset);
                float p01 = sample_wind_pixel(tex, x0, y1, half_offset);
                float p11 = sample_wind_pixel(tex, x1, y1, half_offset);
                
                if (p00 >= 65535.0 || p10 >= 65535.0 || p01 >= 65535.0 || p11 >= 65535.0 ||
                    p00 == 0.0 || p10 == 0.0 || p01 == 0.0 || p11 == 0.0) {
                    return -1.0; 
                }
                
                float p0 = mix(p00, p10, tx);
                float p1 = mix(p01, p11, tx);
                return mix(p0, p1, ty);
            }
            
            void main() {
                float u_raw = interpolate_wind(u_texture, v_texcoord.x, v_texcoord.y, 0.5);
                float v_raw = interpolate_wind(u_texture, v_texcoord.x, v_texcoord.y, 0.0);
                
                if (u_raw < 0.0 || v_raw < 0.0) {
                    discard;
                }
                
                float u = u_raw / 100.0 - 100.0;
                float v = v_raw / 100.0 - 100.0;
                float speed = sqrt(u * u + v * v);
                
                vec4 c = getColor(speed);
                gl_FragColor = vec4(c.rgb, c.a * u_opacity);
            }
        `;

        state.windProgram = this._createProgram(gl, vertexShaderSource, fragmentShaderSource);

        // 2. Compile GPU simulation update program
        const updateVsSource = `
            attribute vec2 a_position;
            varying vec2 v_texcoord;
            void main() {
                gl_Position = vec4(a_position, 0.0, 1.0);
                v_texcoord = a_position * 0.5 + 0.5;
            }
        `;

        const updateFsSource = `
            precision mediump float;
            varying vec2 v_texcoord;
            uniform sampler2D u_state_texture;
            uniform sampler2D u_wind_texture;
            uniform float u_dt;
            uniform float u_speed_factor;
            uniform float u_rand_seed;
            uniform vec2 u_tex_size;

            float rand(vec2 co) {
                return fract(sin(dot(co, vec2(12.9898, 78.233))) * 43758.5453);
            }

            float rand2(vec2 co) {
                return fract(sin(dot(co, vec2(43.2312, 113.8213))) * 43758.5453);
            }

            float unpack12_X(vec4 color) {
                float r = floor(color.r * 255.0 + 0.5);
                float g = floor(color.g * 255.0 + 0.5);
                float hi = r;
                float lo = floor(g / 16.0);
                return (hi * 16.0 + lo) / 4095.0;
            }

            float unpack12_Y(vec4 color) {
                float g = floor(color.g * 255.0 + 0.5);
                float b = floor(color.b * 255.0 + 0.5);
                float hi = b;
                float lo = mod(g, 16.0);
                return (hi * 16.0 + lo) / 4095.0;
            }

            vec4 pack12(float x, float y, float age) {
                float x_val = floor(x * 4095.0 + 0.5);
                float y_val = floor(y * 4095.0 + 0.5);
                
                float x_hi = floor(x_val / 16.0);
                float x_lo = mod(x_val, 16.0);
                
                float y_hi = floor(y_val / 16.0);
                float y_lo = mod(y_val, 16.0);
                
                float r = x_hi / 255.0;
                float g = (x_lo * 16.0 + y_lo) / 255.0;
                float b = y_hi / 255.0;
                float a = age;
                
                return vec4(r, g, b, a);
            }

            float sample_wind_pixel(sampler2D tex, float col, float row, float half_offset) {
                vec2 uv = vec2((col + 0.5) / 700.0, (row + 0.5) / 765.0 * 0.5 + half_offset);
                vec4 color = texture2D(tex, uv);
                float r = floor(color.r * 255.0 + 0.5);
                float g = floor(color.g * 255.0 + 0.5);
                return r * 256.0 + g;
            }

            float interpolate_wind(sampler2D tex, float x_norm, float y_norm, float half_offset) {
                float px = x_norm * 699.0;
                float py = y_norm * 764.0;
                
                float x0 = floor(px);
                float y0 = floor(py);
                float x1 = min(x0 + 1.0, 699.0);
                float y1 = min(y0 + 1.0, 764.0);
                
                float tx = px - x0;
                float ty = py - y0;
                
                float p00 = sample_wind_pixel(tex, x0, y0, half_offset);
                float p10 = sample_wind_pixel(tex, x1, y0, half_offset);
                float p01 = sample_wind_pixel(tex, x0, y1, half_offset);
                float p11 = sample_wind_pixel(tex, x1, y1, half_offset);
                
                if (p00 >= 65535.0 || p10 >= 65535.0 || p01 >= 65535.0 || p11 >= 65535.0 ||
                    p00 == 0.0 || p10 == 0.0 || p01 == 0.0 || p11 == 0.0) {
                    return -1.0; 
                }
                
                float p0 = mix(p00, p10, tx);
                float p1 = mix(p01, p11, tx);
                return mix(p0, p1, ty);
            }

            void main() {
                vec2 pixel = v_texcoord * u_tex_size;
                float col = floor(pixel.x);
                float row = floor(pixel.y);

                if (row == 0.0) {
                    // Head particle update
                    vec2 head_uv = vec2((col + 0.5) / u_tex_size.x, 0.5 / u_tex_size.y);
                    vec4 state_col = texture2D(u_state_texture, head_uv);

                    float x = unpack12_X(state_col);
                    float y = unpack12_Y(state_col);
                    float age = state_col.a;

                    bool reset = false;
                    if (age >= 0.99 || age <= 0.0) {
                        reset = true;
                    }

                    // Re-add a very small continuous random drop rate (0.1% per frame) to gently dissolve long-term sinks/clustering
                    float drop_rand = rand(vec2(col / u_tex_size.x, u_rand_seed + 9.876));
                    if (drop_rand < 0.001) {
                        reset = true;
                    }

                    float u_raw = interpolate_wind(u_wind_texture, x, 1.0 - y, 0.5);
                    float v_raw = interpolate_wind(u_wind_texture, x, 1.0 - y, 0.0);

                    if (u_raw < 0.0 || v_raw < 0.0) {
                        reset = true;
                    }

                    if (reset) {
                        float rx = rand(vec2(col / u_tex_size.x, u_rand_seed));
                        float ry = rand2(vec2(col / u_tex_size.x, u_rand_seed));
                        float rand_age = rand(vec2(rx, ry)) * 0.5;
                        gl_FragColor = pack12(rx, ry, rand_age);
                    } else {
                        float u = u_raw / 100.0 - 100.0;
                        float v = v_raw / 100.0 - 100.0;

                        float dx_norm = (u * u_dt * u_speed_factor * 1200.0) / 1210000.0;
                        float dy_norm = -(v * u_dt * u_speed_factor * 1200.0) / 1310000.0;

                        x += dx_norm;
                        y += dy_norm;

                        if (x < 0.0 || x > 1.0 || y < 0.0 || y > 1.0) {
                            float rx = rand(vec2(col / u_tex_size.x, u_rand_seed + 1.123));
                            float ry = rand2(vec2(col / u_tex_size.x, u_rand_seed + 2.345));
                            gl_FragColor = pack12(rx, ry, 0.0);
                        } else {
                            float max_age_frames = 400.0 + rand(vec2(col / u_tex_size.x, 7.89)) * 400.0;
                            float age_step = (u_dt * 60.0) / max_age_frames;
                            age += age_step;
                            gl_FragColor = pack12(x, y, age);
                        }
                    }
                } else {
                    // Shift trail history (copy previous row)
                    vec2 prev_row_uv = vec2((col + 0.5) / u_tex_size.x, (row - 0.5) / u_tex_size.y);
                    gl_FragColor = texture2D(u_state_texture, prev_row_uv);
                }
            }
        `;

        this.updateProgram = this._createProgram(gl, updateVsSource, updateFsSource);

        // 3. Compile particle rendering program
        const particleVsSource = `
            attribute vec2 a_particle_uv;
            varying float v_fade;
            varying float v_trail;
            uniform sampler2D u_state_texture;
            uniform sampler2D u_wind_texture;
            uniform mat4 u_matrix;
            uniform float u_point_size;

            float unpack12_X(vec4 color) {
                float r = floor(color.r * 255.0 + 0.5);
                float g = floor(color.g * 255.0 + 0.5);
                float hi = r;
                float lo = floor(g / 16.0);
                return (hi * 16.0 + lo) / 4095.0;
            }

            float unpack12_Y(vec4 color) {
                float g = floor(color.g * 255.0 + 0.5);
                float b = floor(color.b * 255.0 + 0.5);
                float hi = b;
                float lo = mod(g, 16.0);
                return (hi * 16.0 + lo) / 4095.0;
            }

            float sample_wind_pixel(sampler2D tex, float col, float row, float half_offset) {
                vec2 uv = vec2((col + 0.5) / 700.0, (row + 0.5) / 765.0 * 0.5 + half_offset);
                vec4 color = texture2D(tex, uv);
                float r = floor(color.r * 255.0 + 0.5);
                float g = floor(color.g * 255.0 + 0.5);
                return r * 256.0 + g;
            }

            float interpolate_wind(sampler2D tex, float x_norm, float y_norm, float half_offset) {
                float px = x_norm * 699.0;
                float py = y_norm * 764.0;
                
                float x0 = floor(px);
                float y0 = floor(py);
                float x1 = min(x0 + 1.0, 699.0);
                float y1 = min(y0 + 1.0, 764.0);
                
                float tx = px - x0;
                float ty = py - y0;
                
                float p00 = sample_wind_pixel(tex, x0, y0, half_offset);
                float p10 = sample_wind_pixel(tex, x1, y0, half_offset);
                float p01 = sample_wind_pixel(tex, x0, y1, half_offset);
                float p11 = sample_wind_pixel(tex, x1, y1, half_offset);
                
                if (p00 >= 65535.0 || p10 >= 65535.0 || p01 >= 65535.0 || p11 >= 65535.0 ||
                    p00 == 0.0 || p10 == 0.0 || p01 == 0.0 || p11 == 0.0) {
                    return -1.0; 
                }
                
                float p0 = mix(p00, p10, tx);
                float p1 = mix(p01, p11, tx);
                return mix(p0, p1, ty);
            }

            void main() {
                vec4 state_col = texture2D(u_state_texture, a_particle_uv);
                
                float x_norm = unpack12_X(state_col);
                float y_norm = unpack12_Y(state_col);
                float age = state_col.a;

                float mx = x_norm * 1210000.0;
                float my = 7560000.0 - y_norm * 1310000.0;

                const float MAP_LIMIT = 20037508.342789244;
                float ux = (mx + MAP_LIMIT) / (2.0 * MAP_LIMIT);
                float uy = (MAP_LIMIT - my) / (2.0 * MAP_LIMIT);

                gl_Position = u_matrix * vec4(ux, uy, 0.0, 1.0);
                
                // Sample wind speed to fade out stationary particles smoothly with full 16-bit bilinear precision
                float u_raw = interpolate_wind(u_wind_texture, x_norm, 1.0 - y_norm, 0.5);
                float v_raw = interpolate_wind(u_wind_texture, x_norm, 1.0 - y_norm, 0.0);

                float speed = 0.0;
                if (u_raw >= 0.0 && v_raw >= 0.0) {
                    float u = u_raw / 100.0 - 100.0;
                    float v = v_raw / 100.0 - 100.0;
                    speed = sqrt(u * u + v * v);
                }

                float speed_fade = smoothstep(0.5, 2.0, speed);
                v_fade = smoothstep(0.0, 0.45, age) * smoothstep(1.0, 0.55, age) * speed_fade;
                v_trail = 1.0 - a_particle_uv.y;

                gl_PointSize = u_point_size * (0.3 + 0.7 * v_trail);
            }
        `;

        const particleFsSource = `
            precision mediump float;
            varying float v_fade;
            varying float v_trail;
            uniform float u_arrow_opacity;
            
            void main() {
                vec2 p = gl_PointCoord - vec2(0.5);
                float dist = length(p);
                if (dist > 0.5) {
                    discard;
                }
                float edgeAlpha = smoothstep(0.5, 0.25, dist);
                float opacity = edgeAlpha * v_fade * v_trail * u_arrow_opacity;
                gl_FragColor = vec4(1.0, 1.0, 1.0, opacity);
            }
        `;

        state.particleProgram = this._createProgram(gl, particleVsSource, particleFsSource);

        // 4. Set up Mercator quad buffers (background speed field overlay)
        const MAP_LIMIT = 20037508.342789244;
        function toMerc(x, y) {
            const ux = (x + MAP_LIMIT) / (2.0 * MAP_LIMIT);
            const uy = (MAP_LIMIT - y) / (2.0 * MAP_LIMIT);
            return [ux, uy];
        }

        const BL = toMerc(0.0, 6250000.0);
        const BR = toMerc(1210000.0, 6250000.0);
        const TR = toMerc(1210000.0, 7560000.0);
        const TL = toMerc(0.0, 7560000.0);

        const vertices = new Float32Array([
            BL[0], BL[1], // SW
            BR[0], BR[1], // SE
            TL[0], TL[1], // NW
            TL[0], TL[1], // NW
            BR[0], BR[1], // SE
            TR[0], TR[1]  // NE
        ]);

        state.windPositionBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, state.windPositionBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);

        const texcoords = new Float32Array([
            0, 0, // BL
            1, 0, // BR
            0, 1, // TL
            0, 1, // TL
            1, 0, // BR
            1, 1  // TR
        ]);

        state.windTexcoordBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, state.windTexcoordBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, texcoords, gl.STATIC_DRAW);

        // 5. Create screen-aligned quad buffer for FBO updates
        const quadVertices = new Float32Array([
            -1, -1,
             1, -1,
            -1,  1,
            -1,  1,
             1, -1,
             1,  1
        ]);
        this.quadBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, this.quadBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, quadVertices, gl.STATIC_DRAW);

        // 6. Set up static particle lookup coordinate buffer (UVs)
        const uvData = new Float32Array(this.numParticles * this.trailLength * 2);
        let idx = 0;
        for (let col = 0; col < this.numParticles; col++) {
            for (let row = 0; row < this.trailLength; row++) {
                uvData[idx++] = (col + 0.5) / this.numParticles;
                uvData[idx++] = (row + 0.5) / this.trailLength;
            }
        }
        this.particleUVBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, this.particleUVBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, uvData, gl.STATIC_DRAW);

        // 7. Initialize Ping-Pong FBOs & State textures
        function pack12_JS(x, y, age) {
            const x_val = Math.floor(x * 4095.0 + 0.5);
            const y_val = Math.floor(y * 4095.0 + 0.5);
            
            const x_hi = Math.floor(x_val / 16.0);
            const x_lo = x_val % 16;
            
            const y_hi = Math.floor(y_val / 16.0);
            const y_lo = y_val % 16;
            
            const r = x_hi;
            const g = (x_lo * 16) + y_lo;
            const b = y_hi;
            const a = Math.floor(age * 255.0 + 0.5);
            
            return [r, g, b, a];
        }

        const data = new Uint8Array(this.numParticles * this.trailLength * 4);
        for (let col = 0; col < this.numParticles; col++) {
            const x = Math.random();
            const y = Math.random();
            const age = Math.random() * 0.8;
            
            const [r, g, b, a] = pack12_JS(x, y, age);
            for (let row = 0; row < this.trailLength; row++) {
                const pixelIdx = (row * this.numParticles + col) * 4;
                data[pixelIdx]     = r;
                data[pixelIdx + 1] = g;
                data[pixelIdx + 2] = b;
                data[pixelIdx + 3] = a;
            }
        }

        for (let i = 0; i < 2; i++) {
            this.stateTextures[i] = gl.createTexture();
            gl.bindTexture(gl.TEXTURE_2D, this.stateTextures[i]);
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, this.numParticles, this.trailLength, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);

            this.stateFBOs[i] = gl.createFramebuffer();
            gl.bindFramebuffer(gl.FRAMEBUFFER, this.stateFBOs[i]);
            gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.stateTextures[i], 0);
        }

        gl.bindTexture(gl.TEXTURE_2D, null);
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);

        state.lastAnimTime = performance.now();
    }

    render(gl, matrix) {
        if (!state.metadata || !state.windProgram || !state.particleProgram || !this.updateProgram) return;

        const timeVal = state.metadata.times[state.currentTimeIndex];
        const windTexture = getOrLoadTexture(gl, timeVal);
        if (!windTexture) return; // Wait for texture load

        // Force NEAREST filtering on the wind texture to ensure manual 16-bit bilinear interpolation gets precise pixel bytes
        gl.bindTexture(gl.TEXTURE_2D, windTexture);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.bindTexture(gl.TEXTURE_2D, null);

        const now = performance.now();
        let dt = (now - state.lastAnimTime) / 1000.0;
        if (dt > 0.1) dt = 0.1;
        state.lastAnimTime = now;

        const depthTestEnabled = gl.isEnabled(gl.DEPTH_TEST);
        if (depthTestEnabled) {
            gl.disable(gl.DEPTH_TEST);
        }
        if (gl.bindVertexArray) {
            gl.bindVertexArray(null);
        }

        // -------------------------------------------------------------
        // Step A: GPU Simulation Update Pass (Ping-Pong FBO)
        // -------------------------------------------------------------
        const blendEnabled = gl.isEnabled(gl.BLEND);
        if (blendEnabled) {
            gl.disable(gl.BLEND);
        }

        const srcTex = this.stateTextures[this.currentStateIndex];
        const dstFBO = this.stateFBOs[1 - this.currentStateIndex];

        gl.bindFramebuffer(gl.FRAMEBUFFER, dstFBO);
        gl.viewport(0, 0, this.numParticles, this.trailLength);

        gl.useProgram(this.updateProgram);

        // Bind update quad position attribute
        const aUpdatePos = gl.getAttribLocation(this.updateProgram, 'a_position');
        gl.enableVertexAttribArray(aUpdatePos);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.quadBuffer);
        gl.vertexAttribPointer(aUpdatePos, 2, gl.FLOAT, false, 0, 0);

        // Set state textures
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, srcTex);
        gl.uniform1i(gl.getUniformLocation(this.updateProgram, 'u_state_texture'), 0);

        // Set wind velocity texture
        gl.activeTexture(gl.TEXTURE1);
        gl.bindTexture(gl.TEXTURE_2D, windTexture);
        gl.uniform1i(gl.getUniformLocation(this.updateProgram, 'u_wind_texture'), 1);

        // Set update uniforms
        gl.uniform1f(gl.getUniformLocation(this.updateProgram, 'u_dt'), dt);
        gl.uniform1f(gl.getUniformLocation(this.updateProgram, 'u_speed_factor'), 2.5 * 4);
        gl.uniform1f(gl.getUniformLocation(this.updateProgram, 'u_rand_seed'), Math.random());
        gl.uniform2f(gl.getUniformLocation(this.updateProgram, 'u_tex_size'), this.numParticles, this.trailLength);

        // Execute update pass
        gl.drawArrays(gl.TRIANGLES, 0, 6);

        // Clean up update pass
        gl.disableVertexAttribArray(aUpdatePos);
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);

        // Swap ping-pong FBOs
        this.currentStateIndex = 1 - this.currentStateIndex;
        const currentUpdatedTex = this.stateTextures[this.currentStateIndex];

        // Restore original screen viewport size
        gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);

        // Restore blend state if it was enabled
        if (blendEnabled) {
            gl.enable(gl.BLEND);
        }

        // -------------------------------------------------------------
        // Step B: Draw Background Vector Speed Field Overlay
        // -------------------------------------------------------------
        gl.useProgram(state.windProgram);

        const aPosition = gl.getAttribLocation(state.windProgram, 'a_position');
        gl.enableVertexAttribArray(aPosition);
        gl.bindBuffer(gl.ARRAY_BUFFER, state.windPositionBuffer);
        gl.vertexAttribPointer(aPosition, 2, gl.FLOAT, false, 0, 0);

        const aTexcoord = gl.getAttribLocation(state.windProgram, 'a_texcoord');
        gl.enableVertexAttribArray(aTexcoord);
        gl.bindBuffer(gl.ARRAY_BUFFER, state.windTexcoordBuffer);
        gl.vertexAttribPointer(aTexcoord, 2, gl.FLOAT, false, 0, 0);

        gl.uniformMatrix4fv(gl.getUniformLocation(state.windProgram, 'u_matrix'), false, matrix);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, windTexture);
        gl.uniform1i(gl.getUniformLocation(state.windProgram, 'u_texture'), 0);

        const opacity = parseFloat(DOM.opacitySlider.value) / 100;
        gl.uniform1f(gl.getUniformLocation(state.windProgram, 'u_opacity'), opacity);

        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

        gl.drawArrays(gl.TRIANGLES, 0, 6);

        gl.disableVertexAttribArray(aPosition);
        gl.disableVertexAttribArray(aTexcoord);

        // -------------------------------------------------------------
        // Step C: Draw Particle Trails from GPU State Texture
        // -------------------------------------------------------------
        gl.useProgram(state.particleProgram);

        const aPartUV = gl.getAttribLocation(state.particleProgram, 'a_particle_uv');
        gl.enableVertexAttribArray(aPartUV);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.particleUVBuffer);
        gl.vertexAttribPointer(aPartUV, 2, gl.FLOAT, false, 0, 0);

        gl.uniformMatrix4fv(gl.getUniformLocation(state.particleProgram, 'u_matrix'), false, matrix);

        // Bind current state texture to look up coordinates in vertex shader
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, currentUpdatedTex);
        gl.uniform1i(gl.getUniformLocation(state.particleProgram, 'u_state_texture'), 0);

        // Bind wind texture to look up velocity/speed in vertex shader
        gl.activeTexture(gl.TEXTURE1);
        gl.bindTexture(gl.TEXTURE_2D, windTexture);
        gl.uniform1i(gl.getUniformLocation(state.particleProgram, 'u_wind_texture'), 1);

        gl.uniform1f(gl.getUniformLocation(state.particleProgram, 'u_point_size'), 5.5);
        gl.uniform1f(gl.getUniformLocation(state.particleProgram, 'u_arrow_opacity'), opacity);

        gl.drawArrays(gl.POINTS, 0, this.numParticles * this.trailLength);

        gl.disableVertexAttribArray(aPartUV);
        gl.bindBuffer(gl.ARRAY_BUFFER, null);

        if (depthTestEnabled) {
            gl.enable(gl.DEPTH_TEST);
        }

        // Trigger map repaint to run the GPU animation loop continuously
        if (state.currentLayerMode === 'wind' && state.map) {
            state.map.triggerRepaint();
        }
    }

    onRemove(map, gl) {
        if (state.windProgram) {
            gl.deleteProgram(state.windProgram);
            state.windProgram = null;
        }
        if (state.particleProgram) {
            gl.deleteProgram(state.particleProgram);
            state.particleProgram = null;
        }
        if (this.updateProgram) {
            gl.deleteProgram(this.updateProgram);
            this.updateProgram = null;
        }
        if (state.windPositionBuffer) {
            gl.deleteBuffer(state.windPositionBuffer);
            state.windPositionBuffer = null;
        }
        if (state.windTexcoordBuffer) {
            gl.deleteBuffer(state.windTexcoordBuffer);
            state.windTexcoordBuffer = null;
        }
        if (this.quadBuffer) {
            gl.deleteBuffer(this.quadBuffer);
            this.quadBuffer = null;
        }
        if (this.particleUVBuffer) {
            gl.deleteBuffer(this.particleUVBuffer);
            this.particleUVBuffer = null;
        }
        for (let i = 0; i < 2; i++) {
            if (this.stateTextures[i]) {
                gl.deleteTexture(this.stateTextures[i]);
                this.stateTextures[i] = null;
            }
            if (this.stateFBOs[i]) {
                gl.deleteFramebuffer(this.stateFBOs[i]);
                this.stateFBOs[i] = null;
            }
        }
    }
}
