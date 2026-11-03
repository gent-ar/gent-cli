@echo off
for /f "delims=" %%i in ('node -p "require('path').resolve(process.execPath,'../../lib/node_modules/npm/bin/npx-cli.js')"') do node "%%i" %*
