import assert from 'node:assert/strict';

console.log('--- Running WebGL Bounded LRU Cache Verification Tests ---');

// Mock WebGL context to track deleted textures
class MockGL {
    constructor() {
        this.deletedTextures = [];
        this.createdTextures = 0;
    }
    createTexture() {
        this.createdTextures++;
        return { id: `tex_${this.createdTextures}` };
    }
    deleteTexture(tex) {
        this.deletedTextures.push(tex.id);
    }
}

// Bounded LRU Cache Implementation for WebGL Textures
class BoundedLRUTextureCache {
    constructor(maxEntries = 48) {
        this.maxEntries = maxEntries;
        this.cache = new Map();
    }

    get(key) {
        if (!this.cache.has(key)) return null;
        const entry = this.cache.get(key);
        // Refresh LRU order (re-insert)
        this.cache.delete(key);
        this.cache.set(key, entry);
        return entry;
    }

    set(key, entry, gl) {
        if (this.cache.has(key)) {
            this.cache.delete(key);
        } else if (this.cache.size >= this.maxEntries) {
            // Evict oldest entry (first item in Map iteration)
            const oldestKey = this.cache.keys().next().value;
            const oldestEntry = this.cache.get(oldestKey);
            if (oldestEntry && oldestEntry.texture && gl) {
                gl.deleteTexture(oldestEntry.texture);
            }
            this.cache.delete(oldestKey);
        }
        this.cache.set(key, entry);
    }

    clear(gl) {
        for (const entry of this.cache.values()) {
            if (entry && entry.texture && gl) {
                gl.deleteTexture(entry.texture);
            }
        }
        this.cache.clear();
    }

    size() {
        return this.cache.size;
    }
}

// 1. Strict capacity bounding
console.log('Testing capacity bounding (max 48 entries)...');
const gl = new MockGL();
const lru = new BoundedLRUTextureCache(48);

for (let i = 0; i < 100; i++) {
    const tex = gl.createTexture();
    lru.set(`key_${i}`, { texture: tex }, gl);
    assert(lru.size() <= 48, `Cache exceeded capacity at iteration ${i}: size is ${lru.size()}`);
}
assert.equal(lru.size(), 48);
assert.equal(gl.deletedTextures.length, 52); // 100 - 48 = 52 evicted textures deleted

// 2. LRU Eviction Order (Most recently accessed kept)
console.log('Testing LRU access order retention...');
const gl2 = new MockGL();
const smallLRU = new BoundedLRUTextureCache(3);

const texA = gl2.createTexture();
const texB = gl2.createTexture();
const texC = gl2.createTexture();
const texD = gl2.createTexture();

smallLRU.set('A', { texture: texA }, gl2);
smallLRU.set('B', { texture: texB }, gl2);
smallLRU.set('C', { texture: texC }, gl2);

// Access 'A' -> moves 'A' to most recently used
smallLRU.get('A');

// Insert 'D' -> should evict 'B' (oldest), keeping 'C', 'A', 'D'
smallLRU.set('D', { texture: texD }, gl2);

assert.equal(smallLRU.get('B'), null);
assert(smallLRU.get('A') !== null);
assert(smallLRU.get('C') !== null);
assert(smallLRU.get('D') !== null);
assert.equal(gl2.deletedTextures[0], texB.id);

// 3. Mobile Low-Memory bound (24 frames)
console.log('Testing mobile bounded cache (max 24 entries)...');
const mobileGL = new MockGL();
const mobileLRU = new BoundedLRUTextureCache(24);
for (let i = 0; i < 50; i++) {
    const tex = mobileGL.createTexture();
    mobileLRU.set(`mob_${i}`, { texture: tex }, mobileGL);
}
assert.equal(mobileLRU.size(), 24);
assert.equal(mobileGL.deletedTextures.length, 26);

// 4. Cache Clear & Full GPU Resource Deletion
console.log('Testing full cache clear and GPU texture release...');
const clearGL = new MockGL();
const clearLRU = new BoundedLRUTextureCache(10);
for (let i = 0; i < 10; i++) {
    clearLRU.set(`c_${i}`, { texture: clearGL.createTexture() }, clearGL);
}
assert.equal(clearLRU.size(), 10);
clearLRU.clear(clearGL);
assert.equal(clearLRU.size(), 0);
assert.equal(clearGL.deletedTextures.length, 10);

// 5. Preload Ahead Queue Simulation
console.log('Testing forward preload ahead queue (3 frames)...');
const preloadGL = new MockGL();
const preloadLRU = new BoundedLRUTextureCache(48);
const totalFrames = 20;
let currentFrame = 5;
const preloadAhead = 3;

for (let offset = 1; offset <= preloadAhead; offset++) {
    const targetFrame = (currentFrame + offset) % totalFrames;
    const key = `rain-med-${targetFrame}`;
    if (!preloadLRU.get(key)) {
        preloadLRU.set(key, { texture: preloadGL.createTexture() }, preloadGL);
    }
}
assert.equal(preloadLRU.size(), 3);
assert(preloadLRU.get('rain-med-6') !== null);
assert(preloadLRU.get('rain-med-7') !== null);
assert(preloadLRU.get('rain-med-8') !== null);

console.log('✓ All 5 Bounded LRU Cache Verification Tests Passed Successfully!');
