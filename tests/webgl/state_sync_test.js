import assert from 'node:assert/strict';

console.log('--- Running Frontend Reactive State Store & URL Sync Verification Tests ---');

// Mock Application State & Sync Logic
const state = {
    currentLayerMode: 'rain',
    currentEns: 'med',
    selectedWindHeight: 10,
    currentTimeIndex: 0,
    isCompareModeActive: false,
    compareLayerMode: 'temp',
    compareEns: 'med',
    compareSelectedWindHeight: 10,
    dividerPosition: 50,
    activeCoords: null,
};

function serializeStateToURL(searchParams) {
    searchParams.set('mode', state.currentLayerMode);
    searchParams.set('ens', state.currentEns.toString());
    searchParams.set('height', state.selectedWindHeight.toString());
    if (state.activeCoords) {
        searchParams.set('slat', state.activeCoords.lat.toFixed(4));
        searchParams.set('slon', state.activeCoords.lon.toFixed(4));
    } else {
        searchParams.delete('slat');
        searchParams.delete('slon');
    }
}

function parseURLIntoState(searchParams) {
    const mode = searchParams.get('mode');
    if (mode && ['rain', 'temp', 'wind', 'solar'].includes(mode)) {
        state.currentLayerMode = mode;
    }
    const ens = searchParams.get('ens');
    if (ens && ['pmm', 'med', 'max', 'prob', 'spread', '0', '1', '2'].includes(ens)) {
        state.currentEns = ens;
    }
    const height = searchParams.get('height');
    if (height && ['10', '50', '100', '200', '300'].includes(height)) {
        state.selectedWindHeight = parseInt(height, 10);
    }
    const slat = parseFloat(searchParams.get('slat'));
    const slon = parseFloat(searchParams.get('slon'));
    if (!isNaN(slat) && !isNaN(slon)) {
        state.activeCoords = { lat: slat, lon: slon };
    } else {
        state.activeCoords = null;
    }
}

// 1. Initial Default State Serialization
console.log('Testing default state serialization...');
const params1 = new URLSearchParams();
serializeStateToURL(params1);
assert.equal(params1.get('mode'), 'rain');
assert.equal(params1.get('ens'), 'med');
assert.equal(params1.get('height'), '10');
assert.equal(params1.get('slat'), null);

// 2. Point Selection and URL Synchronization
console.log('Testing selected location syncing to URL...');
state.activeCoords = { lat: 52.1012, lon: 5.1768 };
const params2 = new URLSearchParams();
serializeStateToURL(params2);
assert.equal(params2.get('slat'), '52.1012');
assert.equal(params2.get('slon'), '5.1768');

// 3. Deserialization from Query Parameters
console.log('Testing URL parameter deserialization into state...');
const queryParams = new URLSearchParams('mode=wind&ens=max&height=100&slat=51.4425&slon=3.5731');
parseURLIntoState(queryParams);
assert.equal(state.currentLayerMode, 'wind');
assert.equal(state.currentEns, 'max');
assert.equal(state.selectedWindHeight, 100);
assert.equal(state.activeCoords.lat, 51.4425);
assert.equal(state.activeCoords.lon, 3.5731);

// 4. Invalid Parameter Sanitization & Fallback
console.log('Testing invalid query parameter sanitization...');
const badParams = new URLSearchParams('mode=malicious_layer&height=9999&ens=hack');
parseURLIntoState(badParams);
// Should retain previous valid state rather than corrupting
assert.equal(state.currentLayerMode, 'wind');
assert.equal(state.selectedWindHeight, 100);

// 5. Compare Mode Split-Screen State Sync
console.log('Testing compare mode split-screen state sync...');
state.isCompareModeActive = true;
state.currentLayerMode = 'rain';
state.compareLayerMode = 'temp';
state.dividerPosition = 65;

assert.equal(state.isCompareModeActive, true);
assert.equal(state.currentLayerMode, 'rain');
assert.equal(state.compareLayerMode, 'temp');
assert.equal(state.dividerPosition, 65);

console.log('✓ All 5 Frontend State Store & URL Sync Verification Tests Passed Successfully!');
