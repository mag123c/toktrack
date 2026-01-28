#!/usr/bin/env node
const { execFileSync } = require('child_process');
const path = require('path');
const os = require('os');

const platform = os.platform();
const arch = os.arch();

// Platform mapping to binary names
const platformMap = {
  'darwin-arm64': 'toktrack-darwin-arm64',
  'darwin-x64': 'toktrack-darwin-x64',
  'linux-x64': 'toktrack-linux-x64',
  'linux-arm64': 'toktrack-linux-arm64',
  'win32-x64': 'toktrack-win32-x64.exe',
};

const key = `${platform}-${arch}`;
const binaryName = platformMap[key];

if (!binaryName) {
  console.error(`Unsupported platform: ${platform}-${arch}`);
  console.error('Supported platforms: darwin-arm64, darwin-x64, linux-x64, linux-arm64, win32-x64');
  process.exit(1);
}

const binary = path.join(__dirname, binaryName);

try {
  execFileSync(binary, process.argv.slice(2), { stdio: 'inherit' });
} catch (e) {
  if (e.status) process.exit(e.status);
  throw e;
}
