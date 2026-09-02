import assert from 'node:assert/strict';

console.log('--- Running WebGL Context Loss & Restoration Lifecycle Tests ---');

// Mock WebGL Layer Lifecycle
class MockWebGLLayer {
    constructor(id = 'radar-webgl-layer') {
        this.id = id;
        this.program = null;
        this.posBuffer = null;
        this.texBuffer = null;
        this.textureCache = new Map();
        this.isContextLost = false;
        this.restorationCount = 0;
    }

    onAdd(gl) {
        this.program = { id: 'prog_1' };
        this.posBuffer = { id: 'buf_pos' };
        this.texBuffer = { id: 'buf_tex' };
        this.isContextLost = false;
    }

    handleContextLost(event) {
        if (event && event.preventDefault) {
            event.preventDefault(); // Required by WebGL spec to allow restoration
        }
        this.isContextLost = true;
        // Invalidate GPU resource handles
        this.program = null;
        this.posBuffer = null;
        this.texBuffer = null;
        this.textureCache.clear();
    }

    handleContextRestored(gl) {
        this.restorationCount++;
        // Recompile shaders and re-create buffers
        this.onAdd(gl);
    }

    render(gl) {
        if (this.isContextLost || !this.program || !this.posBuffer) {
            return false; // Skip render cleanly without WebGL errors or exceptions
        }
        return true;
    }
}

// 1. Initial layer creation
console.log('Testing normal layer initialization...');
const layer = new MockWebGLLayer();
layer.onAdd({});
assert.equal(layer.isContextLost, false);
assert(layer.program !== null);
assert(layer.render({}));

// 2. Context Loss event trigger
console.log('Testing WebGL Context Loss event handling...');
let preventDefaultCalled = false;
const mockLossEvent = {
    preventDefault: () => {
        preventDefaultCalled = true;
    }
};

layer.handleContextLost(mockLossEvent);
assert.equal(preventDefaultCalled, true, 'preventDefault must be called on webglcontextlost event');
assert.equal(layer.isContextLost, true);
assert.equal(layer.program, null);
assert.equal(layer.posBuffer, null);
assert.equal(layer.textureCache.size, 0);

// 3. Render pass during context loss
console.log('Testing render pass safety while context is lost...');
const renderSuccess = layer.render({});
assert.equal(renderSuccess, false, 'Render pass must skip cleanly when context is lost');

// 4. Context Restored event trigger & resource rebuilding
console.log('Testing WebGL Context Restored lifecycle recovery...');
layer.handleContextRestored({});
assert.equal(layer.isContextLost, false);
assert.equal(layer.restorationCount, 1);
assert(layer.program !== null);
assert(layer.posBuffer !== null);
assert.equal(layer.render({}), true);

// 5. Repeated consecutive context loss cycles (stress test)
console.log('Testing repeated context loss and restoration cycles...');
for (let cycle = 1; cycle <= 10; cycle++) {
    layer.handleContextLost(mockLossEvent);
    assert.equal(layer.render({}), false);
    layer.handleContextRestored({});
    assert.equal(layer.render({}), true);
}
assert.equal(layer.restorationCount, 11);

console.log('✓ All 5 WebGL Context Loss & Restoration Lifecycle Tests Passed Successfully!');
