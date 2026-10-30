import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// Launch the same npm script without cmd.exe or persistent environment changes.
const child = spawn(process.execPath, [process.env.npm_execpath, 'run', 'tauri', '--', 'dev'], {
  cwd: fileURLToPath(new URL('../', import.meta.url)),
  stdio: 'inherit',
  env: { ...process.env, SDDIRECTA_DEV_EDITION: 'lite' },
});
let stopping = false;
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    if (stopping || child.exitCode !== null || !child.pid) return;
    stopping = true;
    if (process.platform === 'win32') {
      // Stop only our npm process tree, including Tauri and Vite.
      const stop = spawn('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], {
        stdio: 'ignore', windowsHide: true,
      });
      stop.on('error', () => { child.kill(); });
    } else {
      child.kill(signal);
    }
  });
}
child.on('error', error => {
  console.error(`ライト版の開発起動に失敗しました: ${error.message}`);
  process.exitCode = 1;
});
child.on('exit', (code, signal) => { process.exitCode = code ?? (signal ? 1 : 0); });
