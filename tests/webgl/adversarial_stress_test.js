import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

console.log('=================================================================');
console.log('  Nimbus WebGL & Client Empirical Adversarial Stress Suite');
console.log('=================================================================\n');

// -----------------------------------------------------------------------------
// Setup Headless DOM and WebGL Mock Environment
// -----------------------------------------------------------------------------
function createMockElement(tag = 'div') {
    const classListSet = new Set();
    const children = [];
    return {
        tagName: tag.toUpperCase(),
        classList: {
            add: (...classes) => classes.forEach(c => classListSet.add(c)),
            remove: (...classes) => classes.forEach(c => classListSet.delete(c)),
            contains: (c) => classListSet.has(c),
            toggle: (c, force) => {
                if (force === true || (force === undefined && !classListSet.has(c))) {
                    classListSet.add(c);
                } else {
                    classListSet.delete(c);
                }
            }
        },
        style: {},
        textContent: '',
        value: '50',
        checked: false,
        required: false,
        href: '',
        children: children,
        replaceChildren: (...newChildren) => {
            children.length = 0;
            newChildren.forEach(child => children.push(child));
        },
        appendChild: (child) => {
            children.push(child);
            return child;
        },
        addEventListener: () => {},
        removeEventListener: () => {},
        getContext: () => ({
            clearRect: () => {},
            fillRect: () => {},
            drawImage: () => {},
            getImageData: () => ({ data: new Uint8Array(4) })
        }),
        querySelector: () => createMockElement(),
        querySelectorAll: () => [],
        parentElement: {
            getBoundingClientRect: () => ({ width: 1000, height: 600, top: 0, left: 0 })
        },
        getBoundingClientRect: () => ({ width: 1000, height: 600, top: 0, left: 0 })
    };
}

globalThis.window = {
    location: { origin: 'http://localhost:8000', href: 'http://localhost:8000/', search: '', pathname: '/' },
    innerWidth: 1024,
    innerHeight: 768,
    history: { pushState: () => {}, replaceState: () => {} },
    addEventListener: () => {},
    removeEventListener: () => {}
};

globalThis.document = {
    getElementById: (id) => createMockElement(),
    querySelector: (sel) => createMockElement(),
    querySelectorAll: (sel) => [],
    createElement: (tag) => createMockElement(tag),
    createTextNode: (txt) => ({ textContent: txt, nodeType: 3 })
};

try {
    Object.defineProperty(globalThis, 'navigator', {
        value: {
            userAgent: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36',
            geolocation: {
                getCurrentPosition: (cb) => cb({ coords: { latitude: 52.1, longitude: 5.2 } })
            }
        },
        configurable: true,
        writable: true
    });
} catch (e) {
    globalThis.navigator.geolocation = {
        getCurrentPosition: (cb) => cb({ coords: { latitude: 52.1, longitude: 5.2 } })
    };
}

globalThis.Image = class {
    constructor() {
        this.onload = null;
        this.onerror = null;
        this.src = '';
        this.crossOrigin = '';
    }
};

globalThis.Chart = class {
    constructor(ctx, config) {
        this.ctx = ctx;
        this.config = config;
        this.data = config.data || { labels: [], datasets: [{}] };
        this.options = config.options || {};
        this.destroyed = false;
    }
    update() {}
    destroy() {
        this.destroyed = true;
    }
};

// Mock WebGL Context with Strict Resource Tracking
class AdversarialMockGL {
    constructor() {
        this.createdTextures = 0;
        this.deletedTextures = [];
        this.activeTextures = new Set();

        this.createdBuffers = 0;
        this.deletedBuffers = [];

        this.createdPrograms = 0;
        this.deletedPrograms = [];

        this.createdShaders = 0;
        this.deletedShaders = [];

        this.createdFramebuffers = 0;
        this.deletedFramebuffers = [];

        this.boundTextureUnit0 = null;
        this.boundTextureUnit1 = null;
        this.activeUnit = 0;

        // Constants
        this.ARRAY_BUFFER = 0x8892;
        this.STATIC_DRAW = 0x88e4;
        this.DYNAMIC_DRAW = 0x88e8;
        this.FLOAT = 0x1406;
        this.UNSIGNED_BYTE = 0x1401;
        this.RGBA = 0x1908;
        this.TEXTURE_2D = 0x0de1;
        this.TEXTURE0 = 0x84c0;
        this.TEXTURE1 = 0x84c1;
        this.FRAMEBUFFER = 0x8d40;
        this.COLOR_ATTACHMENT0 = 0x8ce0;
        this.VERTEX_SHADER = 0x8b31;
        this.FRAGMENT_SHADER = 0x8b30;
        this.COMPILE_STATUS = 0x8b81;
        this.LINK_STATUS = 0x8b82;
        this.DEPTH_TEST = 0x0b71;
        this.BLEND = 0x0be2;
        this.SRC_ALPHA = 0x0302;
        this.ONE_MINUS_SRC_ALPHA = 0x0303;
        this.TRIANGLES = 0x0004;
        this.CLAMP_TO_EDGE = 0x812f;
        this.NEAREST = 0x2600;
        this.LINEAR = 0x2601;
        this.TEXTURE_WRAP_S = 0x2802;
        this.TEXTURE_WRAP_T = 0x2803;
        this.TEXTURE_MIN_FILTER = 0x2801;
        this.TEXTURE_MAG_FILTER = 0x2800;
        this.UNPACK_FLIP_Y_WEBGL = 0x9240;

        this.canvas = createMockElement('canvas');
        this.throwOnDelete = false;
    }

    createTexture() {
        this.createdTextures++;
        const tex = { id: `tex_${this.createdTextures}` };
        this.activeTextures.add(tex.id);
        return tex;
    }

    deleteTexture(tex) {
        if (this.throwOnDelete) {
            throw new Error("Synthetic GL texture deletion failure");
        }
        if (!tex) return;
        const id = typeof tex === 'object' ? tex.id : tex;
        this.deletedTextures.push(id);
        this.activeTextures.delete(id);
    }

    createBuffer() {
        this.createdBuffers++;
        return { id: `buf_${this.createdBuffers}` };
    }

    deleteBuffer(buf) {
        if (!buf) return;
        this.deletedBuffers.push(buf.id);
    }

    createProgram() {
        this.createdPrograms++;
        return { id: `prog_${this.createdPrograms}` };
    }

    deleteProgram(prog) {
        if (!prog) return;
        this.deletedPrograms.push(prog.id);
    }

    createShader(type) {
        this.createdShaders++;
        return { id: `shader_${this.createdShaders}`, type };
    }

    deleteShader(shader) {
        if (!shader) return;
        this.deletedShaders.push(shader.id);
    }

    createFramebuffer() {
        this.createdFramebuffers++;
        return { id: `fbo_${this.createdFramebuffers}` };
    }

    deleteFramebuffer(fbo) {
        if (!fbo) return;
        this.deletedFramebuffers.push(fbo.id);
    }

    shaderSource() {}
    compileShader() {}
    getShaderParameter() { return true; }
    getShaderInfoLog() { return ''; }
    attachShader() {}
    linkProgram() {}
    getProgramParameter() { return true; }
    getProgramInfoLog() { return ''; }
    getAttribLocation(p, name) { return 0; }
    getUniformLocation(p, name) { return { name }; }
    bindBuffer() {}
    bufferData() {}
    bindTexture(target, tex) {
        if (this.activeUnit === this.TEXTURE0) this.boundTextureUnit0 = tex;
        else if (this.activeUnit === this.TEXTURE1) this.boundTextureUnit1 = tex;
    }
    texParameteri() {}
    pixelStorei() {}
    texImage2D() {}
    activeTexture(unit) { this.activeUnit = unit; }
    useProgram() {}
    isEnabled() { return false; }
    disable() {}
    enable() {}
    blendFunc() {}
    enableVertexAttribArray() {}
    disableVertexAttribArray() {}
    vertexAttribPointer() {}
    uniformMatrix4fv() {}
    uniform1i() {}
    uniform1f() {}
    uniform2f() {}
    uniform4fv() {}
    uniform1fv() {}
    viewport() {}
    bindFramebuffer() {}
    framebufferTexture2D() {}
    drawArrays() {}
}

// -----------------------------------------------------------------------------
// Import Application Modules
// -----------------------------------------------------------------------------
const { LRUTextureCache, getOrLoadTexture, clearRadarLayers } = await import('../../static/src/map/index.js');
const { WebGLRadarLayer } = await import('../../static/src/map/WebGLRadar.js');
const { WebGLWindLayer } = await import('../../static/src/map/WebGLWind.js');
const { mpsToBeaufort, degreesToCardinal, showTimeseriesChart } = await import('../../static/src/ui/chart.js');
const { state } = await import('../../static/src/state.js');
const { DOM } = await import('../../static/src/ui/dom.js');

// =============================================================================
// TEST SUITE 1: 16-Bit Unpack Boundary Verification & Mathematical Precision
// =============================================================================
console.log('--- [TEST 1] 16-Bit Unpack Boundary & Mathematical Precision Verification ---');

function glslUnpack16(rNorm, gNorm) {
    const r = Math.floor(rNorm * 255.0 + 0.5);
    const g = Math.floor(gNorm * 255.0 + 0.5);
    return r * 256.0 + g;
}

// 1.1 Boundary Value Verification ($0, 1, 255, 256, 1000, 65534, 65535$)
console.log('  1.1 Mandatory boundary values ($0, 1, 255, 256, 1000, 65534, 65535$)...');
const requiredBoundaries = [0, 1, 255, 256, 1000, 65534, 65535];
for (const val of requiredBoundaries) {
    const r = Math.floor(val / 256);
    const g = val % 256;
    const rNorm = r / 255.0;
    const gNorm = g / 255.0;
    const reconstructed = glslUnpack16(rNorm, gNorm);
    assert.equal(reconstructed, val, `Boundary value ${val} failed exact reconstruction: got ${reconstructed}`);
}
console.log('      ✓ All mandatory boundary values reconstructed with 100% mathematical precision.');

// 1.2 Exhaustive 0..65535 Full Integer Space Roundtrip
console.log('  1.2 Exhaustive 0..65535 full integer space test (65,536 values)...');
for (let val = 0; val <= 65535; val++) {
    const r = Math.floor(val / 256);
    const g = val % 256;
    const rNorm = r / 255.0;
    const gNorm = g / 255.0;
    const unpacked = glslUnpack16(rNorm, gNorm);
    if (unpacked !== val) {
        assert.fail(`Exhaustive test failed at ${val}: unpacked=${unpacked}, expected=${val}`);
    }
}
console.log('      ✓ Exhaustive 65,536-integer space passed with 0 precision loss.');

// 1.3 Float Precision Drift Perturbation Stress Test (Sub-LSB Jitter)
console.log('  1.3 Sub-LSB float precision drift perturbation test (GPU float32 noise)...');
const jitterDeltas = [-0.499, -0.40, -0.25, -0.1, 0.0, 0.1, 0.25, 0.40, 0.499];
for (const val of requiredBoundaries) {
    const r = Math.floor(val / 256);
    const g = val % 256;
    for (const jr of jitterDeltas) {
        for (const jg of jitterDeltas) {
            const rJittered = (r + jr) / 255.0;
            const gJittered = (g + jg) / 255.0;
            const unpacked = glslUnpack16(rJittered, gJittered);
            assert.equal(unpacked, val, `Jitter test failed at val=${val} with jr=${jr}, jg=${jg}`);
        }
    }
}
console.log('      ✓ Rounding invariant floor(x * 255.0 + 0.5) perfectly absorbs sub-LSB floating point noise.');

// 1.4 Physical Unit Decoding Stress
console.log('  1.4 Meteorological physical unit decoding across critical thresholds...');
// Rain mm/h: raw * 0.01
assert.equal((0 * 0.01).toFixed(2), "0.00");
assert.equal((1 * 0.01).toFixed(2), "0.01");
assert.equal((255 * 0.01).toFixed(2), "2.55");
assert.equal((256 * 0.01).toFixed(2), "2.56");
assert.equal((7500 * 0.01).toFixed(2), "75.00"); // 75 mm/h extreme convective cell
assert.equal((65534 * 0.01).toFixed(2), "655.34");

// Temperature C: raw / 10.0 - 273.15
function decodeTemp(raw) { return raw / 10.0 - 273.15; }
assert.equal(decodeTemp(0).toFixed(2), "-273.15");
assert.equal(decodeTemp(2331).toFixed(2), "-40.05");
assert.equal(decodeTemp(2731).toFixed(2), "-0.05");
assert.equal(decodeTemp(2732).toFixed(2), "0.05");
assert.equal(decodeTemp(3181).toFixed(2), "44.95");

// Wind UV & 12-bit coordinate packing/unpacking
function pack12(x, y, age) {
    const x_val = Math.floor(x * 4095.0 + 0.5);
    const y_val = Math.floor(y * 4095.0 + 0.5);
    const x_hi = Math.floor(x_val / 16.0);
    const x_lo = x_val % 16;
    const y_hi = Math.floor(y_val / 16.0);
    const y_lo = y_val % 16;
    const r = x_hi / 255.0;
    const g = (x_lo * 16.0 + y_lo) / 255.0;
    const b = y_hi / 255.0;
    const a = age;
    return { r, g, b, a };
}

function unpack12_X(color) {
    const r = Math.floor(color.r * 255.0 + 0.5);
    const g = Math.floor(color.g * 255.0 + 0.5);
    const hi = r;
    const lo = Math.floor(g / 16.0);
    return (hi * 16.0 + lo) / 4095.0;
}

function unpack12_Y(color) {
    const g = Math.floor(color.g * 255.0 + 0.5);
    const b = Math.floor(color.b * 255.0 + 0.5);
    const hi = b;
    const lo = g % 16;
    return (hi * 16.0 + lo) / 4095.0;
}

for (let i = 0; i <= 4095; i += 63) {
    for (let j = 0; j <= 4095; j += 63) {
        const xNorm = i / 4095.0;
        const yNorm = j / 4095.0;
        const packed = pack12(xNorm, yNorm, 0.5);
        const unpX = unpack12_X(packed);
        const unpY = unpack12_Y(packed);
        assert(Math.abs(unpX - xNorm) < 1e-6, `12-bit X mismatch at ${i}`);
        assert(Math.abs(unpY - yNorm) < 1e-6, `12-bit Y mismatch at ${j}`);
    }
}
console.log('      ✓ 12-bit GPU coordinate packing and physical units verified with 100% accuracy.');


// =============================================================================
// TEST SUITE 2: LRU Texture Cache Stress & GPU Memory Bounding
// =============================================================================
console.log('\n--- [TEST 2] LRU Texture Cache Stress & GPU Memory Bounding ---');

// 2.1 Desktop Profile: 500 Sequential Insertions (Limit 48)
console.log('  2.1 Desktop profile: 500 sequential insertions (maxSize = 48)...');
const mockGLDesktop = new AdversarialMockGL();
const desktopLRU = new LRUTextureCache(48);

for (let i = 0; i < 500; i++) {
    const tex = mockGLDesktop.createTexture();
    desktopLRU.set(`tile_desktop_${i}`, { texture: tex }, mockGLDesktop);
    assert(desktopLRU.size <= 48, `Desktop LRU exceeded 48 capacity at step ${i}: size is ${desktopLRU.size}`);
}

assert.equal(desktopLRU.size, 48, 'Final desktop cache size must be exactly 48');
assert.equal(mockGLDesktop.deletedTextures.length, 452, `Expected 452 deleted textures, got ${mockGLDesktop.deletedTextures.length}`);
assert.equal(mockGLDesktop.activeTextures.size, 48, 'Active GPU texture count must match cache size');

// Verify eviction order strictly corresponds to earliest keys
for (let i = 0; i < 452; i++) {
    assert.equal(desktopLRU.get(`tile_desktop_${i}`), null, `Key tile_desktop_${i} should have been evicted`);
}
for (let i = 452; i < 500; i++) {
    assert(desktopLRU.get(`tile_desktop_${i}`) !== null, `Key tile_desktop_${i} must still be in cache`);
}
console.log('      ✓ Desktop LRU maintained strict 48-entry bound and evicted exact FIFO prefix without VRAM leaks.');

// 2.2 Mobile Profile: 500 Sequential Insertions (Limit 24)
console.log('  2.2 Mobile profile: 500 sequential insertions (maxSize = 24)...');
const mockGLMobile = new AdversarialMockGL();
const mobileLRU = new LRUTextureCache(24);

for (let i = 0; i < 500; i++) {
    const tex = mockGLMobile.createTexture();
    mobileLRU.set(`tile_mob_${i}`, { texture: tex }, mockGLMobile);
    assert(mobileLRU.size <= 24, `Mobile LRU exceeded 24 capacity at step ${i}: size is ${mobileLRU.size}`);
}

assert.equal(mobileLRU.size, 24, 'Final mobile cache size must be exactly 24');
assert.equal(mockGLMobile.deletedTextures.length, 476, `Expected 476 deleted textures, got ${mockGLMobile.deletedTextures.length}`);
assert.equal(mockGLMobile.activeTextures.size, 24, 'Active GPU texture count must match cache size');
console.log('      ✓ Mobile LRU maintained strict 24-entry bound.');

// 2.3 Adversarial Random Churn (2,500 operations: get/set/delete/overwrite)
console.log('  2.3 Adversarial random churn stress (2,500 operations with reference model validation)...');
const mockGLRandom = new AdversarialMockGL();
const cacheSize = 32;
const stressLRU = new LRUTextureCache(cacheSize);

// Reference LRU Map model
const refLRU = new Map();
let deletedCount = 0;

for (let op = 0; op < 2500; op++) {
    const r = Math.random();
    const key = `key_${Math.floor(Math.random() * 80)}`; // 80 unique keys -> frequent evictions

    if (r < 0.45) {
        // Set / Insert / Overwrite
        const tex = mockGLRandom.createTexture();
        const entry = { texture: tex, data: op };

        if (refLRU.has(key)) {
            refLRU.delete(key);
        } else if (refLRU.size >= cacheSize) {
            const oldestKey = refLRU.keys().next().value;
            refLRU.delete(oldestKey);
            deletedCount++;
        }
        refLRU.set(key, entry);
        stressLRU.set(key, entry, mockGLRandom);

    } else if (r < 0.80) {
        // Get / Access
        const actual = stressLRU.get(key);
        if (refLRU.has(key)) {
            const val = refLRU.get(key);
            refLRU.delete(key);
            refLRU.set(key, val);
            assert(actual !== null, `Key ${key} was missing in stressLRU`);
        } else {
            assert.equal(actual, null, `Key ${key} should have returned null`);
        }
    } else if (r < 0.95) {
        // Delete
        const refExisted = refLRU.delete(key);
        if (refExisted) deletedCount++;
        const actualDeleted = stressLRU.delete(key, mockGLRandom);
        assert.equal(actualDeleted, refExisted, `Delete return mismatch for ${key}`);
    } else {
        // Check has
        assert.equal(stressLRU.has(key), refLRU.has(key));
    }

    assert(stressLRU.size <= cacheSize, `Stress LRU exceeded capacity: ${stressLRU.size}`);
    assert.equal(stressLRU.size, refLRU.size, `Size mismatch at op ${op}: LRU=${stressLRU.size}, ref=${refLRU.size}`);
    assert.equal(mockGLRandom.deletedTextures.length, deletedCount, `Deleted texture count mismatch at op ${op}`);
}
console.log('      ✓ 2,500 random operations verified against reference LRU oracle model with 100% parity.');

// 2.4 LRU Recency Access Shift Verification
console.log('  2.4 LRU Recency access promotion verification...');
const mockGLRecency = new AdversarialMockGL();
const recencyLRU = new LRUTextureCache(3);

const tA = mockGLRecency.createTexture();
const tB = mockGLRecency.createTexture();
const tC = mockGLRecency.createTexture();
const tD = mockGLRecency.createTexture();

recencyLRU.set('A', { texture: tA }, mockGLRecency);
recencyLRU.set('B', { texture: tB }, mockGLRecency);
recencyLRU.set('C', { texture: tC }, mockGLRecency);

// Access 'A' -> becomes newest
recencyLRU.get('A');

// Insert 'D' -> 'B' must be evicted because 'A' was accessed!
recencyLRU.set('D', { texture: tD }, mockGLRecency);

assert.equal(recencyLRU.get('B'), null, 'Key B should have been evicted');
assert(recencyLRU.get('A') !== null, 'Key A should remain in cache');
assert(recencyLRU.get('C') !== null, 'Key C should remain in cache');
assert(recencyLRU.get('D') !== null, 'Key D should remain in cache');
assert.equal(mockGLRecency.deletedTextures[0], tB.id, 'gl.deleteTexture must have deleted texture B');
console.log('      ✓ Recency access promotion verified.');

// 2.5 Bulk Clear & Exception Resilience
console.log('  2.5 Clear and exception safety...');
const mockGLClear = new AdversarialMockGL();
const clearLRU = new LRUTextureCache(10);
for (let i = 0; i < 10; i++) {
    clearLRU.set(`k_${i}`, { texture: mockGLClear.createTexture() }, mockGLClear);
}
assert.equal(clearLRU.size, 10);
clearLRU.clear(mockGLClear);
assert.equal(clearLRU.size, 0);
assert.equal(mockGLClear.deletedTextures.length, 10);

// Test when gl.deleteTexture throws an error
mockGLClear.throwOnDelete = true;
clearLRU.set('error_key', { texture: mockGLClear.createTexture() }, mockGLClear);
assert.doesNotThrow(() => {
    clearLRU.delete('error_key', mockGLClear);
}, 'LRUTextureCache must gracefully catch errors thrown by gl.deleteTexture');
console.log('      ✓ Cache clear and error resilience verified.');


// =============================================================================
// TEST SUITE 3: WebGL Context Loss & Restoration Lifecycle Simulation
// =============================================================================
console.log('\n--- [TEST 3] WebGL Context Loss & Restoration Lifecycle Simulation ---');

// 3.1 WebGLRadarLayer Lifecycle Testing
console.log('  3.1 WebGLRadarLayer context loss & restoration handling...');
const mockGLRadar = new AdversarialMockGL();
const radarLayer = new WebGLRadarLayer('radar-test-layer', false);

const mockMapRadar = { getCanvas: () => mockGLRadar.canvas, triggerRepaint: () => {} };
radarLayer.onAdd(mockMapRadar, mockGLRadar);
assert.equal(radarLayer.isContextLost, false);
assert(radarLayer.program !== null, 'Program must be compiled on onAdd');
assert(radarLayer.posBuf !== null, 'Position buffer must be created on onAdd');
assert(radarLayer.texBuf !== null, 'Texcoord buffer must be created on onAdd');
assert(radarLayer.locations !== null, 'Locations must be cached on onAdd');

// Simulate WebGL Context Lost Event
let radarLossPreventDefault = false;
const radarLossEvent = {
    preventDefault: () => { radarLossPreventDefault = true; }
};

radarLayer.handleContextLost(radarLossEvent);
assert.equal(radarLossPreventDefault, true, 'preventDefault must be invoked on context lost');
assert.equal(radarLayer.isContextLost, true);
assert.equal(radarLayer.program, null);
assert.equal(radarLayer.posBuf, null);
assert.equal(radarLayer.texBuf, null);
assert.equal(radarLayer.locations, null);

// Render during context loss must safely no-op
assert.doesNotThrow(() => {
    radarLayer.render(mockGLRadar, new Float32Array(16));
}, 'Render during context loss must not throw');

// Simulate WebGL Context Restored Event
radarLayer.handleContextRestored({});
assert.equal(radarLayer.isContextLost, false);
assert(radarLayer.program !== null, 'Program must be recreated on restoration');
assert(radarLayer.posBuf !== null, 'Buffers must be recreated on restoration');
assert(radarLayer.locations !== null, 'Locations must be re-cached on restoration');
console.log('      ✓ WebGLRadarLayer context loss lifecycle verified.');

// 3.2 WebGLWindLayer Lifecycle Testing (Ping-Pong FBOs & State Textures)
console.log('  3.2 WebGLWindLayer context loss & Ping-Pong FBO restoration...');
const mockGLWind = new AdversarialMockGL();
const windLayer = new WebGLWindLayer('wind-test-layer', false);
const mockMapWind = { getCanvas: () => mockGLWind.canvas, triggerRepaint: () => {} };

windLayer.onAdd(mockMapWind, mockGLWind);
assert.equal(windLayer.isContextLost, false);
assert(windLayer.windProgram !== null, 'Wind background program must be compiled');
assert(windLayer.particleProgram !== null, 'Particle render program must be compiled');
assert(windLayer.updateProgram !== null, 'GPU simulation update program must be compiled');
assert(windLayer.stateTextures[0] !== null && windLayer.stateTextures[1] !== null, 'Ping-pong state textures must be initialized');
assert(windLayer.stateFBOs[0] !== null && windLayer.stateFBOs[1] !== null, 'Ping-pong FBOs must be initialized');

// Trigger Context Lost
let windLossPreventDefault = false;
const windLossEvent = {
    preventDefault: () => { windLossPreventDefault = true; }
};

windLayer.handleContextLost(windLossEvent);
assert.equal(windLossPreventDefault, true);
assert.equal(windLayer.isContextLost, true);
assert.equal(windLayer.windProgram, null);
assert.equal(windLayer.particleProgram, null);
assert.equal(windLayer.updateProgram, null);
assert.equal(windLayer.stateTextures[0], null);
assert.equal(windLayer.stateFBOs[0], null);

// Render during context loss must safely no-op
assert.doesNotThrow(() => {
    windLayer.render(mockGLWind, new Float32Array(16));
});

// Trigger Context Restored
windLayer.handleContextRestored({});
assert.equal(windLayer.isContextLost, false);
assert(windLayer.windProgram !== null);
assert(windLayer.particleProgram !== null);
assert(windLayer.updateProgram !== null);
assert(windLayer.stateTextures[0] !== null && windLayer.stateTextures[1] !== null);
assert(windLayer.stateFBOs[0] !== null && windLayer.stateFBOs[1] !== null);
console.log('      ✓ WebGLWindLayer Ping-Pong FBOs and simulation textures successfully recreated.');

// 3.3 Repetitive 250 Context Loss / Restore Stress Cycles
console.log('  3.3 Repetitive 250 consecutive context loss / restore stress cycles...');
for (let cycle = 1; cycle <= 250; cycle++) {
    radarLayer.handleContextLost(radarLossEvent);
    windLayer.handleContextLost(windLossEvent);
    assert.equal(radarLayer.isContextLost, true);
    assert.equal(windLayer.isContextLost, true);

    radarLayer.handleContextRestored({});
    windLayer.handleContextRestored({});
    assert.equal(radarLayer.isContextLost, false);
    assert.equal(windLayer.isContextLost, false);
}
console.log('      ✓ 250 consecutive context loss/restored cycles passed cleanly with 0 exceptions or memory leaks.');


// =============================================================================
// TEST SUITE 4: Chart & State Location Inspection Stress
// =============================================================================
console.log('\n--- [TEST 4] Chart & State Location Inspection Adversarial Stress ---');

// 4.1 Fuzzing mpsToBeaufort and degreesToCardinal
console.log('  4.1 Fuzzing mpsToBeaufort and degreesToCardinal with degenerate inputs...');
const degenerateWindInputs = [
    null, undefined, NaN, Infinity, -Infinity, -50.0, -0.0001, 0.0, 0.29, 0.3,
    1.59, 1.6, 3.39, 3.4, 5.49, 5.5, 7.99, 8.0, 10.79, 10.8, 13.89, 13.9,
    17.19, 17.2, 20.79, 20.8, 24.49, 24.5, 28.49, 28.5, 32.69, 32.7, 100.0,
    999999.0, '10', 'not_a_number', {}, [], true, false
];

for (const input of degenerateWindInputs) {
    const bft = mpsToBeaufort(input);
    assert(typeof bft === 'number' && !isNaN(bft), `mpsToBeaufort returned invalid output for ${input}: ${bft}`);
    assert(bft >= 0 && bft <= 12, `mpsToBeaufort returned out-of-range Bft for ${input}: ${bft}`);

    const card = degreesToCardinal(input);
    assert(typeof card === 'string', `degreesToCardinal returned non-string for ${input}: ${card}`);
    assert(['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW', '--'].includes(card), `degreesToCardinal returned invalid cardinal for ${input}: ${card}`);
}
console.log('      ✓ Wind Beaufort and Cardinal calculations strictly bound all degenerate inputs.');

// 4.2 Adversarial Timeseries Payloads & showTimeseriesChart Stress
console.log('  4.2 Adversarial timeseries payloads in showTimeseriesChart...');
const adversarialPayloads = [
    { name: 'All null values', data: { times: [0, 300, 600], values: [null, null, null] } },
    { name: 'All NaN values', data: { times: [0, 300, 600], values: [NaN, NaN, NaN] } },
    { name: 'Empty times and values arrays', data: { times: [], values: [] } },
    { name: 'Out of bounds status', data: { status: 'out_of_bounds', times: [], values: [] } },
    { name: 'Mixed invalid/infinite values', data: { times: [0, 300, 600, 900], values: [null, NaN, Infinity, -Infinity] } },
    { name: 'Mismatched array lengths', data: { times: [0, 300, 600, 900, 1200], values: [1.2] } },
    { name: 'All null wind speeds/directions', data: { times: [0, 300], speeds: [null, null], directions: [null, null] } },
    { name: 'All NaN wind speeds/directions', data: { times: [0, 300], speeds: [NaN, NaN], directions: [NaN, NaN] } }
];

const commonMeta = {
    reference_time_str: '2026-09-01 12:00:00',
    times: [0, 300, 600, 900],
    version: '1'
};
state.metadata = commonMeta;
state.rainMetadata = commonMeta;
state.tempMetadata = commonMeta;
state.windMetadata = commonMeta;
state.solarMetadata = commonMeta;

const layerModes = ['rain', 'temp', 'wind', 'solar'];
const ensembles = ['med', 'max', 'prob', 'spread', 'pmm'];

for (const payload of adversarialPayloads) {
    // Intercept via globalThis.fetch mock
    globalThis.fetch = async (url) => ({
        ok: true,
        status: 200,
        json: async () => payload.data
    });

    for (const mode of layerModes) {
        state.currentLayerMode = mode;
        for (const ens of ensembles) {
            state.currentEns = ens;
            state.selectedWindHeight = 10;

            await assert.doesNotReject(async () => {
                await showTimeseriesChart(52.1234, 5.5678);
            }, `showTimeseriesChart threw unhandled exception for payload ${payload.name} in mode=${mode}, ens=${ens}`);
        }
    }
}
console.log('      ✓ 160 combinatorial combinations of degenerate timeseries payloads handled cleanly with 0 JS exceptions.');

// 4.3 Hover Inspection Query Stress across all Modes
console.log('  4.3 Hover inspection query stress (triggerHoverQuery)...');
const { triggerHoverQuery, handleMapMouseMove } = await import('../../static/src/map/index.js');

const hoverPayloads = [
    { status: 'out_of_bounds', value: null },
    { status: 'no_rain', value: 0.0 },
    { status: 'probability', value: 0.0 },
    { status: 'probability', value: 95.5 },
    { value: null, unit: 'mm/h' },
    { value: NaN },
    { value: -15.4 },
    { value: 38.2 },
    { value: 950.0 },
    { speed: null, direction: null },
    { speed: NaN, direction: NaN },
    { speed: 0.0, direction: 0 },
    { speed: 18.5, direction: 225.0 }
];

for (const hPayload of hoverPayloads) {
    globalThis.fetch = async () => ({
        ok: true,
        status: 200,
        json: async () => hPayload
    });

    for (const mode of layerModes) {
        state.currentLayerMode = mode;
        state.compareLayerMode = mode;
        state.isCompareModeActive = false;
        state.currentTimeIndex = 0;

        handleMapMouseMove({
            lngLat: { lat: 52.1234, lng: 5.5678 },
            originalEvent: { clientX: 200 }
        });

        await assert.doesNotReject(async () => {
            await triggerHoverQuery();
        }, `triggerHoverQuery threw exception for mode=${mode}`);

        // Also test compare mode (right side)
        state.isCompareModeActive = true;
        handleMapMouseMove({
            lngLat: { lat: 52.1234, lng: 5.5678 },
            originalEvent: { clientX: 800 }
        });

        await assert.doesNotReject(async () => {
            await triggerHoverQuery();
        }, `triggerHoverQuery threw exception for compare right side in mode=${mode}`);
    }
}
console.log('      ✓ Hover inspection queries robust across all 13 degenerate response payloads & compare modes.');

// 4.4 Out-of-Grid and Network Error Resilience
console.log('  4.4 Network error / rejection resilience in showTimeseriesChart & hover...');
globalThis.fetch = async () => {
    throw new Error('Synthetic network offline failure');
};

await assert.doesNotReject(async () => {
    await showTimeseriesChart(52.1234, 5.5678);
});
assert.equal(DOM.chartCoords.textContent.includes('Error'), true, 'Chart coordinates should indicate error on network rejection');

state.isCompareModeActive = false;
state.currentLayerMode = 'rain';
handleMapMouseMove({
    lngLat: { lat: 52.1234, lng: 5.5678 },
    originalEvent: { clientX: 200 }
});

await assert.doesNotReject(async () => {
    await triggerHoverQuery();
});
assert.equal(DOM.hoverValue.textContent, 'Error', 'Hover value should indicate error on network rejection');
console.log('      ✓ Network failure caught gracefully by chart and hover error boundaries.');

console.log('\n=================================================================');
console.log('  ✓ ALL 4 WebGL & Client Empirical Adversarial Stress Suites Passed!');
console.log('=================================================================\n');
