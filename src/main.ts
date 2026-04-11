import { invoke } from '@tauri-apps/api/core';

document.querySelector<HTMLDivElement>('#app')!.innerHTML = `
  <div>
    <h1>PokeClaw Tauri App</h1>
    <p>Testing bridge...</p>
    <div id="ping-result"></div>
  </div>
`

async function testPing() {
  try {
    const result = await invoke('plugin:pokeclaw|ping');
    document.querySelector('#ping-result')!.textContent = `Ping result: ${JSON.stringify(result)}`;
  } catch (error) {
    document.querySelector('#ping-result')!.textContent = `Ping error: ${error}`;
  }
}

testPing();