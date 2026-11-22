#!/usr/bin/env bun

// Prepares the mobile store screenshot upload. The release
// script uses the platform-specific screenshot directories
// and metadata to locate the images for the selected store.

import { commandExists, fail, mobileDirectory, run } from './release-environment'

const platform = Bun.argv[2]
if (platform !== 'ios' && platform !== 'android') {
  fail('Usage: upload-screenshots.ts ios|android')
}
if (!(await commandExists('bundle'))) {
  fail("bundle not found. Run 'bundle install' in mobile/.")
}

if (platform === 'android') {
  const jsonKey = process.env.GOOGLE_PLAY_JSON_KEY ?? process.env.SUPPLY_JSON_KEY
  if (!jsonKey && !process.env.GOOGLE_PLAY_JSON_KEY_DATA && !process.env.SUPPLY_JSON_KEY_DATA) {
    fail('set GOOGLE_PLAY_JSON_KEY to your Play service account JSON path.')
  }
  await run(['bundle', 'exec', 'fastlane', 'android', 'upload_screenshots'], {
    cwd: mobileDirectory,
    env: { GOOGLE_PLAY_JSON_KEY: jsonKey },
  })
} else {
  await run(['bundle', 'exec', 'fastlane', 'ios', 'upload_screenshots'], { cwd: mobileDirectory })
}

console.log('Done.')
