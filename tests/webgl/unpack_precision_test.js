import assert from 'node:assert/strict';

console.log('--- Running WebGL 16-bit Unpack & Precision Verification Tests ---');

// Formula: raw = floor(R * 255.0 + 0.5) * 256.0 + floor(G * 255.0 + 0.5)
function unpack16Bit(rNorm, gNorm) {
    const r = Math.floor(rNorm * 255.0 + 0.5);
    const g = Math.floor(gNorm * 255.0 + 0.5);
    return r * 256.0 + g;
}

// 1. Lossless Roundtrip across 0..65535 u16
console.log('Testing 16-bit integer lossless roundtrip...');
const testValues = [0, 1, 10, 100, 255, 256, 1000, 10500, 32767, 65534, 65535];
for (const val of testValues) {
    const r = Math.floor(val / 256);
    const g = val % 256;
    const rNorm = r / 255.0;
    const gNorm = g / 255.0;
    const unpacked = unpack16Bit(rNorm, gNorm);
    assert.equal(unpacked, val, `Failed roundtrip for value ${val}`);
}

// 2. Precipitation Rate Physical Units: val_mmh = raw * 0.01
console.log('Testing Precipitation Rate Physical Units...');
assert.equal(100 * 0.01, 1.0); // 1.0 mm/h
assert.equal(250 * 0.01, 2.5); // 2.5 mm/h
assert.equal(5000 * 0.01, 50.0); // 50.0 mm/h

// 3. 2m Temperature Physical Units: val_c = raw / 10.0 - 273.15
console.log('Testing 2m Temperature Physical Units...');
function rawToTempC(raw) {
    return raw / 10.0 - 273.15;
}
assert.equal(rawToTempC(2731).toFixed(2), "-0.05"); // ~ 0 C
assert.equal(rawToTempC(2931).toFixed(2), "19.95"); // ~ 20 C
assert.equal(rawToTempC(3031).toFixed(2), "29.95"); // ~ 30 C

// 4. Wind Velocity Physical Units: u = u_raw / 100.0 - 100.0, v = v_raw / 100.0 - 100.0
console.log('Testing Wind Velocity Physical Units...');
function rawToWindUV(uRaw, vRaw) {
    const u = uRaw / 100.0 - 100.0;
    const v = vRaw / 100.0 - 100.0;
    const speed = Math.sqrt(u * u + v * v);
    let dirRad = Math.atan2(u, v) + Math.PI;
    if (dirRad < 0.0) dirRad += 2.0 * Math.PI;
    const direction = (dirRad * 180.0) / Math.PI;
    return { u, v, speed, direction };
}

const calm = rawToWindUV(10000, 10000);
assert.equal(calm.u, 0.0);
assert.equal(calm.v, 0.0);
assert.equal(calm.speed, 0.0);

const easterly = rawToWindUV(10500, 10000); // u = +5 m/s, v = 0 m/s
assert.equal(easterly.u, 5.0);
assert.equal(easterly.v, 0.0);
assert.equal(easterly.speed, 5.0);

const northerly = rawToWindUV(10000, 11000); // u = 0, v = +10 m/s
assert.equal(northerly.u, 0.0);
assert.equal(northerly.v, 10.0);
assert.equal(northerly.speed, 10.0);

// 5. Solar Radiation Physical Units: direct Watts/m2
console.log('Testing Solar Radiation Physical Units...');
function rawToSolar(raw) {
    return raw;
}
assert.equal(rawToSolar(0), 0);
assert.equal(rawToSolar(450), 450);
assert.equal(rawToSolar(1000), 1000);

console.log('✓ All 5 Unpack & Precision Verification Tests Passed Successfully!');
