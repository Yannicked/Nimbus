import { execSync } from 'node:child_process';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

console.log('=================================================================');
console.log('   Nimbus WebGL & Client-Side Automated Test Suite Runner');
console.log('=================================================================\n');

const testFiles = [
    'shader_compilation_test.js',
    'unpack_precision_test.js',
    'lru_cache_test.js',
    'state_sync_test.js',
    'context_loss_test.js',
    'adversarial_stress_test.js'
];

let totalPassed = 0;
let totalFailed = 0;

for (const file of testFiles) {
    const fullPath = resolve(__dirname, file);
    console.log(`\n>> Executing: ${file}`);
    try {
        const output = execSync(`node ${fullPath}`, { stdio: 'pipe', encoding: 'utf8' });
        console.log(output.trim());
        totalPassed++;
    } catch (err) {
        console.error(`❌ Test Suite ${file} FAILED:`, err.stdout || err.message);
        totalFailed++;
    }
}

console.log('\n=================================================================');
console.log(`   Client/WebGL Test Summary: ${totalPassed} Passed, ${totalFailed} Failed`);
console.log('=================================================================\n');

if (totalFailed > 0) {
    process.exit(1);
} else {
    process.exit(0);
}
