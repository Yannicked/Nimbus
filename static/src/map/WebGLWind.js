import { CONFIG } from '../config.js';
import { state } from '../state.js';
import { DOM } from '../ui/dom.js';
import { getOrLoadTexture } from './index.js';

export class WebGLWindLayer {
    constructor() {
        this.id = 'wind-webgl-layer';
        this.type = 'custom';
        this.renderingMode = '2d';
    }

    // Update Wind Pixel Data cache from loaded PNG image
    updateWindPixelData(img) {
        console.log("Extracting wind pixel data for CPU simulation...");
        const canvas = document.createElement('canvas');
        canvas.width = 700;
        canvas.height = 1530;
        const ctx = canvas.getContext('2d');
        ctx.drawImage(img, 0, 0);
        const imageData = ctx.getImageData(0, 0, 700, 1530);
        state.windPixelData = imageData.data;
    }

    // Sample u and v velocities at a Mercator coordinate
    getWindVelocity(mx, my) {
        if (!state.windPixelData) return [0, 0];
        
        // Bounding box: MERCATOR_LEFT: 0.0, MERCATOR_RIGHT: 1210000.0, MERCATOR_BOTTOM: 6250000.0, MERCATOR_TOP: 7560000.0
        const col = Math.floor((mx - 0.0) / 1210000.0 * 700);
        const row = Math.floor((7560000.0 - my) / (7560000.0 - 6250000.0) * 765);
        
        if (col < 0 || col >= 700 || row < 0 || row >= 765) {
            return [0, 0];
        }
        
        // Sample u (top half)
        const idx_u = (row * 700 + col) * 4;
        const r_u = state.windPixelData[idx_u];
        const g_u = state.windPixelData[idx_u + 1];
        const raw_u = r_u * 256 + g_u;
        if (raw_u >= 65535 || raw_u === 0) return [0, 0];
        const u = raw_u / 100.0 - 100.0;
        
        // Sample v (bottom half)
        const idx_v = (((row + 765) * 700) + col) * 4;
        const r_v = state.windPixelData[idx_v];
        const g_v = state.windPixelData[idx_v + 1];
        const raw_v = r_v * 256 + g_v;
        if (raw_v >= 65535 || raw_v === 0) return [0, 0];
        const v = raw_v / 100.0 - 100.0;
        
        return [u, v];
    }

    // Generate a random particle
    randomParticle() {
        const mx = Math.random() * 1210000.0;
        const my = 6250000.0 + Math.random() * (7560000.0 - 6250000.0);
        const maxAge = 150 + Math.random() * 150;
        const age = Math.random() * maxAge;
        const history = [];
        for (let i = 0; i < state.TRAIL_LENGTH; i++) {
            history.push({ mx: mx, my: my });
        }
        return {
            mx: mx,
            my: my,
            age: age,
            maxAge: maxAge,
            history: history,
            activeLength: 1,
            lastBreadcrumb: { mx: mx, my: my }
        };
    }

    // Reset particle properties in place to avoid GC churn
    resetParticle(p) {
        const mx = Math.random() * 1210000.0;
        const my = 6250000.0 + Math.random() * (7560000.0 - 6250000.0);
        const maxAge = 150 + Math.random() * 150;
        
        p.mx = mx;
        p.my = my;
        p.age = 0;
        p.maxAge = maxAge;
        p.activeLength = 1;
        p.lastBreadcrumb.mx = mx;
        p.lastBreadcrumb.my = my;
        
        for (let i = 0; i < state.TRAIL_LENGTH; i++) {
            p.history[i].mx = mx;
            p.history[i].my = my;
        }
    }

    // Initialize the particle list
    initParticles() {
        state.particles = [];
        for (let i = 0; i < state.maxParticles; i++) {
            state.particles.push(this.randomParticle());
        }
    }

    // Update particle positions based on wind velocities
    updateParticles(dt, minDistance) {
        const speedFactor = 2.5; // Controls the movement speed of particles
        
        for (let i = 0; i < state.particles.length; i++) {
            const p = state.particles[i];
            p.age += dt * 60; // Age in frames
            
            if (p.age >= p.maxAge) {
                this.resetParticle(p);
                continue;
            }
            
            const [u, v] = this.getWindVelocity(p.mx, p.my);
            
            // Update positions using velocity (meters per second)
            p.mx += u * dt * speedFactor * 1200.0;
            p.my += v * dt * speedFactor * 1200.0;
            
            // Bounds checking
            if (p.mx < 0.0 || p.mx > 1210000.0 || p.my < 6250000.0 || p.my > 7560000.0) {
                this.resetParticle(p);
                continue;
            }
            
            // Overwrite the head position to the current position
            p.history[0] = { mx: p.mx, my: p.my };
            
            // Push a new trail point if the head has moved far enough from the last recorded breadcrumb
            const dx = p.mx - p.lastBreadcrumb.mx;
            const dy = p.my - p.lastBreadcrumb.my;
            const dist = Math.sqrt(dx * dx + dy * dy);
            
            if (dist >= minDistance) {
                p.history.splice(1, 0, { mx: p.mx, my: p.my });
                p.activeLength = Math.min(p.activeLength + 1, state.TRAIL_LENGTH);
                p.lastBreadcrumb = { mx: p.mx, my: p.my };
                if (p.history.length > state.TRAIL_LENGTH) {
                    p.history.pop();
                }
            }
            
            // Collapse unused history points to the head position so they don't form a dot at the start
            for (let j = p.activeLength; j < state.TRAIL_LENGTH; j++) {
                p.history[j] = { mx: p.mx, my: p.my };
            }
        }
    }

    onAdd(mapInstance, gl) {
        state.glContext = gl;
        console.log("Initializing WebGL Wind Layer shaders and buffers...");
        
        // 1. Compile background color shader
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
            
            void main() {
                // Avoid border/interpolation artifacts by clamping coordinates to pixel centers
                float clamped_x = 0.5 / 700.0 + v_texcoord.x * (699.0 / 700.0);
                float clamped_y = 0.5 / 765.0 + v_texcoord.y * (764.0 / 765.0);
                
                // Top half: u-component, Bottom half: v-component
                vec2 texcoord_u = vec2(clamped_x, clamped_y * 0.5 + 0.5);
                vec2 texcoord_v = vec2(clamped_x, clamped_y * 0.5);
                
                vec4 tex_u = texture2D(u_texture, texcoord_u);
                vec4 tex_v = texture2D(u_texture, texcoord_v);
                
                if (tex_u.a < 0.99 || tex_v.a < 0.99) {
                    discard;
                }
                
                float u_raw = (tex_u.r * 255.0) * 256.0 + (tex_u.g * 255.0);
                float v_raw = (tex_v.r * 255.0) * 256.0 + (tex_v.g * 255.0);
                
                if (u_raw >= 65535.0 || v_raw >= 65535.0 || u_raw == 0.0 || v_raw == 0.0) {
                    discard;
                }
                
                float u = u_raw / 100.0 - 100.0;
                float v = v_raw / 100.0 - 100.0;
                float speed = sqrt(u * u + v * v);
                
                vec4 c = getColor(speed);
                gl_FragColor = vec4(c.rgb, c.a * u_opacity);
            }
        `;
        
        function compileShader(source, type) {
            const shader = gl.createShader(type);
            gl.shaderSource(shader, source);
            gl.compileShader(shader);
            if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
                console.error("Wind shader compilation error:", gl.getShaderInfoLog(shader));
            }
            return shader;
        }
        
        const vs = compileShader(vertexShaderSource, gl.VERTEX_SHADER);
        const fs = compileShader(fragmentShaderSource, gl.FRAGMENT_SHADER);
        
        state.windProgram = gl.createProgram();
        gl.attachShader(state.windProgram, vs);
        gl.attachShader(state.windProgram, fs);
        gl.linkProgram(state.windProgram);
        if (!gl.getProgramParameter(state.windProgram, gl.LINK_STATUS)) {
            console.error("Wind program linking error:", gl.getProgramInfoLog(state.windProgram));
        }
        
        // 2. Compile particle (arrows) shader program
        const particleVsSource = `
            attribute vec2 a_position;
            attribute float a_fade;
            attribute float a_trail;
            varying float v_fade;
            varying float v_trail;
            uniform mat4 u_matrix;
            uniform float u_point_size;
            void main() {
                gl_Position = u_matrix * vec4(a_position, 0.0, 1.0);
                gl_PointSize = u_point_size * (0.3 + 0.7 * a_trail);
                v_fade = a_fade;
                v_trail = a_trail;
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
        
        const pVs = compileShader(particleVsSource, gl.VERTEX_SHADER);
        const pFs = compileShader(particleFsSource, gl.FRAGMENT_SHADER);
        
        state.particleProgram = gl.createProgram();
        gl.attachShader(state.particleProgram, pVs);
        gl.attachShader(state.particleProgram, pFs);
        gl.linkProgram(state.particleProgram);
        if (!gl.getProgramParameter(state.particleProgram, gl.LINK_STATUS)) {
            console.error("Particle program linking error:", gl.getProgramInfoLog(state.particleProgram));
        }
        
        // 3. Set up Mercator quad buffers
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
        
        // 4. Set up dynamic buffer for particles
        state.particleBuffer = gl.createBuffer();
        
        // Seed particles on startup
        this.initParticles();
        state.lastAnimTime = performance.now();
    }
    
    render(gl, matrix) {
        if (!state.metadata || !state.windProgram || !state.particleProgram) return;
        
        const timeVal = state.metadata.times[state.currentTimeIndex];
        const texture = getOrLoadTexture(gl, timeVal);
        if (!texture) return; // Wait for texture load
        
        // 1. Update Particle positions on CPU
        const now = performance.now();
        let dt = (now - state.lastAnimTime) / 1000.0;
        if (dt > 0.1) dt = 0.1; // Cap dt to prevent warp jumps
        state.lastAnimTime = now;
        
        const zoom = state.map ? state.map.getZoom() : 6;
        const lat = 52.0;
        const metersPerPixel = 156543.03 * Math.cos(lat * Math.PI / 180) / Math.pow(2, zoom);
        const minDistance = 1.2 * metersPerPixel;
        
        if (state.windPixelData) {
            this.updateParticles(dt, minDistance);
        }
        
        // Disable depth test
        const depthTestEnabled = gl.isEnabled(gl.DEPTH_TEST);
        if (depthTestEnabled) {
            gl.disable(gl.DEPTH_TEST);
        }
        if (gl.bindVertexArray) {
            gl.bindVertexArray(null);
        }
        
        // -------------------------------------------------------------
        // Step A: Draw Background Vector Speed Field Overlay
        // -------------------------------------------------------------
        gl.useProgram(state.windProgram);
        
        // Bind position quad attributes
        const aPosition = gl.getAttribLocation(state.windProgram, 'a_position');
        gl.enableVertexAttribArray(aPosition);
        gl.bindBuffer(gl.ARRAY_BUFFER, state.windPositionBuffer);
        gl.vertexAttribPointer(aPosition, 2, gl.FLOAT, false, 0, 0);
        
        const aTexcoord = gl.getAttribLocation(state.windProgram, 'a_texcoord');
        gl.enableVertexAttribArray(aTexcoord);
        gl.bindBuffer(gl.ARRAY_BUFFER, state.windTexcoordBuffer);
        gl.vertexAttribPointer(aTexcoord, 2, gl.FLOAT, false, 0, 0);
        
        // Set uniforms
        gl.uniformMatrix4fv(gl.getUniformLocation(state.windProgram, 'u_matrix'), false, matrix);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.uniform1i(gl.getUniformLocation(state.windProgram, 'u_texture'), 0);
        
        const opacity = parseFloat(DOM.opacitySlider.value) / 100;
        gl.uniform1f(gl.getUniformLocation(state.windProgram, 'u_opacity'), opacity);
        
        // Blend mode configuration
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        
        gl.drawArrays(gl.TRIANGLES, 0, 6);
        
        gl.disableVertexAttribArray(aPosition);
        gl.disableVertexAttribArray(aTexcoord);
        
        // -------------------------------------------------------------
        // Step B: Draw Moving Particles (Arrows) on Top
        // -------------------------------------------------------------
        gl.useProgram(state.particleProgram);
        
        // Helper to convert Mercator meters to normalized coordinate space [0, 1]
        const MAP_LIMIT = 20037508.342789244;
        function toMercNormalized(x, y) {
            const ux = (x + MAP_LIMIT) / (2.0 * MAP_LIMIT);
            const uy = (MAP_LIMIT - y) / (2.0 * MAP_LIMIT);
            return [ux, uy];
        }
        
        // Pack active particle variables: positions, fade, trail factor
        const bufferData = new Float32Array(state.maxParticles * state.TRAIL_LENGTH * 4);
        let offset = 0;
        for (let i = 0; i < state.maxParticles; i++) {
            const p = state.particles[i];
            
            // Calculate fade envelope (sinusoidal fade in/out)
            const progress = Math.min(Math.max(p.age / p.maxAge, 0.0), 1.0);
            const fade = Math.sin(progress * Math.PI);
            
            for (let j = 0; j < state.TRAIL_LENGTH; j++) {
                const pos = p.history[j];
                const [ux, uy] = toMercNormalized(pos.mx, pos.my);
                const trailFactor = 1.0 - (j / (state.TRAIL_LENGTH - 1));
                
                bufferData[offset++] = ux;
                bufferData[offset++] = uy;
                bufferData[offset++] = fade;
                bufferData[offset++] = trailFactor;
            }
        }
        
        // Upload dynamic particle buffer data to VBO
        gl.bindBuffer(gl.ARRAY_BUFFER, state.particleBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, bufferData, gl.DYNAMIC_DRAW);
        
        // Set attributes
        const stride = 16; // 4 floats * 4 bytes/float = 16
        const aPartPos = gl.getAttribLocation(state.particleProgram, 'a_position');
        gl.enableVertexAttribArray(aPartPos);
        gl.vertexAttribPointer(aPartPos, 2, gl.FLOAT, false, stride, 0);
        
        const aPartFade = gl.getAttribLocation(state.particleProgram, 'a_fade');
        gl.enableVertexAttribArray(aPartFade);
        gl.vertexAttribPointer(aPartFade, 1, gl.FLOAT, false, stride, 8);
        
        const aPartTrail = gl.getAttribLocation(state.particleProgram, 'a_trail');
        gl.enableVertexAttribArray(aPartTrail);
        gl.vertexAttribPointer(aPartTrail, 1, gl.FLOAT, false, stride, 12);
        
        // Set uniforms
        gl.uniformMatrix4fv(gl.getUniformLocation(state.particleProgram, 'u_matrix'), false, matrix);
        // Base streak point size: 7.5px for the head
        gl.uniform1f(gl.getUniformLocation(state.particleProgram, 'u_point_size'), 7.5);
        gl.uniform1f(gl.getUniformLocation(state.particleProgram, 'u_arrow_opacity'), 0.85);
        
        // Draw particle arrays
        gl.drawArrays(gl.POINTS, 0, state.maxParticles * state.TRAIL_LENGTH);
        
        // Clean attributes
        gl.disableVertexAttribArray(aPartPos);
        gl.disableVertexAttribArray(aPartFade);
        gl.disableVertexAttribArray(aPartTrail);
        gl.bindBuffer(gl.ARRAY_BUFFER, null);
        
        if (depthTestEnabled) {
            gl.enable(gl.DEPTH_TEST);
        }
        
        // Trigger repaint to run animation loop if Wind is the active layer
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
        if (state.windPositionBuffer) {
            gl.deleteBuffer(state.windPositionBuffer);
            state.windPositionBuffer = null;
        }
        if (state.windTexcoordBuffer) {
            gl.deleteBuffer(state.windTexcoordBuffer);
            state.windTexcoordBuffer = null;
        }
        if (state.particleBuffer) {
            gl.deleteBuffer(state.particleBuffer);
            state.particleBuffer = null;
        }
    }
}
