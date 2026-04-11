const { spawn } = require('child_process');

const p = spawn('npx', ['@tauri-apps/cli', 'plugin', 'new', 'pokeclaw', '--android', '--no-example', '--author', 'Jaron', '--no-api'], {
  stdio: ['pipe', 'pipe', 'pipe'],
  shell: true
});

p.stdout.on('data', (data) => {
  console.log(`STDOUT: ${data}`);
  p.stdin.write('\n'); // always hit enter
});

p.stderr.on('data', (data) => {
  console.error(`STDERR: ${data}`);
  p.stdin.write('\n');
});

p.on('close', (code) => {
  console.log(`child process exited with code ${code}`);
});
